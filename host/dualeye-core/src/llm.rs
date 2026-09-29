//! A local language model with a [llama.cpp](https://github.com/ggml-org/llama.cpp)
//! `llama-server` sidecar on `127.0.0.1`.
//!
//! [`Llm`] starts the server in the background with the model loaded,
//! restarts it if it dies and stops it when dropped ([`crate::sidecar`]).
//! Requests go to its OpenAI-style `/v1/chat/completions`, with the chat
//! template's own tool calling (`--jinja`) and thinking turned off: a voice
//! command wants an answer, not a train of thought. The server keeps the
//! prompt's common prefix (system prompt and tools) cached between requests.

use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::sidecar::{self, Process};
use crate::stt::models_dir;

/// Loading a 4B model from a cold disk can take a while.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(180);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Tokens of context: the system prompt and tools take about 1,500, a few
/// turns of conversation the rest.
const CONTEXT: u32 = 8192;

#[derive(Debug, Clone, PartialEq)]
pub struct LlmConfig {
    /// The `llama-server` binary.
    pub server: PathBuf,
    /// A GGUF model with a chat template that does tool calls.
    pub model: PathBuf,
    /// Layers to put on the GPU: `None` lets llama.cpp decide (all of them on
    /// Apple silicon and a big enough NVIDIA card), `Some(0)` runs on the CPU.
    pub gpu_layers: Option<u32>,
}

/// `llama-server` from the PATH, or where Homebrew puts it.
pub fn find_server() -> Option<PathBuf> {
    let name = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from))
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

#[derive(Debug)]
pub enum LlmError {
    /// The server couldn't be started, or died and couldn't be restarted.
    Server(String),
    Io(io::Error),
    /// The server answered something unexpected.
    Invalid(String),
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::Server(why) | LlmError::Invalid(why) => write!(f, "llama-server: {why}"),
            LlmError::Io(e) => write!(f, "llama-server: {e}"),
        }
    }
}

impl std::error::Error for LlmError {}

impl From<io::Error> for LlmError {
    fn from(e: io::Error) -> Self {
        LlmError::Io(e)
    }
}

/// The sidecar. Shared between connections: it outlives a board reconnect.
pub struct Llm {
    config: LlmConfig,
    server: Mutex<Option<Process>>,
}

impl fmt::Debug for Llm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Llm").field("config", &self.config).finish_non_exhaustive()
    }
}

impl Llm {
    pub fn new(config: LlmConfig) -> Self {
        Self { config, server: Mutex::new(None) }
    }

    pub fn config(&self) -> &LlmConfig {
        &self.config
    }

    /// Stop the server now, even while others still hold this [`Llm`]; the
    /// next request would start it again.
    pub fn shutdown(&self) {
        self.server.lock().unwrap().take();
    }

    /// Start the server now (loading the model takes seconds).
    pub fn warm_up(&self) -> Result<(), LlmError> {
        self.addr().map(|_| ())
    }

    /// One `/v1/chat/completions` request (`messages`, `tools` and the
    /// sampling settings in `request`): the first choice's message.
    pub fn chat(&self, request: &Value) -> Result<Value, LlmError> {
        let body = serde_json::to_vec(request).map_err(|e| LlmError::Invalid(e.to_string()))?;
        // A server that died since the last request gets one restart.
        for attempt in 0..2 {
            let addr = self.addr()?;
            match sidecar::post(addr, "/v1/chat/completions", "application/json", &body, REQUEST_TIMEOUT) {
                Ok(reply) => {
                    let reply: Value = serde_json::from_slice(&reply).map_err(|e| LlmError::Invalid(e.to_string()))?;
                    return reply
                        .pointer("/choices/0/message")
                        .cloned()
                        .ok_or_else(|| LlmError::Invalid(format!("no message in {}", reply.to_string().chars().take(300).collect::<String>())));
                }
                Err(e) if attempt == 0 && matches!(e.kind(), io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset) => {
                    self.server.lock().unwrap().take();
                }
                Err(e) => return Err(e.into()),
            }
        }
        unreachable!()
    }

    fn addr(&self) -> Result<SocketAddr, LlmError> {
        let mut server = self.server.lock().unwrap();
        if let Some(s) = server.as_mut() {
            if s.is_running() {
                return Ok(s.addr);
            }
            *server = None;
        }
        let s = start(&self.config)?;
        let addr = s.addr;
        *server = Some(s);
        Ok(addr)
    }
}

fn start(config: &LlmConfig) -> Result<Process, LlmError> {
    if !config.model.is_file() {
        return Err(LlmError::Server(format!("no model at {}", config.model.display())));
    }
    let mut cmd = Command::new(&config.server);
    cmd.arg("--model").arg(&config.model);
    cmd.args(["--jinja", "--reasoning", "off", "--ctx-size", &CONTEXT.to_string(), "--parallel", "1", "--no-webui"]);
    if let Some(n) = config.gpu_layers {
        cmd.args(["--gpu-layers", &n.to_string()]);
        if n == 0 {
            cmd.args(["--device", "none"]);
        }
    }
    let dir = models_dir();
    let (pid_file, log) = (dir.as_ref().map(|d| d.join("llama-server.pid")), dir.as_ref().map(|d| d.join("llama-server.log")));
    let started = Instant::now();
    let mut process = Process::start(cmd, "llama-server", pid_file, log, STARTUP_TIMEOUT).map_err(LlmError::Server)?;
    // It listens at once, and answers 503 until the model is loaded.
    while sidecar::get(process.addr, "/health", Duration::from_secs(5)).is_err() {
        if !process.is_running() {
            return Err(LlmError::Server("exited while loading the model (see llama-server.log)".into()));
        }
        if started.elapsed() > STARTUP_TIMEOUT {
            return Err(LlmError::Server("didn't load the model in time".into()));
        }
        thread::sleep(Duration::from_millis(200));
    }
    Ok(process)
}
