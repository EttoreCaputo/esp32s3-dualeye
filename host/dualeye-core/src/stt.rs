//! Speech-to-text with a [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
//! `whisper-server` sidecar on `127.0.0.1`.
//!
//! [`Stt`] starts the server in the background with the model loaded, so the
//! first utterance doesn't wait for it, restarts it if it dies, and stops it
//! when dropped ([`crate::sidecar`]).
//!
//! With [`SttLanguage::Auto`] Whisper detects the language (10 out of 10
//! short commands right from the board's mic, 0.67–0.997); when it picks
//! neither Italian nor English, the utterance is decoded again in whichever
//! of the two it found more likely. Comparing the log probability of an
//! Italian and an English decoding doesn't work: Whisper forced into the
//! wrong language is often just as sure of itself.

use std::fmt;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::sidecar::{self, Process};
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

/// `whisper-server`: `DUALEYE_WHISPER_SERVER`, the one the app ships, or
/// one on the PATH (see [`crate::sidecar::find_program`]).
pub fn find_server() -> Option<PathBuf> {
    crate::sidecar::find_program("whisper-server", "DUALEYE_WHISPER_SERVER")
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

/// The sidecar. Shared between connections: it outlives a board reconnect.
pub struct Stt {
    config: SttConfig,
    server: Mutex<Option<Process>>,
}

impl Stt {
    pub fn new(config: SttConfig) -> Self {
        Self { config, server: Mutex::new(None) }
    }

    pub fn config(&self) -> &SttConfig {
        &self.config
    }

    /// Stop the server now, even while others still hold this [`Stt`] (an
    /// utterance being transcribed); the next request would start it again.
    pub fn shutdown(&self) {
        self.server.lock().unwrap().take();
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
        let (content_type, body) = multipart(wav, &fields)?;
        // A server that died since the last request gets one restart.
        for attempt in 0..2 {
            let addr = self.addr()?;
            match sidecar::post(addr, "/inference", &content_type, &body, REQUEST_TIMEOUT) {
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

impl fmt::Debug for Stt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stt").field("config", &self.config).finish_non_exhaustive()
    }
}

fn start(config: &SttConfig) -> Result<Process, SttError> {
    if !config.model.is_file() {
        return Err(SttError::Server(format!("no model at {}", config.model.display())));
    }
    let mut cmd = Command::new(&config.server);
    cmd.arg("--model").arg(&config.model).args(["--language", "auto"]);
    // Its log goes next to the models, for when it won't start. The pid file
    // lets the next host stop one a killed host left running.
    let dir = models_dir();
    let (pid_file, log) = (dir.as_ref().map(|d| d.join("whisper-server.pid")), dir.as_ref().map(|d| d.join("whisper-server.log")));
    Process::start(cmd, "whisper-server", pid_file, log, STARTUP_TIMEOUT).map_err(SttError::Server)
}

/// A multipart form with the fields and the WAV as `file`.
fn multipart(wav: &[u8], fields: &[(&str, &str)]) -> io::Result<(String, Vec<u8>)> {
    let boundary = "dualeye-7b3f9c2e";
    let mut body = Vec::with_capacity(wav.len() + 1024);
    for (name, value) in fields {
        write!(body, "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")?;
    }
    write!(body, "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"utterance.wav\"\r\nContent-Type: audio/wav\r\n\r\n")?;
    body.extend_from_slice(wav);
    write!(body, "\r\n--{boundary}--\r\n")?;
    Ok((format!("multipart/form-data; boundary={boundary}"), body))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
