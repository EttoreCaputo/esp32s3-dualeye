//! The language model: a local one with a [llama.cpp](https://github.com/ggml-org/llama.cpp)
//! `llama-server` sidecar on `127.0.0.1`, or a provider's ([`crate::cloud`]).
//!
//! [`Llm`] starts the server in the background with the model loaded,
//! restarts it if it dies and stops it when dropped ([`crate::sidecar`]).
//! Requests go to its OpenAI-style `/v1/chat/completions`, with the chat
//! template's own tool calling (`--jinja`) and thinking turned off: a voice
//! command wants an answer, not a train of thought. The server keeps the
//! prompt's common prefix (system prompt and tools) cached between requests.
//! A cloud model gets the same requests, less what only llama.cpp reads.

use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::cloud::{self, CloudError};
use crate::sidecar::{self, Process};
use crate::stt::models_dir;

/// Loading a 4B model from a cold disk can take a while.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(180);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Tokens of context: the system prompt and tools take about 1,500, a few
/// turns of conversation the rest.
const CONTEXT: u32 = 8192;

#[derive(Debug, Clone, PartialEq)]
pub enum LlmConfig {
    Local(LocalLlm),
    /// A provider's model, with its key.
    Cloud(cloud::Client),
}

impl LlmConfig {
    /// It runs on this computer, and caches the prompt's prefix.
    pub fn is_local(&self) -> bool {
        matches!(self, LlmConfig::Local(_))
    }

    /// The local model's file.
    pub fn model_path(&self) -> Option<&std::path::Path> {
        match self {
            LlmConfig::Local(local) => Some(&local.model),
            LlmConfig::Cloud(_) => None,
        }
    }

    /// What it is, for a log: the model's path, or the cloud model's id.
    pub fn label(&self) -> String {
        match self {
            LlmConfig::Local(local) => local.model.display().to_string(),
            LlmConfig::Cloud(client) => client.model.id(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalLlm {
    /// The `llama-server` binary.
    pub server: PathBuf,
    /// A GGUF model with a chat template that does tool calls.
    pub model: PathBuf,
    /// Layers to put on the GPU: `None` lets llama.cpp decide (all of them on
    /// Apple silicon and a big enough NVIDIA card), `Some(0)` runs on the CPU.
    pub gpu_layers: Option<u32>,
}

/// `llama-server`: `DUALEYE_LLAMA_SERVER`, the one the app ships, or one on
/// the PATH (see [`crate::sidecar::find_program`]).
pub fn find_server() -> Option<PathBuf> {
    crate::sidecar::find_program("llama-server", "DUALEYE_LLAMA_SERVER")
}

/// The devices `server` can run a model on besides the processor, as it
/// lists them (`--list-devices`): "MTL0: Apple M1 Pro (12124 MiB, …)",
/// "CUDA0: NVIDIA GeForce RTX 4070 (…)". `None` when it couldn't be asked.
pub fn devices(server: &std::path::Path) -> Option<Vec<String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut cmd = Command::new(server);
    cmd.arg("--list-devices").stdin(std::process::Stdio::null());
    // The first run on a Mac compiles Metal's shaders: seconds. A hung one
    // is left behind rather than waited for.
    thread::spawn(move || {
        let _ = tx.send(cmd.output());
    });
    let out = rx.recv_timeout(Duration::from_secs(60)).ok()?.ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    Some(parse_devices(&text))
}

fn parse_devices(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|l| !l.trim_start().starts_with("Available devices"))
        .skip(1)
        .map(str::trim)
        .take_while(|l| l.contains(':'))
        .filter(|l| !["BLAS", "CPU"].iter().any(|cpu| l.starts_with(cpu)))
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn devices_without_the_processor() {
        let out = "0.00.000.088 I srv  llama_server: initializing ...\nAvailable devices:\n  BLAS: Accelerate (0 MiB, 0 MiB free)\n  MTL0: Apple M1 Pro (12124 MiB, 12123 MiB free)\n";
        assert_eq!(super::parse_devices(out), ["MTL0: Apple M1 Pro (12124 MiB, 12123 MiB free)"]);
        assert!(super::parse_devices("Available devices:\n").is_empty());
    }
}

#[derive(Debug)]
pub enum LlmError {
    /// The server couldn't be started, or died and couldn't be restarted.
    Server(String),
    Io(io::Error),
    /// The server answered something unexpected.
    Invalid(String),
    Cloud(CloudError),
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::Server(why) | LlmError::Invalid(why) => write!(f, "llama-server: {why}"),
            LlmError::Io(e) => write!(f, "llama-server: {e}"),
            LlmError::Cloud(e) => e.fmt(f),
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

    /// Start the server now (loading the model takes seconds); for a cloud
    /// model, check its key.
    pub fn warm_up(&self) -> Result<(), LlmError> {
        match &self.config {
            LlmConfig::Local(_) => self.addr().map(|_| ()),
            LlmConfig::Cloud(client) => client.check().map_err(LlmError::Cloud),
        }
    }

    /// One `/v1/chat/completions` request (`messages`, `tools` and the
    /// sampling settings in `request`): the first choice's message.
    pub fn chat(&self, request: &Value) -> Result<Value, LlmError> {
        if let LlmConfig::Cloud(client) = &self.config {
            return client.chat(request).map_err(LlmError::Cloud);
        }
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
        let LlmConfig::Local(config) = &self.config else { return Err(LlmError::Server("a cloud model has no server".into())) };
        let s = start(config)?;
        let addr = s.addr;
        *server = Some(s);
        Ok(addr)
    }
}

fn start(config: &LocalLlm) -> Result<Process, LlmError> {
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
