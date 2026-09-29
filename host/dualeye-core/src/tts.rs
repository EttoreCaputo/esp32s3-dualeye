//! Text-to-speech with a [Piper](https://github.com/OHF-Voice/piper1-gpl)
//! `http_server` sidecar on `127.0.0.1`.
//!
//! Piper is a Python package (`piper-tts`, GPL-3.0, run as a separate
//! process), installed into a virtualenv in DualEye's data folder with
//! [`install`]. [`Tts`] starts its server in the background with the voices
//! loaded, restarts it if it dies and stops it when dropped
//! ([`crate::sidecar`]). One server speaks every voice: each request names
//! the one for its language. Voices come from [`crate::models`]; Piper's
//! 22 kHz output is resampled to the board's 16 kHz.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;

use crate::flasher::{self, FlashEvent};
use crate::models::{self, Kind, Model};
use crate::sidecar::{self, Process};
use crate::stt::models_dir;
use crate::voice::SAMPLE_RATE;

/// What [`install`] puts in the virtualenv.
pub const PIPER_REQUIREMENT: &str = "piper-tts[http]==1.8.0";
/// Written into the virtualenv once Piper is installed in it.
const READY_MARKER: &str = ".dualeye-ready";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Where Piper's virtualenv lives: `piper/` in DualEye's data folder.
pub fn piper_dir() -> Option<PathBuf> {
    crate::claude::data_dir().map(|d| d.join("piper"))
}

/// The virtualenv's interpreter with Piper in it: `DUALEYE_PIPER_PYTHON`, or
/// the one [`install`] set up.
pub fn find_python() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("DUALEYE_PIPER_PYTHON").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let venv = piper_dir()?.join("venv");
    venv.join(READY_MARKER).is_file().then(|| flasher::venv_python(&venv)).filter(|p| p.is_file())
}

/// Create Piper's virtualenv with `python` (3.9 or later) and install
/// [`PIPER_REQUIREMENT`] into it (about 60 MB, from PyPI), passing on pip's
/// output line by line. Returns the virtualenv's interpreter.
pub fn install(python: &Path, mut on_line: impl FnMut(String)) -> io::Result<PathBuf> {
    let dir = piper_dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))?;
    let venv = dir.join("venv");
    fs::create_dir_all(&dir)?;
    let _ = fs::remove_file(venv.join(READY_MARKER));
    let mut forward = |e: FlashEvent| {
        if let FlashEvent::Log { line } = e {
            on_line(line);
        }
    };
    let mut cmd = flasher::command(python);
    cmd.arg("-m").arg("venv").arg(&venv);
    flasher::run_streaming(cmd, &mut forward).map_err(|e| io::Error::new(e.kind(), format!("creating the virtualenv: {e}")))?;
    let venv_python = flasher::venv_python(&venv);
    let mut cmd = flasher::command(&venv_python);
    cmd.args(["-m", "pip", "install", "--disable-pip-version-check", "--no-input", "--retries", "10", PIPER_REQUIREMENT]);
    flasher::run_streaming(cmd, &mut forward).map_err(|e| io::Error::new(e.kind(), format!("installing Piper: {e}")))?;
    File::create(venv.join(READY_MARKER))?;
    Ok(venv_python)
}

/// A Python 3 on the PATH to create the virtualenv with (not macOS's
/// `/usr/bin/python3` stub, which offers to install the Xcode tools).
pub fn system_python() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let names: &[&str] = if cfg!(windows) { &["python.exe", "python3.exe"] } else { &["python3"] };
    std::env::split_paths(&path)
        .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from))
        .filter(|dir| !(cfg!(target_os = "macos") && dir == Path::new("/usr/bin")))
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|p| p.is_file())
}

#[derive(Debug, Clone, PartialEq)]
pub struct TtsConfig {
    /// The virtualenv's interpreter, run as `python -m piper.http_server`.
    pub python: PathBuf,
    /// Voice by language (`it`, `en`): ids from [`crate::models`], downloaded.
    pub voices: BTreeMap<String, String>,
}

impl TtsConfig {
    /// The default voice of each language, where it's downloaded.
    pub fn with_default_voices(python: PathBuf) -> Self {
        let voices = ["it", "en"]
            .into_iter()
            .filter_map(|lang| models::default_voice(lang).map(|v| (lang.to_string(), v.to_string())))
            .filter(|(_, v)| Model::by_id(v).is_some_and(Model::is_installed))
            .collect();
        Self { python, voices }
    }
}

#[derive(Debug)]
pub struct TtsError(String);

impl fmt::Display for TtsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "piper: {}", self.0)
    }
}

impl std::error::Error for TtsError {}

impl From<io::Error> for TtsError {
    fn from(e: io::Error) -> Self {
        TtsError(e.to_string())
    }
}

/// The sidecar. Shared between connections: it outlives a board reconnect.
pub struct Tts {
    config: TtsConfig,
    server: Mutex<Option<Process>>,
}

impl fmt::Debug for Tts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tts").field("config", &self.config).finish_non_exhaustive()
    }
}

impl Tts {
    pub fn new(config: TtsConfig) -> Self {
        Self { config, server: Mutex::new(None) }
    }

    pub fn config(&self) -> &TtsConfig {
        &self.config
    }

    /// Stop the server now, even while others still hold this [`Tts`] (a
    /// reply being spoken); the next request would start it again.
    pub fn shutdown(&self) {
        self.server.lock().unwrap().take();
    }

    /// The voice `language` speaks with: its own, or else any there is.
    pub fn voice_for(&self, language: &str) -> Option<&str> {
        self.config.voices.get(language).or_else(|| self.config.voices.values().next()).map(String::as_str)
    }

    /// Start the server and load every voice (Piper loads one on its first
    /// request, which takes most of a second).
    pub fn warm_up(&self) -> Result<(), TtsError> {
        for voice in self.config.voices.values() {
            self.request("Ok.", voice)?;
        }
        Ok(())
    }

    /// Speak `text` in `language`: 16 kHz mono samples.
    pub fn synthesize(&self, text: &str, language: &str) -> Result<Vec<i16>, TtsError> {
        // Piper fails on text it makes no sound for.
        if !text.chars().any(char::is_alphanumeric) {
            return Ok(Vec::new());
        }
        let voice = self.voice_for(language).ok_or_else(|| TtsError("no voice downloaded".into()))?.to_string();
        let wav = self.request(text, &voice)?;
        let (rate, samples) = parse_wav(&wav).ok_or_else(|| TtsError("not a 16-bit mono WAV".into()))?;
        Ok(resample(&samples, rate, SAMPLE_RATE))
    }

    fn request(&self, text: &str, voice: &str) -> Result<Vec<u8>, TtsError> {
        let body = json!({"text": text, "voice": voice}).to_string();
        // A server that died since the last request gets one restart.
        for attempt in 0..2 {
            let addr = self.addr()?;
            match sidecar::post(addr, "/synthesize", "application/json", body.as_bytes(), REQUEST_TIMEOUT) {
                Ok(wav) => return Ok(wav),
                Err(e) if attempt == 0 && matches!(e.kind(), io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset) => {
                    self.server.lock().unwrap().take();
                }
                Err(e) => return Err(e.into()),
            }
        }
        unreachable!()
    }

    fn addr(&self) -> Result<SocketAddr, TtsError> {
        let mut server = self.server.lock().unwrap();
        if let Some(s) = server.as_mut() {
            if s.is_running() {
                return Ok(s.addr);
            }
            *server = None;
        }
        let s = self.start()?;
        let addr = s.addr;
        *server = Some(s);
        Ok(addr)
    }

    fn start(&self) -> Result<Process, TtsError> {
        let dir = models_dir().ok_or_else(|| TtsError("no data folder for the voices".into()))?;
        let first = self.config.voices.values().next().ok_or_else(|| TtsError("no voice downloaded".into()))?;
        for voice in self.config.voices.values() {
            if !Model::by_id(voice).is_some_and(|m| m.kind == Kind::Voice && m.is_installed()) {
                return Err(TtsError(format!("the voice {voice} isn't downloaded")));
            }
        }
        let mut cmd = flasher::command(&self.config.python);
        cmd.args(["-m", "piper.http_server", "-m", first]).arg("--data-dir").arg(&dir);
        let piper = piper_dir();
        let (pid_file, log) = (piper.as_ref().map(|d| d.join("server.pid")), piper.as_ref().map(|d| d.join("server.log")));
        Process::start(cmd, "piper.http_server", pid_file, log, STARTUP_TIMEOUT).map_err(TtsError)
    }
}

/// Rate and samples of a 16-bit mono PCM WAV.
pub fn parse_wav(wav: &[u8]) -> Option<(u32, Vec<i16>)> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let (mut rate, mut pos) = (None, 12);
    while pos + 8 <= wav.len() {
        let id = &wav[pos..pos + 4];
        let len = u32::from_le_bytes(wav[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = wav.get(pos + 8..(pos + 8 + len).min(wav.len()))?;
        match id {
            b"fmt " if body.len() >= 16 => {
                let format = u16::from_le_bytes([body[0], body[1]]);
                let channels = u16::from_le_bytes([body[2], body[3]]);
                let bits = u16::from_le_bytes([body[14], body[15]]);
                if format != 1 || channels != 1 || bits != 16 {
                    return None;
                }
                rate = Some(u32::from_le_bytes(body[4..8].try_into().ok()?));
            }
            b"data" => return Some((rate?, body.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect())),
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    None
}

/// Band-limited resampling: a windowed sinc (Blackman, 16 zero crossings
/// each side), cut off just below the lower Nyquist frequency.
pub fn resample(input: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    const ZEROS: f64 = 16.0;
    let ratio = to as f64 / from as f64;
    // Cutoff as a fraction of the input's Nyquist frequency.
    let cutoff = ratio.min(1.0) * 0.95;
    let half = ZEROS / cutoff;
    let out_len = (input.len() as f64 * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for n in 0..out_len {
        let t = n as f64 / ratio;
        let (lo, hi) = (((t - half).ceil() as i64).max(0), ((t + half).floor() as i64).min(input.len() as i64 - 1));
        let (mut acc, mut norm) = (0.0, 0.0);
        for k in lo..=hi {
            let x = k as f64 - t;
            let sinc = if x == 0.0 { 1.0 } else { (std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * cutoff * x) };
            let w = 0.42 + 0.5 * (std::f64::consts::PI * x / half).cos() + 0.08 * (2.0 * std::f64::consts::PI * x / half).cos();
            let h = sinc * w;
            acc += input[k as usize] as f64 * h;
            norm += h;
        }
        out.push((acc / norm).round().clamp(-32768.0, 32767.0) as i16);
    }
    out
}

/// `text` in sentences, so the first can be spoken while the next is
/// synthesized. Very short pieces stay with the one before.
pub fn sentences(text: &str) -> Vec<String> {
    const MIN_CHARS: usize = 12;
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        let ends = matches!(c, '.' | '!' | '?' | ';' | '\n') && chars.peek().is_none_or(|n| n.is_whitespace());
        if ends {
            push_sentence(&mut out, &mut current, MIN_CHARS);
        }
    }
    push_sentence(&mut out, &mut current, 0);
    out
}

fn push_sentence(out: &mut Vec<String>, current: &mut String, min: usize) {
    let s = current.trim();
    if s.is_empty() {
        current.clear();
        return;
    }
    match out.last_mut() {
        Some(last) if last.chars().count() < min => {
            last.push(' ');
            last.push_str(s);
        }
        _ => out.push(s.to_string()),
    }
    current.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::wav;

    #[test]
    fn parses_what_wav_writes() {
        assert_eq!(parse_wav(&wav(&[1, -2, 3], 22_050)), Some((22_050, vec![1, -2, 3])));
        assert_eq!(parse_wav(b"RIFF\0\0\0\0WAVE"), None);
    }

    #[test]
    fn resampling_keeps_a_tone_and_its_level() {
        let tone: Vec<i16> = (0..22_050).map(|i| (10_000.0 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 22_050.0).sin()) as i16).collect();
        let out = resample(&tone, 22_050, 16_000);
        assert_eq!(out.len(), 16_000);
        let want: Vec<f64> = (0..16_000).map(|i| 10_000.0 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 16_000.0).sin()).collect();
        // Away from the edges, within a small fraction of the amplitude.
        let worst = out[200..15_800].iter().zip(&want[200..15_800]).map(|(a, b)| (*a as f64 - b).abs()).fold(0.0, f64::max);
        assert!(worst < 150.0, "error {worst}");
    }

    #[test]
    fn resampling_removes_what_16k_cannot_carry() {
        // 10 kHz is above 16 kHz's Nyquist frequency: it must not alias down.
        let tone: Vec<i16> = (0..22_050).map(|i| (10_000.0 * (2.0 * std::f64::consts::PI * 10_000.0 * i as f64 / 22_050.0).sin()) as i16).collect();
        let out = resample(&tone, 22_050, 16_000);
        let rms = (out[200..15_800].iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / 15_600.0).sqrt();
        assert!(rms < 300.0, "rms {rms}");
    }

    #[test]
    fn splits_sentences() {
        assert_eq!(sentences("Fatto. La faccia rings è a sinistra! E poi?"), ["Fatto. La faccia rings è a sinistra!", "E poi?"]);
        assert_eq!(sentences("It's 3.5 degrees warmer. Done"), ["It's 3.5 degrees warmer.", "Done"]);
        assert!(sentences("  ").is_empty());
    }
}
