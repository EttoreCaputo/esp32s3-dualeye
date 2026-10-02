//! A protocol v2 session with the board (see `docs/protocol.md`): a reader
//! thread that decodes frames and hands JSON-RPC responses back to the
//! [`Link::call`] waiting for them, and a writer any thread can send on.

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use serialport::SerialPort;

use crate::protocol::{self, Channel, Chunk, Decoder, PROTOCOL};
use crate::serial;

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

pub struct Link {
    tx: Mutex<Box<dyn SerialPort>>,
    shared: Arc<Shared>,
    next_id: AtomicU64,
    reader: Option<JoinHandle<()>>,
}

impl Link {
    /// Open the port (without resetting the board) and start reading.
    pub fn open(port: &str, on_event: impl Fn(LinkEvent) + Send + 'static) -> io::Result<Self> {
        Self::new(serial::open(port)?, on_event)
    }

    pub fn new(port: Box<dyn SerialPort>, on_event: impl Fn(LinkEvent) + Send + 'static) -> io::Result<Self> {
        let rx = port.try_clone()?;
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
        Ok(Self { tx: Mutex::new(port), shared, next_id: AtomicU64::new(1), reader: Some(reader) })
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
        self.send_raw(&protocol::encode(channel, payload))
    }

    /// Bytes as they are, outside the framing (the protocol 1 version query).
    pub fn send_raw(&self, bytes: &[u8]) -> io::Result<()> {
        let mut tx = self.tx.lock().unwrap();
        tx.write_all(bytes)?;
        tx.flush()
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

    pub fn list_tools(&self, timeout: Duration) -> Result<Vec<Tool>, CallError> {
        #[derive(Deserialize)]
        struct List {
            tools: Vec<Tool>,
        }
        let reply = self.call("tools/list", json!({}), timeout)?;
        serde_json::from_value::<List>(reply).map(|l| l.tools).map_err(|e| CallError::Invalid(e.to_string()))
    }

    pub fn call_tool(&self, name: &str, arguments: Value, timeout: Duration) -> Result<ToolResult, CallError> {
        let reply = self.call("tools/call", json!({"name": name, "arguments": arguments}), timeout)?;
        serde_json::from_value(reply).map_err(|e| CallError::Invalid(e.to_string()))
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// Where the board's last tool list is kept, for when it doesn't answer.
pub(crate) fn tools_cache_file() -> Option<std::path::PathBuf> {
    crate::claude::data_dir().map(|d| d.join("board-tools.json"))
}

fn read_loop(mut rx: Box<dyn SerialPort>, shared: &Shared, on_event: &dyn Fn(LinkEvent)) {
    let mut decoder = Decoder::new();
    let mut text = TextLines::default();
    let mut buf = [0u8; 1024];
    let mut handle = |chunk: Chunk| match chunk {
        Chunk::Frame(frame) => {
            text.flush(on_event);
            on_frame(frame, shared, on_event);
        }
        Chunk::Text(t) => text.push(&t, on_event),
        Chunk::Bad => {}
    };
    while !shared.stop.load(Ordering::Relaxed) {
        match rx.read(&mut buf) {
            Ok(0) => decoder.idle(&mut handle),
            Ok(n) => decoder.push(&buf[..n], &mut handle),
            Err(e) if e.kind() == io::ErrorKind::TimedOut => decoder.idle(&mut handle),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => {
                *shared.closed_reason.lock().unwrap() = Some(e.to_string());
                shared.alive.store(false, Ordering::Relaxed);
                // Dropping the senders wakes every waiting call with Closed.
                shared.pending.lock().unwrap().clear();
                on_event(LinkEvent::Closed(e.to_string()));
                return;
            }
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
