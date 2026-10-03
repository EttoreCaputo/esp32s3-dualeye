//! Text-to-speech with sidecars on `127.0.0.1`: a
//! [Piper](https://github.com/OHF-Voice/piper1-gpl) `http_server`, and a
//! small server of ours around [kokoro-onnx](https://github.com/thewh1teagle/kokoro-onnx)
//! (`kokoro_server.py`) for Kokoro's voices.
//!
//! Each engine is a Python package (`piper-tts`, GPL-3.0; `kokoro-onnx`,
//! MIT), run as a separate process and installed into a virtualenv of its
//! own in DualEye's data folder with [`install`]. [`Tts`] starts the servers
//! its voices need in the background, restarts one that dies and stops them
//! when dropped ([`crate::sidecar`]). One server speaks every voice of its
//! engine: each request names the one for its language. Voices come from
//! [`crate::models`]; Piper's 22 kHz and Kokoro's 24 kHz are resampled to
//! the board's 16 kHz.
//!
//! A voice can also be a provider's ([`crate::cloud`], `groq:hannah`): no
//! server here, just a request per sentence (split further where the
//! provider takes less text at once).

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;

pub use crate::models::Engine;
use crate::cloud::{self, CloudError};
use crate::flasher::{self, FlashEvent};
use crate::models::{self, Kind, Model};
use crate::sidecar::{self, Process};
use crate::stt::models_dir;
use crate::voice::SAMPLE_RATE;

/// What [`install`] puts in Piper's virtualenv.
pub const PIPER_REQUIREMENT: &str = "piper-tts[http]==1.8.0";
/// And in Kokoro's: it wants Python 3.10 to 3.13.
pub const KOKORO_REQUIREMENT: &str = "kokoro-onnx==0.6.1";
/// Kokoro's server, written next to its virtualenv each time it starts.
const KOKORO_SERVER: &str = include_str!("kokoro_server.py");
const KOKORO_SERVER_FILE: &str = "dualeye_kokoro.py";
/// Written into the virtualenv once the engine is installed in it.
const READY_MARKER: &str = ".dualeye-ready";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

impl Engine {
    pub const ALL: [Engine; 2] = [Engine::Piper, Engine::Kokoro];

    pub fn name(self) -> &'static str {
        match self {
            Engine::Piper => "Piper",
            Engine::Kokoro => "Kokoro",
        }
    }

    /// What it's called in DualEye's data folder, commands and errors.
    pub fn key(self) -> &'static str {
        match self {
            Engine::Piper => "piper",
            Engine::Kokoro => "kokoro",
        }
    }

    pub fn requirement(self) -> &'static str {
        match self {
            Engine::Piper => PIPER_REQUIREMENT,
            Engine::Kokoro => KOKORO_REQUIREMENT,
        }
    }

    /// The Python versions it installs into, as (major, minor) bounds.
    fn pythons(self) -> std::ops::RangeInclusive<u32> {
        match self {
            Engine::Piper => 9..=99,
            Engine::Kokoro => 10..=13,
        }
    }

    /// Where its virtualenv lives: `piper/` or `kokoro/` in DualEye's data folder.
    pub fn dir(self) -> Option<PathBuf> {
        crate::claude::data_dir().map(|d| d.join(self.key()))
    }

    /// The virtualenv's interpreter: `DUALEYE_PIPER_PYTHON` (or
    /// `DUALEYE_KOKORO_PYTHON`), or the one [`install`] set up.
    pub fn python(self) -> Option<PathBuf> {
        if let Some(p) = std::env::var_os(format!("DUALEYE_{}_PYTHON", self.key().to_uppercase())).map(PathBuf::from) {
            return p.is_file().then_some(p);
        }
        let venv = self.dir()?.join("venv");
        venv.join(READY_MARKER).is_file().then(|| flasher::venv_python(&venv)).filter(|p| p.is_file())
    }
}

/// Piper's interpreter ([`Engine::python`]).
pub fn find_python() -> Option<PathBuf> {
    Engine::Piper.python()
}

/// Create `engine`'s virtualenv with `python` and install its
/// [`Engine::requirement`] into it (about 60 MB for Piper, 150 MB for
/// Kokoro, from PyPI), passing on pip's output line by line. Returns the
/// virtualenv's interpreter.
pub fn install(engine: Engine, python: &Path, mut on_line: impl FnMut(String)) -> io::Result<PathBuf> {
    let dir = engine.dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))?;
    let venv = dir.join("venv");
    fs::create_dir_all(&dir)?;
    let _ = fs::remove_file(venv.join(READY_MARKER));
    let mut forward = |e: FlashEvent| {
        if let FlashEvent::Log { line } = e {
            on_line(line);
        }
    };
    let mut cmd = flasher::command(python);
    cmd.arg("-m").arg("venv").arg("--clear").arg(&venv);
    flasher::run_streaming(cmd, &mut forward).map_err(|e| io::Error::new(e.kind(), format!("creating the virtualenv: {e}")))?;
    let venv_python = flasher::venv_python(&venv);
    let mut cmd = flasher::command(&venv_python);
    cmd.args(["-m", "pip", "install", "--disable-pip-version-check", "--no-input", "--retries", "10", engine.requirement()]);
    flasher::run_streaming(cmd, &mut forward).map_err(|e| io::Error::new(e.kind(), format!("installing {}: {e}", engine.name())))?;
    File::create(venv.join(READY_MARKER))?;
    Ok(venv_python)
}

/// A Python 3 on the PATH that `engine` installs into, to create its
/// virtualenv with (not macOS's `/usr/bin/python3` stub, which offers to
/// install the Xcode tools): `python3`, else `python3.13` down to `3.9`.
pub fn system_python(engine: Engine) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let exe = if cfg!(windows) { ".exe" } else { "" };
    let mut names: Vec<String> = if cfg!(windows) { vec!["python.exe".into(), "python3.exe".into()] } else { vec!["python3".into()] };
    names.extend((9..=13).rev().map(|minor| format!("python3.{minor}{exe}")));
    let dirs: Vec<PathBuf> = std::env::split_paths(&path)
        .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from))
        .filter(|dir| !(cfg!(target_os = "macos") && dir == Path::new("/usr/bin")))
        .collect();
    names.iter().flat_map(|n| dirs.iter().map(move |d| d.join(n))).find(|p| p.is_file() && python_minor(p).is_some_and(|m| engine.pythons().contains(&m)))
}

/// The minor version of a Python 3.
fn python_minor(python: &Path) -> Option<u32> {
    let out = flasher::command(python).args(["-c", "import sys; print(sys.version_info[0], sys.version_info[1])"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text.split_whitespace();
    (parts.next()? == "3").then_some(())?;
    parts.next()?.parse().ok()
}

#[derive(Debug, Clone, PartialEq)]
pub struct TtsConfig {
    /// Each engine's virtualenv interpreter: those of `voices` must be here.
    pub pythons: BTreeMap<Engine, PathBuf>,
    /// Voice by language (`it`, `en`): ids from [`crate::models`],
    /// downloaded, or of [`crate::cloud`] models.
    pub voices: BTreeMap<String, String>,
    /// The cloud voices among `voices`, by id, with their provider's key.
    pub clients: BTreeMap<String, cloud::Client>,
}

impl TtsConfig {
    /// The default voice of each language, where it's downloaded, with the
    /// engines that are installed.
    pub fn with_default_voices() -> Self {
        let voices = ["it", "en"]
            .into_iter()
            .filter_map(|lang| models::default_voice(lang).map(|v| (lang.to_string(), v.to_string())))
            .filter(|(_, v)| Model::by_id(v).is_some_and(Model::is_installed))
            .collect();
        Self { pythons: installed_engines(), voices, clients: BTreeMap::new() }
    }

    /// `voices`, local or cloud, with the engines in `pythons`. Fails on a
    /// cloud voice whose provider has no key.
    pub fn new(pythons: BTreeMap<Engine, PathBuf>, voices: BTreeMap<String, String>) -> Result<Self, String> {
        let clients = voices
            .values()
            .filter(|v| cloud::is_cloud_id(v))
            .map(|v| cloud::Client::new(v, Kind::Voice).map(|c| (v.clone(), c)))
            .collect::<Result<_, _>>()?;
        Ok(Self { pythons, voices, clients })
    }

    /// The engines its voices need.
    pub fn engines(&self) -> impl Iterator<Item = Engine> + '_ {
        Engine::ALL.into_iter().filter(|e| self.voices.values().any(|v| Model::by_id(v).and_then(Model::engine) == Some(*e)))
    }
}

/// The interpreter of every engine that's installed.
pub fn installed_engines() -> BTreeMap<Engine, PathBuf> {
    Engine::ALL.into_iter().filter_map(|e| e.python().map(|p| (e, p))).collect()
}

/// Says which engine failed: `piper: ...` or `kokoro: ...`.
#[derive(Debug)]
pub struct TtsError(String);

impl TtsError {
    fn new(engine: Engine, why: impl fmt::Display) -> Self {
        TtsError(format!("{}: {why}", engine.key()))
    }

    fn cloud(e: CloudError) -> Self {
        TtsError(e.to_string())
    }
}

impl fmt::Display for TtsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TtsError {}

/// The sidecars. Shared between connections: they outlive a board reconnect.
pub struct Tts {
    config: TtsConfig,
    servers: Mutex<BTreeMap<Engine, Process>>,
}

impl fmt::Debug for Tts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tts").field("config", &self.config).finish_non_exhaustive()
    }
}

impl Tts {
    pub fn new(config: TtsConfig) -> Self {
        Self { config, servers: Mutex::new(BTreeMap::new()) }
    }

    pub fn config(&self) -> &TtsConfig {
        &self.config
    }

    /// Stop the servers now, even while others still hold this [`Tts`] (a
    /// reply being spoken); the next request would start them again.
    pub fn shutdown(&self) {
        self.servers.lock().unwrap().clear();
    }

    /// The voice `language` speaks with: its own, or else any there is.
    pub fn voice_for(&self, language: &str) -> Option<&str> {
        self.config.voices.get(language).or_else(|| self.config.voices.values().next()).map(String::as_str)
    }

    /// Start the servers and load every voice (Piper loads one on its first
    /// request, which takes most of a second). A cloud voice's key is only
    /// checked: each sentence spoken counts against a free plan.
    pub fn warm_up(&self) -> Result<(), TtsError> {
        for voice in self.config.voices.values() {
            match self.config.clients.get(voice) {
                Some(client) => client.check().map_err(TtsError::cloud)?,
                None => drop(self.request("Ok.", voice)?),
            }
        }
        Ok(())
    }

    /// Speak `text` in `language`: 16 kHz mono samples.
    pub fn synthesize(&self, text: &str, language: &str) -> Result<Vec<i16>, TtsError> {
        // Piper fails on text it makes no sound for.
        if !text.chars().any(char::is_alphanumeric) {
            return Ok(Vec::new());
        }
        let voice = self.voice_for(language).ok_or_else(|| TtsError::new(Engine::Piper, "no voice downloaded"))?.to_string();
        if let Some(client) = self.config.clients.get(&voice) {
            let mut out = Vec::new();
            for piece in cloud::chunks(text, client.model.model.max_chars) {
                let wav = client.speak(&piece).map_err(TtsError::cloud)?;
                let (rate, samples) = parse_wav(&wav).ok_or_else(|| TtsError(format!("{}: not a 16-bit mono WAV", client.model.provider.name)))?;
                out.extend(resample(&samples, rate, SAMPLE_RATE));
            }
            return Ok(out);
        }
        let wav = self.request(text, &voice)?;
        let engine = Model::by_id(&voice).and_then(Model::engine).unwrap_or(Engine::Piper);
        let (rate, samples) = parse_wav(&wav).ok_or_else(|| TtsError::new(engine, "not a 16-bit mono WAV"))?;
        Ok(resample(&samples, rate, SAMPLE_RATE))
    }

    fn request(&self, text: &str, voice: &str) -> Result<Vec<u8>, TtsError> {
        let model = Model::by_id(voice).filter(|m| m.kind == Kind::Voice);
        let engine = model.and_then(Model::engine).unwrap_or(Engine::Piper);
        let model = model.ok_or_else(|| TtsError::new(engine, format!("{voice} isn't a voice")))?;
        let body = json!({"text": text, "voice": model.voice_name()}).to_string();
        // A server that died since the last request gets one restart.
        for attempt in 0..2 {
            let addr = self.addr(engine)?;
            match sidecar::post(addr, "/synthesize", "application/json", body.as_bytes(), REQUEST_TIMEOUT) {
                Ok(wav) => return Ok(wav),
                Err(e) if attempt == 0 && matches!(e.kind(), io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset) => {
                    self.servers.lock().unwrap().remove(&engine);
                }
                Err(e) => return Err(TtsError::new(engine, e)),
            }
        }
        unreachable!()
    }

    fn addr(&self, engine: Engine) -> Result<SocketAddr, TtsError> {
        let mut servers = self.servers.lock().unwrap();
        if let Some(s) = servers.get_mut(&engine) {
            if s.is_running() {
                return Ok(s.addr);
            }
            servers.remove(&engine);
        }
        let s = self.start(engine).map_err(|why| TtsError::new(engine, why))?;
        let addr = s.addr;
        servers.insert(engine, s);
        Ok(addr)
    }

    fn start(&self, engine: Engine) -> Result<Process, String> {
        let dir = models_dir().ok_or("no data folder for the voices")?;
        let voices: Vec<&Model> = self.config.voices.values().filter_map(|v| Model::by_id(v)).filter(|m| m.engine() == Some(engine)).collect();
        let first = voices.first().ok_or("no voice downloaded")?;
        if let Some(missing) = voices.iter().find(|m| !m.is_installed()) {
            return Err(format!("the voice {} isn't downloaded", missing.id));
        }
        let python = self.config.pythons.get(&engine).ok_or_else(|| format!("{} isn't installed yet", engine.name()))?;
        let home = engine.dir().ok_or("no data folder")?;
        let mut cmd = flasher::command(python);
        let marker = match engine {
            Engine::Piper => {
                cmd.args(["-m", "piper.http_server", "-m", first.id]).arg("--data-dir").arg(&dir);
                "piper.http_server"
            }
            Engine::Kokoro => {
                let script = home.join(KOKORO_SERVER_FILE);
                fs::create_dir_all(&home).and_then(|()| fs::write(&script, KOKORO_SERVER)).map_err(|e| e.to_string())?;
                cmd.arg(&script).arg("--model").arg(dir.join(first.files[0].name)).arg("--voices").arg(dir.join(first.files[1].name));
                KOKORO_SERVER_FILE
            }
        };
        Process::start(cmd, marker, Some(home.join("server.pid")), Some(home.join("server.log")), STARTUP_TIMEOUT)
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
