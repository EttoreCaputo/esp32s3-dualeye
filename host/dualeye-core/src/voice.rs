//! The host's half of voice: what the board says after its wake word.
//!
//! The board streams an utterance on `audio_up` between an `utterance_start`
//! and an `utterance_end` notification (see `docs/protocol.md`). [`Capture`]
//! puts the frames back together, filling any it lost with silence so the
//! timing stays right, and the bridge's voice thread takes each finished
//! [`Utterance`]: it keeps a WAV copy when asked, transcribes it with
//! [`Stt`] when there's speech and puts the eyes back to idle.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use chrono::Local;
use serde::Serialize;
use serde_json::{Value, json};

use crate::bridge::{BridgeEvent, EventSink};
use crate::link::Link;
use crate::stt::Stt;

/// The board's audio: 16 kHz, mono, s16le.
pub const SAMPLE_RATE: u32 = 16_000;
/// Bytes before the PCM in each `audio_up` frame: utterance id, flags, sequence (u16 LE).
const HEADER: usize = 4;
/// A gap longer than this is a broken stream, not a few lost frames.
const MAX_GAP_FRAMES: u16 = 64;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Utterance {
    /// The board's id for it; wraps at 256.
    pub id: u8,
    /// What started it: `wake` (the wake word) or `host` (`voice/listen`).
    pub trigger: String,
    /// Why it ended: `end_of_speech`, `no_speech`, `max_length`, `host` or `muted`.
    pub reason: String,
    /// The board's VAD heard speech in it.
    pub speech: bool,
    /// Frames that never arrived (the board couldn't send them, or they came
    /// in corrupt), each replaced by silence.
    pub lost_frames: u32,
    #[serde(skip)]
    pub samples: Vec<i16>,
}

impl Utterance {
    pub fn duration_ms(&self) -> u64 {
        self.samples.len() as u64 * 1000 / SAMPLE_RATE as u64
    }

    /// Peak level in dBFS; `None` for silence.
    pub fn peak_db(&self) -> Option<f64> {
        let peak = self.samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        (peak > 0).then(|| 20.0 * (peak as f64 / 32768.0).log10())
    }

    /// The samples as a 16 kHz mono 16-bit WAV file.
    pub fn to_wav(&self) -> Vec<u8> {
        wav(&self.samples, SAMPLE_RATE)
    }
}

/// A PCM WAV file: RIFF header, then the samples little-endian.
pub fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

struct Building {
    id: u8,
    trigger: String,
    next_seq: u16,
    frame_samples: usize,
    lost: u32,
    samples: Vec<i16>,
}

/// Puts an utterance back together from the board's notifications and frames.
#[derive(Default)]
pub struct Capture {
    current: Option<Building>,
}

impl Capture {
    /// `utterance_start`. One still open is dropped: its end got lost.
    pub fn start(&mut self, params: &Value) {
        let Some(id) = params["id"].as_u64() else {
            return;
        };
        self.current = Some(Building {
            id: id as u8,
            trigger: params["trigger"].as_str().unwrap_or("wake").to_string(),
            next_seq: 0,
            frame_samples: 0,
            lost: 0,
            samples: Vec::with_capacity(SAMPLE_RATE as usize * 4),
        });
    }

    /// One `audio_up` frame. Frames of another utterance are ignored.
    pub fn audio(&mut self, payload: &[u8]) {
        let Some(b) = self.current.as_mut() else {
            return;
        };
        if payload.len() < HEADER || payload[0] != b.id || !(payload.len() - HEADER).is_multiple_of(2) {
            return;
        }
        let seq = u16::from_le_bytes([payload[2], payload[3]]);
        let pcm = &payload[HEADER..];
        let gap = seq.wrapping_sub(b.next_seq);
        if gap >= MAX_GAP_FRAMES {
            // A repeat or something from long ago.
            return;
        }
        let frame_samples = pcm.len() / 2;
        b.samples.resize(b.samples.len() + gap as usize * frame_samples.max(b.frame_samples), 0);
        b.lost += gap as u32;
        b.frame_samples = frame_samples;
        b.samples.extend(pcm.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)));
        b.next_seq = seq.wrapping_add(1);
    }

    /// `utterance_end`: the finished utterance, if it's the one being built.
    pub fn end(&mut self, params: &Value) -> Option<Utterance> {
        let id = params["id"].as_u64()? as u8;
        if self.current.as_ref()?.id != id {
            return None;
        }
        let mut b = self.current.take()?;
        // Frames lost at the end, which no later frame revealed.
        if let Some(frames) = params["frames"].as_u64() {
            let missing = (frames as u16).wrapping_sub(b.next_seq);
            if missing < MAX_GAP_FRAMES {
                b.samples.resize(b.samples.len() + missing as usize * b.frame_samples, 0);
                b.lost += missing as u32;
            }
        }
        Some(Utterance {
            id,
            trigger: b.trigger,
            reason: params["reason"].as_str().unwrap_or("unknown").to_string(),
            speech: params["speech"].as_bool().unwrap_or(true),
            lost_frames: b.lost,
            samples: b.samples,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct VoiceConfig {
    /// Keep each utterance as a WAV file here (a debugging aid; nothing is
    /// written without it).
    pub dump_dir: Option<PathBuf>,
    /// Transcribe what the board hears. Shared: the sidecar outlives a
    /// reconnect.
    pub stt: Option<Arc<Stt>>,
}

/// Whisper's output for silence or noise rather than words.
fn is_non_speech(text: &str) -> bool {
    let t = text.trim();
    t.is_empty() || (t.starts_with('[') && t.ends_with(']')) || (t.starts_with('(') && t.ends_with(')'))
}

/// Where WAV dumps go by default: `voice/` in DualEye's data folder.
pub fn default_dump_dir() -> Option<PathBuf> {
    crate::claude::data_dir().map(|d| d.join("voice"))
}

const STATE_TIMEOUT: Duration = Duration::from_secs(1);

/// The pipeline: handles each finished utterance, on its own thread so the
/// link's reader never waits on it, until the sender is dropped. `link` is
/// weak because the link's reader holds that sender.
pub(crate) fn run(config: &Mutex<VoiceConfig>, link: &Weak<Link>, sink: &EventSink, rx: Receiver<Utterance>) {
    for utterance in rx {
        let config = config.lock().unwrap().clone();
        let wav = config.dump_dir.as_deref().and_then(|dir| match dump(dir, &utterance) {
            Ok(path) => Some(path),
            Err(e) => {
                sink(BridgeEvent::BoardLog { line: format!("host: could not save the utterance: {e}") });
                None
            }
        });
        sink(BridgeEvent::Utterance {
            duration_ms: utterance.duration_ms(),
            peak_db: utterance.peak_db(),
            wav: wav.map(|p| p.display().to_string()),
            utterance: utterance.clone(),
        });
        if !utterance.speech {
            continue;
        }
        // The board shows thinking meanwhile.
        if let Some(stt) = &config.stt {
            match stt.transcribe(&utterance.samples) {
                Ok(t) if is_non_speech(&t.text) => sink(BridgeEvent::Transcript { id: utterance.id, transcript: None }),
                Ok(t) => sink(BridgeEvent::Transcript { id: utterance.id, transcript: Some(t) }),
                Err(e) => sink(BridgeEvent::VoiceError { message: e.to_string() }),
            }
        }
        // Nothing acts on the words yet (M5, M6).
        if let Some(link) = link.upgrade() {
            let _ = link.call("voice/state", json!({"state": "idle"}), STATE_TIMEOUT);
        }
    }
}

fn dump(dir: &Path, utterance: &Utterance) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}-{:03}.wav", Local::now().format("%Y%m%d-%H%M%S"), utterance.id));
    fs::write(&path, utterance.to_wav())?;
    Ok(path)
}

/// Keeps the capture and the pipeline's sender together for the link's reader.
pub struct Receiving {
    capture: Capture,
    pipeline: Sender<Utterance>,
}

impl Receiving {
    pub fn new(pipeline: Sender<Utterance>) -> Self {
        Self { capture: Capture::default(), pipeline }
    }

    pub fn start(&mut self, params: &Value) {
        self.capture.start(params);
    }

    pub fn audio(&mut self, payload: &[u8]) {
        self.capture.audio(payload);
    }

    pub fn end(&mut self, params: &Value) {
        if let Some(u) = self.capture.end(params) {
            let _ = self.pipeline.send(u);
        }
    }
}

/// For tests and tools: an `audio_up` frame as the board sends it.
pub fn audio_frame(id: u8, seq: u16, samples: &[i16]) -> Vec<u8> {
    let mut out = vec![id, 0];
    out.extend_from_slice(&seq.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reassembles_and_fills_gaps() {
        let mut c = Capture::default();
        c.start(&json!({"id": 3, "trigger": "wake"}));
        c.audio(&audio_frame(3, 0, &[1, 2]));
        // Frame 1 lost.
        c.audio(&audio_frame(3, 2, &[5, 6]));
        // Another utterance's frame and a repeat are ignored.
        c.audio(&audio_frame(4, 3, &[9, 9]));
        c.audio(&audio_frame(3, 2, &[5, 6]));
        let u = c.end(&json!({"id": 3, "reason": "end_of_speech", "speech": true, "frames": 4})).unwrap();
        assert_eq!(u.samples, [1, 2, 0, 0, 5, 6, 0, 0]);
        assert_eq!(u.lost_frames, 2);
        assert_eq!(u.reason, "end_of_speech");
        assert!(c.end(&json!({"id": 3})).is_none());
    }

    #[test]
    fn end_of_another_utterance_is_ignored() {
        let mut c = Capture::default();
        c.start(&json!({"id": 1}));
        assert!(c.end(&json!({"id": 2})).is_none());
        assert!(c.end(&json!({"id": 1, "frames": 0})).is_some());
    }

    #[test]
    fn wav_header() {
        let w = wav(&[0, -1], 16_000);
        assert_eq!(w.len(), 48);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 4);
        assert_eq!(&w[44..], [0, 0, 0xff, 0xff]);
    }
}
