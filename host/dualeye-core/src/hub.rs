//! Share the board between processes. Only one process can hold the serial
//! port, and while the app or `dualeye` streams, the bridge holds it. Its
//! [`Hub`] lets others (`dualeye mcp`, `dualeye call`) use the board through
//! the bridge instead of opening the port a second time. [`Board`] is the
//! other side: it goes through a running hub, and opens the port for the call
//! itself when there is none.
//!
//! The hub listens on 127.0.0.1, on a port the OS picks, and leaves the port
//! and a random token in `hub.json` in DualEye's data folder, readable by this
//! user only. A client's first request is `hello` with that token. Then, one
//! JSON-RPC 2.0 message per line, a request at a time:
//!
//! | Method | Result |
//! |--------|--------|
//! | `hello` `{"token","client"}` | `{"bridge", "board": Hello or null, "port"}` |
//! | `tools/list`, `tools/call`, `media/...` | Passed to the board as they are |
//! | `host/snapshot` | `{"snapshot": Snapshot or null, "age_ms"}`: the bridge's latest sample |
//! | `host/say` `{"text","language"?}` | How it was spoken, once played |
//! | `host/claude_hook` (a Claude Code hook's JSON) | `{"taken"}`: whether a bridge takes alerts |
//! | `host/timers` `{"name","arguments","language"?}` | `{"text","is_error"}`: a [`Timers::tools`] tool, run on the bridge's timers |

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::link::{CallError, Hello, Link, Tool, ToolResult};
use crate::serial;
use crate::snapshot::Snapshot;
use crate::timers::{self, Timers};

/// Reply for a request the hub can't pass on because no board is attached.
pub const NO_BOARD: i64 = -32000;
const UNAUTHORIZED: i64 = -32001;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_REQUEST: i64 = -32600;
const INVALID_PARAMS: i64 = -32602;
/// `host/say` answers once the board has spoken.
const SAY_TIMEOUT: Duration = Duration::from_secs(90);

/// How long the board gets for one tool call.
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(3);
/// A client waits this long for the hub, which waits [`TOOL_TIMEOUT`] for the board.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(100);
/// Tools whose effect the bridge mirrors in its settings.
const SETTINGS_TOOLS: [&str; 2] = ["set_face", "set_rotation"];

/// Where a running hub says how to reach it.
pub fn hub_file() -> Option<PathBuf> {
    crate::claude::data_dir().map(|d| d.join("hub.json"))
}

#[derive(Serialize, Deserialize)]
struct HubFile {
    port: u16,
    token: String,
    pid: u32,
}

/// What a frontend shows about the processes using the board through the hub.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HubStatus {
    /// Clients connected now (each MCP server keeps one open).
    pub clients: usize,
    /// Tool calls passed to the board since the hub started.
    pub calls: u64,
    pub last_tool: Option<String>,
    pub last_call_age_ms: Option<u64>,
}

struct Attached {
    link: Arc<Link>,
    hello: Hello,
    port: String,
}

struct Shared {
    token: String,
    stop: AtomicBool,
    board: Mutex<Result<Attached, String>>,
    /// One board request at a time: the board drops input it has no room for.
    one_at_a_time: Mutex<()>,
    snapshot: Mutex<Option<(Snapshot, Instant)>>,
    settings_changed: AtomicBool,
    /// Speaks through the board, when the bridge has text-to-speech.
    say: Mutex<Option<SayFn>>,
    /// Takes Claude Code's hook events, while a bridge runs.
    claude_hook: Mutex<Option<HookFn>>,
    /// The bridge's timers, while it runs.
    timers: Mutex<Option<Arc<Timers>>>,
    clients: AtomicUsize,
    calls: AtomicU64,
    last_call: Mutex<Option<(String, Instant)>>,
}

/// The listening end, owned by the frontend and handed to its bridge through
/// [`crate::BridgeConfig::hub`]. It outlives bridge restarts, so clients keep
/// being told the board is busy while esptool holds the port, rather than
/// opening it themselves. Dropping it stops listening and removes `hub.json`.
pub struct Hub {
    shared: Arc<Shared>,
    file: Option<PathBuf>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hub").field("file", &self.file).finish_non_exhaustive()
    }
}

impl Hub {
    pub fn start() -> io::Result<Arc<Hub>> {
        Self::start_at(hub_file())
    }

    fn start_at(file: Option<PathBuf>) -> io::Result<Arc<Hub>> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let mut token = [0u8; 16];
        getrandom::fill(&mut token).map_err(|e| io::Error::other(e.to_string()))?;
        let token: String = token.iter().map(|b| format!("{b:02x}")).collect();
        if let Some(path) = &file {
            let info = HubFile { port, token: token.clone(), pid: std::process::id() };
            write_private(path, &serde_json::to_vec(&info).expect("hub file serializes"))?;
        }
        let shared = Arc::new(Shared {
            token,
            stop: AtomicBool::new(false),
            board: Mutex::new(Err("the bridge has not found the board yet".into())),
            one_at_a_time: Mutex::default(),
            snapshot: Mutex::default(),
            settings_changed: AtomicBool::new(false),
            say: Mutex::default(),
            claude_hook: Mutex::default(),
            timers: Mutex::default(),
            clients: AtomicUsize::new(0),
            calls: AtomicU64::new(0),
            last_call: Mutex::default(),
        });
        let thread = {
            let shared = shared.clone();
            thread::Builder::new().name("dualeye-hub".into()).spawn(move || accept_loop(&listener, &shared))?
        };
        Ok(Arc::new(Hub { shared, file, thread: Some(thread) }))
    }

    pub fn status(&self) -> HubStatus {
        let last = self.shared.last_call.lock().unwrap().clone();
        HubStatus {
            clients: self.shared.clients.load(Ordering::Relaxed),
            calls: self.shared.calls.load(Ordering::Relaxed),
            last_tool: last.as_ref().map(|(tool, _)| tool.clone()),
            last_call_age_ms: last.map(|(_, at)| at.elapsed().as_millis() as u64),
        }
    }

    /// Clients get this as the reason their board calls fail until a board is attached again.
    pub fn set_unavailable(&self, reason: &str) {
        *self.shared.board.lock().unwrap() = Err(reason.to_string());
    }

    pub(crate) fn attach(&self, link: Arc<Link>, hello: Hello, port: &str) {
        *self.shared.board.lock().unwrap() = Ok(Attached { link, hello, port: port.to_string() });
    }

    /// What `host/say` runs; `None` while the bridge can't speak.
    pub(crate) fn set_say(&self, say: Option<SayFn>) {
        *self.shared.say.lock().unwrap() = say;
    }

    /// What `host/claude_hook` hands its event to; `None` without a bridge.
    pub(crate) fn set_claude_hook(&self, hook: Option<HookFn>) {
        *self.shared.claude_hook.lock().unwrap() = hook;
    }

    /// The timers `host/timers` works on; `None` without a bridge.
    pub(crate) fn set_timers(&self, timers: Option<Arc<Timers>>) {
        *self.shared.timers.lock().unwrap() = timers;
    }

    pub(crate) fn set_snapshot(&self, snapshot: &Snapshot) {
        *self.shared.snapshot.lock().unwrap() = Some((snapshot.clone(), Instant::now()));
    }

    /// A client changed a face or a rotation since the last call.
    pub(crate) fn take_settings_changed(&self) -> bool {
        self.shared.settings_changed.swap(false, Ordering::Relaxed)
    }

    #[cfg(test)]
    fn addr(&self) -> SocketAddr {
        let info: HubFile = serde_json::from_slice(&fs::read(self.file.as_ref().unwrap()).unwrap()).unwrap();
        SocketAddr::from((Ipv4Addr::LOCALHOST, info.port))
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // Leave a newer hub's file alone.
        if let Some(path) = &self.file
            && read_hub_file(path).is_some_and(|f| f.token == self.shared.token)
        {
            let _ = fs::remove_file(path);
        }
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

fn read_hub_file(path: &Path) -> Option<HubFile> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn accept_loop(listener: &TcpListener, shared: &Arc<Shared>) {
    while !shared.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let shared = shared.clone();
                let _ = thread::Builder::new().name("dualeye-hub-client".into()).spawn(move || {
                    shared.clients.fetch_add(1, Ordering::Relaxed);
                    let _ = serve_client(stream, &shared);
                    shared.clients.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(POLL),
            Err(_) => thread::sleep(POLL),
        }
    }
}

fn serve_client(stream: TcpStream, shared: &Shared) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(POLL * 2))?;
    let mut out = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    let mut authorized = false;
    while !shared.stop.load(Ordering::Relaxed) {
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => return Ok(()),
            Ok(_) if line.ends_with(b"\n") => {}
            Ok(_) => return Ok(()),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => continue,
            Err(e) => return Err(e),
        }
        let request: Option<Value> = serde_json::from_slice(&line).ok();
        line.clear();
        let Some(request) = request else { continue };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let reply = if method == "hello" {
            authorized = params.get("token").and_then(Value::as_str) == Some(shared.token.as_str());
            if authorized { Ok(hello(shared)) } else { Err((UNAUTHORIZED, "wrong hub token".to_string())) }
        } else if !authorized {
            Err((UNAUTHORIZED, "send hello first".to_string()))
        } else {
            handle(shared, method, params)
        };
        let message = match reply {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        };
        let mut bytes = message.to_string().into_bytes();
        bytes.push(b'\n');
        out.write_all(&bytes)?;
        if !authorized {
            return Ok(());
        }
    }
    Ok(())
}

fn hello(shared: &Shared) -> Value {
    let board = shared.board.lock().unwrap();
    let (hello, port) = match &*board {
        Ok(a) => (serde_json::to_value(&a.hello).unwrap_or(Value::Null), Some(a.port.clone())),
        Err(_) => (Value::Null, None),
    };
    json!({"bridge": env!("CARGO_PKG_VERSION"), "board": hello, "port": port})
}

/// Speak `text` (in `language`, or the one it looks like) and return how it went.
pub(crate) type SayFn = Arc<dyn Fn(&str, Option<&str>) -> Result<Value, String> + Send + Sync>;

/// Take a Claude Code hook's JSON; it must not wait on the board.
pub(crate) type HookFn = Arc<dyn Fn(Value) + Send + Sync>;

fn handle(shared: &Shared, method: &str, params: Value) -> Result<Value, (i64, String)> {
    match method {
        "tools/list" | "tools/call" => {}
        // Image uploads (firmware 1.1); writes take a moment when the flash erases.
        m if m.starts_with("media/") => {}
        "host/say" => {
            let text = params.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty());
            let Some(text) = text else { return Err((INVALID_PARAMS, "expected {\"text\": string, \"language\"?: \"it\" | \"en\"}".into())) };
            let say = shared.say.lock().unwrap().clone();
            let say = say.ok_or((NO_BOARD, "the bridge isn't speaking: turn on spoken replies in the app, or run dualeye --tts".to_string()))?;
            return say(text, params.get("language").and_then(Value::as_str)).map_err(|e| (NO_BOARD, e));
        }
        "host/claude_hook" => {
            let hook = shared.claude_hook.lock().unwrap().clone();
            let taken = hook.is_some();
            if let Some(hook) = hook {
                hook(params);
            }
            return Ok(json!({"taken": taken}));
        }
        "host/timers" => {
            let timers = shared.timers.lock().unwrap().clone().ok_or((NO_BOARD, "the bridge isn't running".to_string()))?;
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let language = params.get("language").and_then(Value::as_str).unwrap_or("en");
            return match timers.call_tool(name, &arguments, language) {
                Some(Ok(text)) => Ok(json!({"text": text, "is_error": false})),
                Some(Err(text)) => Ok(json!({"text": text, "is_error": true})),
                None => Err((INVALID_PARAMS, format!("unknown timer tool `{name}`"))),
            };
        }
        "host/snapshot" => {
            let snapshot = shared.snapshot.lock().unwrap();
            let (snap, age) = match &*snapshot {
                Some((s, at)) => (serde_json::to_value(s).unwrap_or(Value::Null), Some(at.elapsed().as_millis() as u64)),
                None => (Value::Null, None),
            };
            return Ok(json!({"snapshot": snap, "age_ms": age}));
        }
        "" => return Err((INVALID_REQUEST, "no method".into())),
        other => return Err((METHOD_NOT_FOUND, format!("unknown method `{other}`"))),
    }
    let link = match &*shared.board.lock().unwrap() {
        Ok(a) => a.link.clone(),
        Err(reason) => return Err((NO_BOARD, reason.clone())),
    };
    let tool = (method == "tools/call").then(|| params.get("name").and_then(Value::as_str).unwrap_or("").to_string());
    let reply = {
        let _one = shared.one_at_a_time.lock().unwrap();
        link.call(method, params, TOOL_TIMEOUT)
    };
    if let Some(tool) = tool {
        shared.calls.fetch_add(1, Ordering::Relaxed);
        let ok = reply.as_ref().is_ok_and(|r| r.get("isError") != Some(&Value::Bool(true)));
        if ok && SETTINGS_TOOLS.contains(&tool.as_str()) {
            shared.settings_changed.store(true, Ordering::Relaxed);
        }
        *shared.last_call.lock().unwrap() = Some((tool, Instant::now()));
    }
    reply.map_err(|e| match e {
        CallError::Rpc { code, message } => (code, message),
        other => (NO_BOARD, other.to_string()),
    })
}

/// What `hello` told a client about the hub.
#[derive(Debug, Clone, Deserialize)]
pub struct HubHello {
    pub bridge: String,
    pub board: Option<Hello>,
    pub port: Option<String>,
}

/// One connection to a running hub.
struct HubClient {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: u64,
}

impl HubClient {
    /// `Ok(None)` when no hub is running.
    fn connect(file: &Path, client: &str) -> io::Result<Option<(HubClient, HubHello)>> {
        let Some(info) = read_hub_file(file) else { return Ok(None) };
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, info.port));
        // A hub that went away without cleaning up: its port refuses.
        let Ok(stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else { return Ok(None) };
        Self::handshake(stream, &info.token, client).map(Some)
    }

    fn handshake(stream: TcpStream, token: &str, client: &str) -> io::Result<(HubClient, HubHello)> {
        // The hub answers at once; a stale hub.json may point at someone else's port.
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        stream.set_nodelay(true)?;
        let writer = stream.try_clone()?;
        let mut hub = HubClient { reader: BufReader::new(stream), writer, next_id: 1 };
        let hello = hub.request("hello", json!({"token": token, "client": client})).map_err(|e| io::Error::other(e.to_string()))?;
        let hello = serde_json::from_value(hello).map_err(io::Error::other)?;
        hub.writer.set_read_timeout(Some(CLIENT_TIMEOUT))?;
        Ok((hub, hello))
    }

    fn request_waiting(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, CallError> {
        self.writer.set_read_timeout(Some(timeout))?;
        let reply = self.request(method, params);
        self.writer.set_read_timeout(Some(CLIENT_TIMEOUT))?;
        reply
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, CallError> {
        let id = self.next_id;
        self.next_id += 1;
        let mut bytes = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string().into_bytes();
        bytes.push(b'\n');
        self.writer.write_all(&bytes)?;
        loop {
            let mut line = Vec::new();
            match self.reader.read_until(b'\n', &mut line) {
                Ok(0) => return Err(CallError::Closed),
                Ok(_) => {}
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => return Err(CallError::Timeout),
                Err(e) => return Err(e.into()),
            }
            let msg: Value = serde_json::from_slice(&line).map_err(|e| CallError::Invalid(e.to_string()))?;
            // A reply to an earlier request that timed out.
            if msg.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(result) = msg.get("result") {
                return Ok(result.clone());
            }
            let err = msg.get("error").cloned().unwrap_or_default();
            let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
            let message = err.get("message").and_then(Value::as_str).unwrap_or("error").to_string();
            return Err(if code == NO_BOARD { CallError::Unavailable(message) } else { CallError::Rpc { code, message } });
        }
    }
}

/// How [`Board`] reached the board for its last call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// Through the app's or `dualeye`'s bridge.
    Hub,
    /// The port, opened for that call only.
    Direct,
}

/// The board, for processes that don't stream to it: through the running
/// bridge when there is one, otherwise over the port, opened for each call
/// and closed right after so it stays free for the app.
pub struct Board {
    port: Option<String>,
    client: String,
    file: Option<PathBuf>,
    hub: Mutex<Option<HubClient>>,
    route: Mutex<Option<Route>>,
}

impl Board {
    /// `port` is only used when no bridge runs; `None` auto-detects the board.
    /// `client` names this process to the hub (`dualeye-mcp/0.1.0`).
    pub fn new(port: Option<String>, client: &str) -> Self {
        Self::with_hub_file(port, client, hub_file())
    }

    pub(crate) fn with_hub_file(port: Option<String>, client: &str, file: Option<PathBuf>) -> Self {
        Self { port, client: client.to_string(), file, hub: Mutex::default(), route: Mutex::default() }
    }

    pub fn last_route(&self) -> Option<Route> {
        *self.route.lock().unwrap()
    }

    pub fn list_tools(&self) -> Result<Vec<Tool>, CallError> {
        #[derive(Deserialize)]
        struct List {
            tools: Vec<Tool>,
        }
        let list = self.run(|hub| hub.request("tools/list", json!({})), |link| Ok(json!({"tools": link.list_tools(TOOL_TIMEOUT)?})))?;
        serde_json::from_value::<List>(list).map(|l| l.tools).map_err(|e| CallError::Invalid(e.to_string()))
    }

    pub fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolResult, CallError> {
        let params = json!({"name": name, "arguments": arguments});
        let result = self.run(
            |hub| hub.request("tools/call", params.clone()),
            |link| Ok(serde_json::to_value(link.call_tool(name, arguments.clone(), TOOL_TIMEOUT)?).unwrap()),
        )?;
        serde_json::from_value(result).map_err(|e| CallError::Invalid(e.to_string()))
    }

    /// Speak through the board's speaker, in `language` (`it`, `en`) or the
    /// one the text looks like. Needs a bridge with text-to-speech (the app
    /// with spoken replies on, or `dualeye --tts`). Returns once it's played.
    pub fn say(&self, text: &str, language: Option<&str>) -> Result<Value, CallError> {
        let mut slot = self.hub.lock().unwrap();
        let hub = self
            .hub_client(&mut slot)
            .ok_or_else(|| CallError::Unavailable("speaking needs the DualEye app with spoken replies on, or dualeye --tts, running".into()))?;
        *self.route.lock().unwrap() = Some(Route::Hub);
        let reply = hub.request_waiting("host/say", json!({"text": text, "language": language}), SAY_TIMEOUT);
        if matches!(reply, Err(CallError::Io(_) | CallError::Closed | CallError::Invalid(_))) {
            *slot = None;
        }
        reply
    }

    /// Run `job` with a way to send the board JSON-RPC requests (`media/...`
    /// for an image upload): through the bridge, or over the port, opened once
    /// for the whole job.
    pub fn session<T>(&self, job: impl FnOnce(&dyn Fn(&str, Value, Duration) -> Result<Value, CallError>) -> Result<T, CallError>) -> Result<T, CallError> {
        {
            let mut slot = self.hub.lock().unwrap();
            if let Some(hub) = self.hub_client(&mut slot) {
                *self.route.lock().unwrap() = Some(Route::Hub);
                let hub = std::cell::RefCell::new(hub);
                let result = job(&|method, params, timeout| hub.borrow_mut().request_waiting(method, params, timeout));
                if matches!(result, Err(CallError::Io(_) | CallError::Closed | CallError::Invalid(_))) {
                    *slot = None;
                }
                return result;
            }
        }
        *self.route.lock().unwrap() = Some(Route::Direct);
        let link = self.open_direct()?;
        job(&|method, params, timeout| link.call(method, params, timeout))
    }

    /// Hand a Claude Code hook's JSON to the running bridge, for its alerts.
    /// Only through the hub: without a bridge there is nobody to tell.
    pub fn claude_hook(&self, event: Value) -> Result<Value, CallError> {
        let mut slot = self.hub.lock().unwrap();
        let hub = self.hub_client(&mut slot).ok_or_else(|| CallError::Unavailable("the DualEye app isn't running".into()))?;
        *self.route.lock().unwrap() = Some(Route::Hub);
        hub.request("host/claude_hook", event)
    }

    /// Run timer tool `name` ([`Timers::tools`]) on the running bridge's
    /// timers; without a bridge, on the ones kept on disk, which ring once
    /// the app or `dualeye` runs again. `Err` is the tool's failure, as text.
    pub fn timer_tool(&self, name: &str, arguments: &Value, language: &str) -> Result<String, String> {
        {
            let mut slot = self.hub.lock().unwrap();
            if let Some(hub) = self.hub_client(&mut slot) {
                *self.route.lock().unwrap() = Some(Route::Hub);
                match hub.request("host/timers", json!({"name": name, "arguments": arguments, "language": language})) {
                    Ok(reply) => {
                        let text = reply["text"].as_str().unwrap_or_default().to_string();
                        return if reply["is_error"].as_bool() == Some(true) { Err(text) } else { Ok(text) };
                    }
                    // An older hub, or one whose bridge stopped: the file then.
                    Err(CallError::Rpc { code: METHOD_NOT_FOUND, .. } | CallError::Unavailable(_)) => {}
                    Err(CallError::Rpc { message, .. }) => return Err(message),
                    Err(_) => *slot = None,
                }
            }
        }
        *self.route.lock().unwrap() = Some(Route::Direct);
        let timers = Timers::open(timers::default_file());
        timers.call_tool(name, arguments, language).unwrap_or_else(|| Err(format!("unknown timer tool `{name}`")))
    }

    /// The bridge's latest sample and its age; `None` without a bridge, or
    /// before it has sampled.
    pub fn bridge_snapshot(&self) -> Option<(Snapshot, Duration)> {
        let mut slot = self.hub.lock().unwrap();
        let hub = self.hub_client(&mut slot)?;
        let reply = hub.request("host/snapshot", json!({}));
        let reply = match reply {
            Ok(r) => r,
            Err(_) => {
                *slot = None;
                return None;
            }
        };
        let snapshot = serde_json::from_value(reply.get("snapshot")?.clone()).ok()?;
        Some((snapshot, Duration::from_millis(reply.get("age_ms")?.as_u64()?)))
    }

    fn hub_client<'a>(&self, slot: &'a mut Option<HubClient>) -> Option<&'a mut HubClient> {
        if slot.is_none() {
            *slot = HubClient::connect(self.file.as_deref()?, &self.client).ok().flatten().map(|(hub, _)| hub);
        }
        slot.as_mut()
    }

    fn run(
        &self,
        via_hub: impl Fn(&mut HubClient) -> Result<Value, CallError>,
        direct: impl FnOnce(&Link) -> Result<Value, CallError>,
    ) -> Result<Value, CallError> {
        {
            let mut slot = self.hub.lock().unwrap();
            // A second try for a connection the hub closed since the last call.
            for _ in 0..2 {
                let Some(hub) = self.hub_client(&mut slot) else { break };
                match via_hub(hub) {
                    Err(CallError::Io(_) | CallError::Closed | CallError::Invalid(_)) => *slot = None,
                    other => {
                        *self.route.lock().unwrap() = Some(Route::Hub);
                        return other;
                    }
                }
            }
        }
        *self.route.lock().unwrap() = Some(Route::Direct);
        direct(&self.open_direct()?)
    }

    fn open_direct(&self) -> Result<Link, CallError> {
        let port = self.port.clone().or_else(serial::detect_board).ok_or_else(|| CallError::Unavailable("board not found on USB".into()))?;
        let link = Link::open(&port, |_| {}).map_err(|e| CallError::Unavailable(format!("{port}: {e}")))?;
        link.handshake(Duration::from_secs(3)).map_err(|e| match e {
            CallError::Timeout => CallError::Unavailable(format!("{port}: the board does not answer (firmware 0.4 or later needed)")),
            other => other,
        })?;
        Ok(link)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub() -> (tempfile::TempDir, Arc<Hub>) {
        let dir = tempfile::tempdir().unwrap();
        let hub = Hub::start_at(Some(dir.path().join("hub.json"))).unwrap();
        (dir, hub)
    }

    fn client(dir: &tempfile::TempDir) -> HubClient {
        HubClient::connect(&dir.path().join("hub.json"), "test").unwrap().expect("hub running").0
    }

    #[test]
    fn needs_the_token() {
        let (_dir, hub) = hub();
        let stream = TcpStream::connect(hub.addr()).unwrap();
        let err = HubClient::handshake(stream, "not-the-token", "test").err().unwrap();
        assert!(err.to_string().contains("token"), "{err}");
    }

    #[test]
    fn without_a_board_calls_say_why() {
        let (dir, hub) = hub();
        let mut c = client(&dir);
        hub.set_unavailable("esptool is using the port");
        match c.request("tools/list", json!({})) {
            Err(CallError::Unavailable(why)) => assert_eq!(why, "esptool is using the port"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(c.request("nope", json!({})), Err(CallError::Rpc { code: METHOD_NOT_FOUND, .. })));
        assert_eq!(hub.status().clients, 1);
    }

    #[test]
    fn serves_the_latest_snapshot() {
        let (dir, hub) = hub();
        let mut c = client(&dir);
        assert_eq!(c.request("host/snapshot", json!({})).unwrap()["snapshot"], Value::Null);
        let snap: Snapshot = serde_json::from_value(json!({"v": 2, "ts": 5, "cpu": {"temp_c": 40.0}})).unwrap();
        hub.set_snapshot(&snap);
        let board = Board::with_hub_file(None, "test", Some(dir.path().join("hub.json")));
        let (got, age) = board.bridge_snapshot().unwrap();
        assert_eq!(got, snap);
        assert!(age < Duration::from_secs(1));
    }

    #[test]
    fn claude_hooks_reach_the_bridge() {
        let (dir, hub) = hub();
        let board = Board::with_hub_file(None, "test", Some(dir.path().join("hub.json")));
        assert_eq!(board.claude_hook(json!({"hook_event_name": "Stop"})).unwrap()["taken"], false);
        let got = Arc::new(Mutex::new(None));
        let sink = got.clone();
        hub.set_claude_hook(Some(Arc::new(move |v| *sink.lock().unwrap() = Some(v))));
        assert_eq!(board.claude_hook(json!({"hook_event_name": "Stop"})).unwrap()["taken"], true);
        assert_eq!(got.lock().unwrap().as_ref().unwrap()["hook_event_name"], "Stop");
    }

    #[test]
    fn file_goes_with_the_hub() {
        let (dir, hub) = hub();
        let path = dir.path().join("hub.json");
        assert!(path.exists());
        drop(hub);
        assert!(!path.exists());
        assert!(HubClient::connect(&path, "test").unwrap().is_none());
    }
}
