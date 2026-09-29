//! Speech-to-text with a [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
//! `whisper-server` sidecar on `127.0.0.1`.
//!
//! [`Stt`] starts the server in the background with the model loaded, so the
//! first utterance doesn't wait for it, restarts it if it dies, and stops it
//! when dropped. Requests are plain HTTP on the loopback, so no HTTP client
//! is needed.
//!
//! With [`SttLanguage::Auto`] Whisper detects the language (10 out of 10
//! short commands right from the board's mic, 0.67–0.997); when it picks
//! neither Italian nor English, the utterance is decoded again in whichever
//! of the two it found more likely. Comparing the log probability of an
//! Italian and an English decoding doesn't work: Whisper forced into the
//! wrong language is often just as sure of itself.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::snapshot::Face;
use crate::voice;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Which language to transcribe in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SttLanguage {
    /// Italian or English, whichever fits better.
    #[default]
    Auto,
    It,
    En,
}

impl SttLanguage {
    fn code(self) -> &'static str {
        match self {
            SttLanguage::Auto => "auto",
            SttLanguage::It => "it",
            SttLanguage::En => "en",
        }
    }
}

/// The languages [`SttLanguage::Auto`] settles on.
const LANGUAGES: [&str; 2] = ["it", "en"];

impl FromStr for SttLanguage {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "auto" => Ok(SttLanguage::Auto),
            "it" => Ok(SttLanguage::It),
            "en" => Ok(SttLanguage::En),
            _ => Err(format!("unknown language {s:?}: auto, it or en")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SttConfig {
    /// The `whisper-server` binary.
    pub server: PathBuf,
    /// A ggml Whisper model (multilingual, e.g. `ggml-small.bin`).
    pub model: PathBuf,
    pub language: SttLanguage,
}

/// Where Whisper models are kept: `models/` in DualEye's data folder.
pub fn models_dir() -> Option<PathBuf> {
    crate::claude::data_dir().map(|d| d.join("models"))
}

/// `whisper-server` from the PATH, or where Homebrew puts it.
pub fn find_server() -> Option<PathBuf> {
    let name = if cfg!(windows) { "whisper-server.exe" } else { "whisper-server" };
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from))
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Transcript {
    pub text: String,
    /// `it` or `en`.
    pub language: String,
    /// Token-weighted average log probability of the decoding (closer to 0 is surer).
    pub logprob: f64,
    /// Time spent transcribing.
    pub elapsed_ms: u64,
}

#[derive(Debug)]
pub enum SttError {
    /// The server couldn't be started, or died and couldn't be restarted.
    Server(String),
    Io(io::Error),
    /// The server answered something unexpected.
    Invalid(String),
}

impl fmt::Display for SttError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SttError::Server(why) => write!(f, "whisper-server: {why}"),
            SttError::Io(e) => write!(f, "whisper-server: {e}"),
            SttError::Invalid(why) => write!(f, "whisper-server: {why}"),
        }
    }
}

impl std::error::Error for SttError {}

impl From<io::Error> for SttError {
    fn from(e: io::Error) -> Self {
        SttError::Io(e)
    }
}

struct Server {
    child: Child,
    addr: SocketAddr,
}

/// The sidecar. Shared between connections: it outlives a board reconnect.
pub struct Stt {
    config: SttConfig,
    server: Mutex<Option<Server>>,
}

impl Stt {
    pub fn new(config: SttConfig) -> Self {
        Self { config, server: Mutex::new(None) }
    }

    pub fn config(&self) -> &SttConfig {
        &self.config
    }

    /// Start the server now (it takes a few seconds to load the model).
    pub fn warm_up(&self) -> Result<(), SttError> {
        self.addr().map(|_| ())
    }

    /// Transcribe 16 kHz mono samples.
    pub fn transcribe(&self, samples: &[i16]) -> Result<Transcript, SttError> {
        let started = Instant::now();
        let wav = voice::wav(samples, voice::SAMPLE_RATE);
        let lang = self.config.language.code();
        let reply = self.request(&wav, lang)?;
        let mut t = parse_reply(&reply)?;
        if !LANGUAGES.contains(&t.language.as_str()) {
            let probs = &reply["language_probabilities"];
            let p = |l: &str| probs[l].as_f64().unwrap_or(0.0);
            let lang = if p("en") > p("it") { "en" } else { "it" };
            t = parse_reply(&self.request(&wav, lang)?)?;
        }
        t.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(t)
    }

    fn request(&self, wav: &[u8], lang: &str) -> Result<Value, SttError> {
        let prompt = vocabulary_prompt(lang);
        let fields = [("language", lang), ("response_format", "verbose_json"), ("temperature", "0"), ("prompt", &prompt)];
        // A server that died since the last request gets one restart.
        for attempt in 0..2 {
            let addr = self.addr()?;
            match post_inference(addr, wav, &fields) {
                Ok(body) => return serde_json::from_slice(&body).map_err(|e| SttError::Invalid(e.to_string())),
                Err(e) if attempt == 0 && matches!(e.kind(), io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset) => {
                    self.server.lock().unwrap().take();
                }
                Err(e) => return Err(e.into()),
            }
        }
        unreachable!()
    }

    /// The running server's address, starting it if needed.
    fn addr(&self) -> Result<SocketAddr, SttError> {
        let mut server = self.server.lock().unwrap();
        if let Some(s) = server.as_mut() {
            match s.child.try_wait() {
                Ok(None) => return Ok(s.addr),
                _ => *server = None,
            }
        }
        let s = start(&self.config)?;
        let addr = s.addr;
        *server = Some(s);
        Ok(addr)
    }
}

impl fmt::Debug for Stt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stt").field("config", &self.config).finish_non_exhaustive()
    }
}

impl Drop for Stt {
    fn drop(&mut self) {
        if let Some(mut s) = self.server.lock().unwrap().take() {
            let _ = s.child.kill();
            let _ = s.child.wait();
            if let Some(f) = pid_file() {
                let _ = fs::remove_file(f);
            }
        }
    }
}

/// The last server we started, so one left running by a host that was
/// killed is stopped by the next.
fn pid_file() -> Option<PathBuf> {
    models_dir().map(|d| d.join("whisper-server.pid"))
}

fn kill_stale_server() {
    let Some(pid) = pid_file().and_then(|f| fs::read_to_string(f).ok()).and_then(|s| s.trim().parse::<usize>().ok()) else {
        return;
    };
    let pid = sysinfo::Pid::from(pid);
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    if let Some(p) = sys.process(pid)
        && p.name().to_string_lossy().starts_with("whisper-server")
    {
        p.kill();
    }
}

fn start(config: &SttConfig) -> Result<Server, SttError> {
    if !config.model.is_file() {
        return Err(SttError::Server(format!("no model at {}", config.model.display())));
    }
    kill_stale_server();
    let port = free_port()?;
    // Its log goes next to the models, for when it won't start.
    let log = models_dir().and_then(|d| fs::create_dir_all(&d).ok().and_then(|_| File::create(d.join("whisper-server.log")).ok()));
    let mut child = Command::new(&config.server)
        .arg("--model")
        .arg(&config.model)
        .args(["--host", "127.0.0.1", "--port", &port.to_string(), "--language", "auto"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log.map_or_else(Stdio::null, Stdio::from))
        .spawn()
        .map_err(|e| SttError::Server(format!("{}: {e}", config.server.display())))?;
    if let Some(f) = pid_file() {
        let _ = fs::write(f, child.id().to_string());
    }
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(SttError::Server(format!("exited at startup ({status})")));
        }
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            return Ok(Server { child, addr });
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Err(SttError::Server("didn't start in time".into()));
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn free_port() -> io::Result<u16> {
    Ok(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?.local_addr()?.port())
}

/// Words the commands use, so Whisper spells them the way the tools do. In
/// the language decoded: a prompt in the other one drags the text into it,
/// so while detecting only the names, which belong to neither.
fn vocabulary_prompt(lang: &str) -> String {
    let faces = Face::ALL.iter().map(|f| f.name()).collect::<Vec<_>>().join(", ");
    match lang {
        "it" => format!("DualEye. Facce: {faces}. Schermo sinistro, destro. Luminosità, rotazione."),
        "en" => format!("DualEye. Faces: {faces}. Left screen, right screen. Brightness, rotation."),
        _ => format!("DualEye: {faces}."),
    }
}

/// `verbose_json` from whisper-server → a transcript.
fn parse_reply(reply: &Value) -> Result<Transcript, SttError> {
    if let Some(err) = reply.get("error") {
        return Err(SttError::Invalid(err.to_string()));
    }
    let text = reply["text"].as_str().ok_or_else(|| SttError::Invalid("no text".into()))?;
    let (mut sum, mut tokens) = (0.0, 0usize);
    for seg in reply["segments"].as_array().into_iter().flatten() {
        let n = seg["tokens"].as_array().map_or(1, |t| t.len().max(1));
        sum += seg["avg_logprob"].as_f64().unwrap_or(-10.0) * n as f64;
        tokens += n;
    }
    Ok(Transcript {
        text: text.trim().to_string(),
        language: language_code(reply["language"].as_str().unwrap_or("")),
        logprob: if tokens > 0 { sum / tokens as f64 } else { f64::NEG_INFINITY },
        elapsed_ms: 0,
    })
}

/// whisper-server names the language (`italian`); the code (`it`) is shorter.
fn language_code(name: &str) -> String {
    match name {
        "italian" => "it",
        "english" => "en",
        other => other,
    }
    .to_string()
}

/// POST a multipart form with the WAV to `/inference`; the response body.
fn post_inference(addr: SocketAddr, wav: &[u8], fields: &[(&str, &str)]) -> io::Result<Vec<u8>> {
    let boundary = "dualeye-7b3f9c2e";
    let mut body = Vec::with_capacity(wav.len() + 1024);
    for (name, value) in fields {
        write!(body, "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")?;
    }
    write!(body, "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"utterance.wav\"\r\nContent-Type: audio/wav\r\n\r\n")?;
    body.extend_from_slice(wav);
    write!(body, "\r\n--{boundary}--\r\n")?;

    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
    write!(
        stream,
        "POST /inference HTTP/1.1\r\nHost: {addr}\r\nContent-Type: multipart/form-data; boundary={boundary}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    http_body(&response)
}

/// The body of an HTTP/1.1 response with status 200, plain or chunked.
fn http_body(response: &[u8]) -> io::Result<Vec<u8>> {
    let bad = |why: &str| io::Error::new(io::ErrorKind::InvalidData, why.to_string());
    let split = response.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| bad("no HTTP header"))?;
    let head = String::from_utf8_lossy(&response[..split]).to_ascii_lowercase();
    let body = &response[split + 4..];
    let status = head.split_whitespace().nth(1).unwrap_or("");
    if status != "200" {
        return Err(bad(&format!("HTTP {status}: {}", String::from_utf8_lossy(body).trim())));
    }
    if !head.contains("transfer-encoding: chunked") {
        return Ok(body.to_vec());
    }
    let mut out = Vec::new();
    let mut rest = body;
    loop {
        let line_end = rest.windows(2).position(|w| w == b"\r\n").ok_or_else(|| bad("bad chunk"))?;
        let size_str = String::from_utf8_lossy(&rest[..line_end]);
        let size = usize::from_str_radix(size_str.split(';').next().unwrap_or("").trim(), 16).map_err(|_| bad("bad chunk size"))?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if rest.len() < size {
            return Err(bad("short chunk"));
        }
        out.extend_from_slice(&rest[..size]);
        rest = rest.get(size + 2..).unwrap_or(&[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_and_chunked_bodies() {
        assert_eq!(http_body(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").unwrap(), b"{}");
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n{\"a\r\n2\r\n\"}\r\n0\r\n\r\n";
        assert_eq!(http_body(chunked).unwrap(), b"{\"a\"}");
        assert!(http_body(b"HTTP/1.1 500 Internal\r\n\r\noops").is_err());
    }

    #[test]
    fn weights_logprob_by_tokens() {
        let reply = json!({"text": " ciao ", "language": "italian", "segments": [
            {"avg_logprob": -0.2, "tokens": [1, 2, 3]},
            {"avg_logprob": 0.0, "tokens": [4]},
        ]});
        let t = parse_reply(&reply).unwrap();
        assert_eq!(t.text, "ciao");
        assert_eq!(t.language, "it");
        assert!((t.logprob - -0.15).abs() < 1e-9);
    }
}
