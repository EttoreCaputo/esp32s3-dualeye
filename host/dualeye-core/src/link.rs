//! A protocol v2 session with the board (see `docs/protocol.md`): a reader
//! thread that decodes frames and hands JSON-RPC responses back to the
//! [`Link::call`] waiting for them, and a writer any thread can send on.

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use serialport::SerialPort;

use crate::protocol::{self, Channel, Chunk, Decoder, Frame, PROTOCOL};
use crate::serial;
use crate::wifi::{self, Cipher};

/// What the board sends besides responses.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEvent {
    /// A line from the board's log.
    Log(String),
    /// A line of plain console text from outside the framing: ROM,
    /// bootloader, a crash, or firmware before protocol v2.
    Text(String),
    /// A JSON-RPC notification, such as `ready` after a reboot.
    Notification { method: String, params: Value },
    /// An `audio_up` frame: header and PCM, see [`crate::voice`].
    Audio(Vec<u8>),
    /// Reading the port failed; the link is dead.
    Closed(String),
}

#[derive(Debug)]
pub enum CallError {
    Io(io::Error),
    /// No response in time: the board is busy, rebooting, or doesn't speak protocol v2.
    Timeout,
    /// The link went down while waiting.
    Closed,
    /// The board answered with a JSON-RPC error.
    Rpc { code: i64, message: String },
    /// A response that doesn't have the expected shape.
    Invalid(String),
    /// No board to send it to: not found on USB, not answering, or held by
    /// esptool. The text says which.
    Unavailable(String),
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Io(e) => write!(f, "{e}"),
            CallError::Timeout => write!(f, "the board did not answer"),
            CallError::Closed => write!(f, "the connection to the board closed"),
            CallError::Rpc { code, message } => write!(f, "{message} ({code})"),
            CallError::Invalid(why) => write!(f, "unexpected reply: {why}"),
            CallError::Unavailable(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for CallError {}

impl From<io::Error> for CallError {
    fn from(e: io::Error) -> Self {
        CallError::Io(e)
    }
}

/// The board's answer to `hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    pub firmware: String,
    #[serde(default)]
    pub idf: Option<String>,
    #[serde(default)]
    pub board: Option<String>,
    #[serde(default)]
    pub max_payload: Option<usize>,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// The board's id, its WiFi MAC (firmware 1.4.3).
    #[serde(default)]
    pub id: Option<String>,
    /// How this host reaches it: `usb` or `wifi` (firmware 1.4.3).
    #[serde(default)]
    pub transport: Option<String>,
}

/// A board tool, as `tools/list` describes it (the MCP `Tool` shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
}

/// What `tools/call` returned (the MCP `CallToolResult` shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    #[serde(default)]
    pub content: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(default)]
    pub is_error: bool,
}

impl ToolResult {
    /// The text parts of the content, one per line.
    pub fn text(&self) -> String {
        let parts: Vec<&str> = self.content.iter().filter_map(|c| c.get("text")?.as_str()).collect();
        parts.join("\n")
    }
}

type Reply = Result<Value, CallError>;

struct Shared {
    pending: Mutex<HashMap<u64, mpsc::Sender<Reply>>>,
    stop: AtomicBool,
    alive: AtomicBool,
    closed_reason: Mutex<Option<String>>,
}

/// Where frames go: the port, or the WiFi session sealing each one.
struct Tx {
    out: Box<dyn Write + Send>,
    seal: Option<Cipher>,
}

/// How the reader thread gets its bytes.
struct Rx {
    input: Box<dyn Read + Send>,
    /// TCP: a read of nothing is the board hanging up. On a serial port it's
    /// just the timeout.
    eof_closes: bool,
    decoder: Decoder,
    /// Frames read before the reader started (the WiFi handshake's).
    pending: Vec<Frame>,
    open: Option<Cipher>,
}

pub struct Link {
    tx: Mutex<Tx>,
    shared: Arc<Shared>,
    next_id: AtomicU64,
    reader: Option<JoinHandle<()>>,
    /// The board's address on WiFi; the socket, to hang up on drop.
    peer: Option<(SocketAddr, TcpStream)>,
}

impl Link {
    /// Open the port (without resetting the board) and start reading. `wifi`
    /// or `wifi:<id>` is a paired board on the network ([`wifi::open`]).
    pub fn open(port: &str, on_event: impl Fn(LinkEvent) + Send + 'static) -> io::Result<Self> {
        if wifi::target(port).is_some() {
            return Self::wifi(wifi::open(port)?, on_event);
        }
        Self::new(serial::open(port)?, on_event)
    }

    pub fn new(port: Box<dyn SerialPort>, on_event: impl Fn(LinkEvent) + Send + 'static) -> io::Result<Self> {
        let rx = Rx { input: port.try_clone()?, eof_closes: false, decoder: Decoder::new(), pending: Vec::new(), open: None };
        Self::start(Tx { out: Box::new(port), seal: None }, rx, None, on_event)
    }

    /// A WiFi session that passed the handshake.
    pub fn wifi(session: wifi::Session, on_event: impl Fn(LinkEvent) + Send + 'static) -> io::Result<Self> {
        let wifi::Session { stream, addr, tx, rx, decoder, pending } = session;
        stream.set_read_timeout(Some(Duration::from_millis(200)))?;
        let input = stream.try_clone()?;
        let hangup = stream.try_clone()?;
        let rx = Rx { input: Box::new(input), eof_closes: true, decoder, pending, open: Some(rx) };
        Self::start(Tx { out: Box::new(stream), seal: Some(tx) }, rx, Some((addr, hangup)), on_event)
    }

    fn start(tx: Tx, rx: Rx, peer: Option<(SocketAddr, TcpStream)>, on_event: impl Fn(LinkEvent) + Send + 'static) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            pending: Mutex::default(),
            stop: AtomicBool::new(false),
            alive: AtomicBool::new(true),
            closed_reason: Mutex::default(),
        });
        let reader = {
            let shared = shared.clone();
            thread::Builder::new().name("dualeye-link".into()).spawn(move || read_loop(rx, &shared, &on_event))?
        };
        Ok(Self { tx: Mutex::new(tx), shared, next_id: AtomicU64::new(1), reader: Some(reader), peer })
    }

    /// The board's address, when this link is on WiFi.
    pub fn peer(&self) -> Option<SocketAddr> {
        self.peer.as_ref().map(|(addr, _)| *addr)
    }

    /// False once reading the port has failed (unplugged, or the board reset its USB).
    pub fn is_alive(&self) -> bool {
        self.shared.alive.load(Ordering::Relaxed)
    }

    /// Why the link died, once [`Link::is_alive`] is false.
    pub fn closed_reason(&self) -> Option<String> {
        self.shared.closed_reason.lock().unwrap().clone()
    }

    pub fn send(&self, channel: Channel, payload: &[u8]) -> io::Result<()> {
        if payload.len() > protocol::MAX_PAYLOAD {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "payload too large for one frame"));
        }
        let mut tx = self.tx.lock().unwrap();
        let bytes = match tx.seal.as_mut() {
            Some(seal) => protocol::encode_raw(channel as u8, &seal.seal(channel as u8, payload)),
            None => protocol::encode(channel, payload),
        };
        tx.out.write_all(&bytes)?;
        tx.out.flush()
    }

    /// Bytes as they are, outside the framing (the protocol 1 version query).
    /// Not on WiFi, where there is no firmware that old.
    pub fn send_raw(&self, bytes: &[u8]) -> io::Result<()> {
        let mut tx = self.tx.lock().unwrap();
        if tx.seal.is_some() {
            return Ok(());
        }
        tx.out.write_all(bytes)?;
        tx.out.flush()
    }

    /// Send a JSON-RPC request and wait for its response.
    pub fn call(&self, method: &str, params: Value, timeout: Duration) -> Reply {
        if !self.is_alive() {
            return Err(CallError::Closed);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply_tx, reply_rx) = mpsc::channel();
        self.shared.pending.lock().unwrap().insert(id, reply_tx);
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let sent = self.send(Channel::Ctrl, request.to_string().as_bytes());
        let reply = match sent {
            Ok(()) => reply_rx.recv_timeout(timeout).unwrap_or_else(|e| match e {
                mpsc::RecvTimeoutError::Timeout => Err(CallError::Timeout),
                mpsc::RecvTimeoutError::Disconnected => Err(CallError::Closed),
            }),
            Err(e) => Err(CallError::Io(e)),
        };
        self.shared.pending.lock().unwrap().remove(&id);
        reply
    }

    /// Send a JSON-RPC notification: no `id`, so the board doesn't answer.
    pub fn notify(&self, method: &str, params: Value) -> io::Result<()> {
        let message = json!({"jsonrpc": "2.0", "method": method, "params": params});
        self.send(Channel::Ctrl, message.to_string().as_bytes())
    }

    /// One `hello`; the handshake retries it while the board boots.
    pub fn hello(&self, timeout: Duration) -> Result<Hello, CallError> {
        let client = format!("dualeye-core/{}", env!("CARGO_PKG_VERSION"));
        let reply = self.call("hello", json!({"protocol": PROTOCOL, "client": client}), timeout)?;
        serde_json::from_value(reply).map_err(|e| CallError::Invalid(e.to_string()))
    }

    /// `hello` every 500 ms until the board answers or `total` has passed,
    /// for a board that opening the port may have rebooted.
    pub fn handshake(&self, total: Duration) -> Result<Hello, CallError> {
        let each = Duration::from_millis(500);
        let tries = (total.as_millis() / each.as_millis()).max(1);
        for _ in 1..tries {
            match self.hello(each) {
                Err(CallError::Timeout) => continue,
                other => return other,
            }
        }
        self.hello(each)
    }

    /// All the board's tools: a frame holds only so many (4 KB), so the
    /// board hands them out in pages (firmware 1.3.1), each `timeout` long.
    pub fn list_tools(&self, timeout: Duration) -> Result<Vec<Tool>, CallError> {
        #[derive(Deserialize)]
        struct List {
            tools: Vec<Tool>,
            #[serde(rename = "nextCursor")]
            next_cursor: Option<String>,
        }
        let mut tools = Vec::new();
        let mut params = json!({});
        for _ in 0..MAX_TOOL_PAGES {
            let reply = self.call("tools/list", params, timeout)?;
            let page = serde_json::from_value::<List>(reply).map_err(|e| CallError::Invalid(e.to_string()))?;
            tools.extend(page.tools);
            match page.next_cursor {
                Some(cursor) => params = json!({"cursor": cursor}),
                None => return Ok(tools),
            }
        }
        Err(CallError::Invalid("too many pages of tools".into()))
    }

    pub fn call_tool(&self, name: &str, arguments: Value, timeout: Duration) -> Result<ToolResult, CallError> {
        let reply = self.call("tools/call", json!({"name": name, "arguments": arguments}), timeout)?;
        serde_json::from_value(reply).map_err(|e| CallError::Invalid(e.to_string()))
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some((_, socket)) = &self.peer {
            // The board learns at once, and goes back to USB.
            let _ = socket.shutdown(Shutdown::Both);
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// Pages of `tools/list` before giving up on a board that never ends them.
const MAX_TOOL_PAGES: usize = 16;

/// Where the board's last tool list is kept, for when it doesn't answer.
pub(crate) fn tools_cache_file() -> Option<std::path::PathBuf> {
    crate::claude::data_dir().map(|d| d.join("board-tools.json"))
}

/// Keep `tools` as the board's last tool list ([`tools_cache_file`]).
pub(crate) fn save_tools_cache(tools: &[Tool]) {
    if let Some(path) = tools_cache_file()
        && let Ok(json) = serde_json::to_vec_pretty(tools)
    {
        let _ = std::fs::write(path, json);
    }
}

fn read_loop(mut rx: Rx, shared: &Shared, on_event: &dyn Fn(LinkEvent)) {
    let mut text = TextLines::default();
    let mut buf = [0u8; 1024];
    // A sealed frame that doesn't open: tampered with, or out of step.
    let broken = std::cell::Cell::new(false);
    let open = &mut rx.open;
    let mut handle = |chunk: Chunk| match chunk {
        Chunk::Frame(mut frame) => {
            text.flush(on_event);
            if let Some(cipher) = open.as_mut() {
                match cipher.open(frame.channel, &frame.payload) {
                    Some(plain) => frame.payload = plain,
                    None => {
                        broken.set(true);
                        return;
                    }
                }
            }
            on_frame(frame, shared, on_event);
        }
        Chunk::Text(t) => text.push(&t, on_event),
        Chunk::Bad => {}
    };
    for frame in std::mem::take(&mut rx.pending) {
        handle(Chunk::Frame(frame));
    }
    let close = |why: String| {
        *shared.closed_reason.lock().unwrap() = Some(why.clone());
        shared.alive.store(false, Ordering::Relaxed);
        // Dropping the senders wakes every waiting call with Closed.
        shared.pending.lock().unwrap().clear();
        on_event(LinkEvent::Closed(why));
    };
    while !shared.stop.load(Ordering::Relaxed) {
        match rx.input.read(&mut buf) {
            Ok(0) if rx.eof_closes => {
                drop(handle);
                return close("the board closed the connection".into());
            }
            Ok(0) => rx.decoder.idle(&mut handle),
            Ok(n) => rx.decoder.push(&buf[..n], &mut handle),
            Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => rx.decoder.idle(&mut handle),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => {
                drop(handle);
                return close(e.to_string());
            }
        }
        if broken.get() {
            drop(handle);
            return close("a frame from the board didn't check out".into());
        }
    }
}

fn on_frame(frame: protocol::Frame, shared: &Shared, on_event: &dyn Fn(LinkEvent)) {
    match frame.channel {
        c if c == Channel::Log as u8 => {
            let line = String::from_utf8_lossy(&frame.payload).trim_end().to_string();
            on_event(LinkEvent::Log(line));
        }
        c if c == Channel::AudioUp as u8 => on_event(LinkEvent::Audio(frame.payload)),
        c if c == Channel::Ctrl as u8 => {
            let Ok(msg) = serde_json::from_slice::<Value>(&frame.payload) else {
                return;
            };
            if let Some(id) = msg.get("id").and_then(Value::as_u64) {
                let reply = match (msg.get("result"), msg.get("error")) {
                    (Some(result), _) => Ok(result.clone()),
                    (None, Some(err)) => Err(CallError::Rpc {
                        code: err.get("code").and_then(Value::as_i64).unwrap_or(0),
                        message: err.get("message").and_then(Value::as_str).unwrap_or("error").to_string(),
                    }),
                    (None, None) => return,
                };
                if let Some(waiter) = shared.pending.lock().unwrap().remove(&id) {
                    let _ = waiter.send(reply);
                }
            } else if let Some(method) = msg.get("method").and_then(Value::as_str) {
                let params = msg.get("params").cloned().unwrap_or(Value::Null);
                on_event(LinkEvent::Notification { method: method.to_string(), params });
            }
        }
        _ => {}
    }
}

/// Raw console text arrives in pieces cut wherever a frame came in between;
/// hand it on a line at a time.
#[derive(Default)]
struct TextLines {
    partial: String,
}

impl TextLines {
    fn push(&mut self, text: &str, on_event: &dyn Fn(LinkEvent)) {
        self.partial.push_str(text);
        while let Some(end) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=end).collect();
            emit_text(&line, on_event);
        }
    }

    /// A frame came: whatever text came before it is complete.
    fn flush(&mut self, on_event: &dyn Fn(LinkEvent)) {
        if !self.partial.is_empty() {
            emit_text(&std::mem::take(&mut self.partial), on_event);
        }
    }
}

fn emit_text(line: &str, on_event: &dyn Fn(LinkEvent)) {
    let line = line.trim_end();
    if !line.is_empty() {
        on_event(LinkEvent::Text(line.to_string()));
    }
}
