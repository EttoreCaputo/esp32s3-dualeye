//! The host's half of voice: what the board says after its wake word.
//!
//! The board streams an utterance on `audio_up` between an `utterance_start`
//! and an `utterance_end` notification (see `docs/protocol.md`). [`Capture`]
//! puts the frames back together, filling any it lost with silence so the
//! timing stays right, and the bridge's voice thread takes each finished
//! [`Utterance`]: it keeps a WAV copy when asked, transcribes it with
//! [`Stt`] when there's speech, acts on the words with the local language
//! model ([`Agent`]), or the rules of [`intents`] without one or when it
//! fails, and answers with [`Tts`] through the board's speaker
//! ([`Speaker`]), or puts the eyes back to idle. When something fails, or
//! Whisper heard no words, the board shows `error` (a red ring and a short
//! sound). After a spoken answer the board listens again for a few seconds
//! without the wake word ([`VoiceConfig::follow_up`]); the wake word said
//! over the answer stops it (barge-in, on the board).
//!
//! Speech goes to the board on `audio_down`, paced in real time a little
//! ahead of the speaker, a sentence at a time: the first plays while the
//! next is synthesized. The board says `playback_end` once it has played it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Local;
use serde::Serialize;
use serde_json::{Value, json};

use crate::agent::{Action, Agent, Toolbox};
use crate::bridge::{BridgeEvent, EventSink};
use crate::intents::{self, Context};
use crate::link::{Link, Tool};
use crate::protocol::Channel;
use crate::snapshot::Snapshot;
use crate::stt::Stt;
use crate::tts::{self, Tts};

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
    /// Answer out loud through the board's speaker; without it, answers
    /// are only reported ([`BridgeEvent::Reply`]).
    pub tts: Option<Arc<Tts>>,
    /// Understand what was said with a local language model; without it,
    /// with the rules of [`intents`].
    pub agent: Option<Arc<Agent>>,
    /// After a spoken answer, listen for a few seconds more without the
    /// wake word.
    pub follow_up: bool,
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

/// What the eyes show next. `error` needs firmware 1.0; an older board
/// just goes back to idle.
fn set_state(link: &Link, state: &str) {
    if link.call("voice/state", json!({"state": state}), STATE_TIMEOUT).is_err() && state != "idle" {
        let _ = link.call("voice/state", json!({"state": "idle"}), STATE_TIMEOUT);
    }
}
const TOOL_TIMEOUT: Duration = Duration::from_secs(2);

/// What the voice thread shares with the bridge's session.
pub(crate) struct Session {
    pub link: Weak<Link>,
    pub speaker: Arc<Speaker>,
    /// The latest snapshot, for "how hot is the CPU".
    pub snapshot: Arc<Mutex<Option<Snapshot>>>,
    /// Set after a tool changed faces or rotation: the bridge reads them back.
    pub settings_changed: Arc<AtomicBool>,
    /// The board's tools for the agent, asked for once per connection.
    pub tools: Mutex<Option<Vec<Tool>>>,
}

/// What the link's reader hands the pipeline.
pub enum Hearing {
    /// The board started streaming: the language model can read its prompt
    /// meanwhile.
    Started,
    Utterance(Utterance),
}

/// The pipeline: handles each finished utterance, on its own thread so the
/// link's reader never waits on it, until the sender is dropped. The link is
/// weak because the link's reader holds that sender.
pub(crate) fn run(config: &Mutex<VoiceConfig>, session: &Session, sink: &EventSink, rx: Receiver<Hearing>) {
    for heard in rx {
        let config = config.lock().unwrap().clone();
        let utterance = match heard {
            Hearing::Utterance(u) => u,
            Hearing::Started => {
                if let (Some(agent), Some(link)) = (&config.agent, session.link.upgrade())
                    && let Err(e) = agent.prime(&BoardToolbox { link: &link, session })
                {
                    sink(BridgeEvent::VoiceError { message: e.to_string() });
                }
                continue;
            }
        };
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
        let Some(link) = session.link.upgrade() else { return };
        // The board shows thinking meanwhile.
        let follow_up = utterance.trigger == "follow_up";
        let transcript = match &config.stt {
            None => Err("idle"),
            Some(stt) => match stt.transcribe(&utterance.samples) {
                Ok(t) if is_non_speech(&t.text) => {
                    sink(BridgeEvent::Transcript { id: utterance.id, transcript: None });
                    // Nothing said after an answer is no mistake.
                    Err(if follow_up { "idle" } else { "error" })
                }
                Ok(t) => {
                    sink(BridgeEvent::Transcript { id: utterance.id, transcript: Some(t.clone()) });
                    Ok(t)
                }
                Err(e) => {
                    sink(BridgeEvent::VoiceError { message: e.to_string() });
                    Err("error")
                }
            },
        };
        let next = match transcript {
            Ok(t) => respond(&link, session, &config, sink, utterance.id, &t.text, &t.language),
            Err(state) => Next::State(state),
        };
        match next {
            Next::State(state) => set_state(&link, state),
            Next::Spoken if config.follow_up => {
                if let Err(e) = link.call("voice/listen", json!({"follow_up": true}), STATE_TIMEOUT) {
                    sink(BridgeEvent::BoardLog { line: format!("host: no follow-up: {e}") });
                }
            }
            Next::Spoken | Next::Interrupted => {}
        }
    }
}

/// What the board does once the host has answered.
enum Next {
    /// Show this state (`idle`, `error`): nothing was said.
    State(&'static str),
    /// The answer was played to the end, and the board went back to idle.
    Spoken,
    /// The wake word stopped it: the board is listening already.
    Interrupted,
}

/// Act on the words and answer.
fn respond(link: &Link, session: &Session, config: &VoiceConfig, sink: &EventSink, id: u8, text: &str, language: &str) -> Next {
    let started = Instant::now();
    let answer = config.agent.as_ref().and_then(|agent| match agent.respond(text, language, &BoardToolbox { link, session }) {
        Ok(turn) => Some((turn.reply, turn.actions.iter().map(action_line).collect(), true, "llm")),
        Err(e) => {
            sink(BridgeEvent::VoiceError { message: format!("{e}; answering with the rules instead") });
            None
        }
    });
    let (reply, actions, understood, by) = answer.unwrap_or_else(|| {
        let (reply, actions, understood) = rules(link, session, text, language);
        (reply, actions, understood, "rules")
    });
    let elapsed_ms = started.elapsed().as_millis() as u64;
    sink(BridgeEvent::Reply { id, text: reply.clone(), language: language.to_string(), actions, understood, by: by.into(), elapsed_ms });
    let Some(tts) = config.tts.as_deref() else { return Next::State("idle") };
    match session.speaker.speak(link, tts, &reply, language) {
        Ok(spoken) => {
            let next = match spoken.reason.as_str() {
                "done" => Next::Spoken,
                "barge_in" => Next::Interrupted,
                // Stopped or replaced by someone else: they say what's next.
                _ => Next::State("idle"),
            };
            sink(BridgeEvent::Spoken { id, spoken });
            next
        }
        Err(e) => {
            sink(BridgeEvent::VoiceError { message: e });
            Next::State("error")
        }
    }
}

/// "set_face: left: rings", "set_face failed: …"
fn action_line(a: &Action) -> String {
    if a.ok { format!("{}: {}", a.tool, a.result) } else { format!("{} failed: {}", a.tool, a.result.trim_start_matches("error: ")) }
}

/// The M5 rules: the reply, the actions taken, and whether anything matched.
fn rules(link: &Link, session: &Session, text: &str, language: &str) -> (String, Vec<String>, bool) {
    let volume = || {
        let state = link.call_tool("get_state", json!({}), TOOL_TIMEOUT).ok()?;
        Some(state.structured_content?.pointer("/audio/volume")?.as_u64()? as u8)
    };
    let ctx = Context { snapshot: session.snapshot.lock().unwrap().clone(), volume: volume() };
    let plan = intents::understand(text, language, &ctx);
    let toolbox = BoardToolbox { link, session };
    let mut actions = Vec::new();
    for (tool, args) in &plan.calls {
        let result = toolbox.call(tool, args);
        let ok = result.is_ok();
        actions.push(action_line(&Action { tool: tool.clone(), arguments: args.clone(), result: result.unwrap_or_else(|e| e), ok }));
        if !ok {
            return (plan.failure, actions, plan.understood);
        }
    }
    (plan.reply, actions, plan.understood)
}

/// Board tools the voice agent doesn't get: muted by voice, the board
/// couldn't be unmuted by voice; the eyes are a setting for the app and a
/// toy, not worth a tool in a small model's prompt.
const NOT_BY_VOICE: &[&str] = &["set_mic", "set_eyes", "play_eyes"];

/// The board's tools as the voice agent gets them: without those in
/// [`NOT_BY_VOICE`], plus the host's `get_metrics`.
pub fn voice_tools(board: Vec<Tool>) -> Vec<Tool> {
    let metrics = Tool {
        name: "get_metrics".into(),
        description: "Current CPU and GPU temperature, load, clock, power and memory of this computer, and its fan speeds.".into(),
        input_schema: json!({"type": "object", "properties": {}}),
    };
    board.into_iter().filter(|t| !NOT_BY_VOICE.contains(&t.name.as_str())).chain([metrics]).collect()
}

/// `get_state` without what only a developer wants (memory, link and UI
/// statistics): fewer tokens for the model to read.
pub fn trim_state(state: &Value) -> Value {
    let mut out = json!({"screens": state["screens"], "voice": {}, "audio": {"volume": state["audio"]["volume"]}});
    for key in ["wake_word", "wake_words", "muted"] {
        out["voice"][key] = state["voice"][key].clone();
    }
    out
}

/// The board's tools, run over the link.
struct BoardToolbox<'a> {
    link: &'a Link,
    session: &'a Session,
}

impl Toolbox for BoardToolbox<'_> {
    fn tools(&self) -> Vec<Tool> {
        let mut tools = self.session.tools.lock().unwrap();
        if tools.is_none() {
            *tools = self.link.list_tools(TOOL_TIMEOUT).ok().map(voice_tools);
        }
        tools.clone().unwrap_or_default()
    }

    fn call(&self, name: &str, arguments: &Value) -> Result<String, String> {
        if name == "get_metrics" {
            let snapshot = self.session.snapshot.lock().unwrap().clone().ok_or("no sensor reading yet")?;
            return Ok(snapshot.metrics_json().to_string());
        }
        if NOT_BY_VOICE.contains(&name) {
            return Err(format!("{name} can't be used by voice"));
        }
        let result = self.link.call_tool(name, arguments.clone(), TOOL_TIMEOUT).map_err(|e| e.to_string())?;
        if result.is_error {
            return Err(result.text());
        }
        if matches!(name, "set_face" | "set_rotation") {
            self.session.settings_changed.store(true, Ordering::Relaxed);
        }
        Ok(match (name, &result.structured_content) {
            ("get_state", Some(state)) => trim_state(state).to_string(),
            _ => result.text(),
        })
    }
}

/// Samples per `audio_down` frame: 64 ms, 2 KB.
const PLAY_FRAME: usize = 1024;
/// How far ahead of the speaker the host keeps the board's buffer.
const PLAY_LEAD: Duration = Duration::from_millis(500);
/// Past the end of the audio, how long to wait for `playback_end`.
const PLAY_GRACE: Duration = Duration::from_secs(3);

/// How a reply went out.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Spoken {
    pub text: String,
    /// From the request to the first frame sent: synthesizing the first sentence.
    pub first_audio_ms: u64,
    /// What the board played, from its `playback_end`.
    pub played_ms: u64,
    /// Why it ended: `done`, `stopped`, `replaced`, `starved` or `barge_in`
    /// (the wake word was said over it).
    pub reason: String,
    /// Times the board ran out of audio mid-stream.
    pub underruns: u32,
    /// Frames the board never got.
    pub lost: u32,
}

/// Plays speech on the board, one reply at a time. Shared by the voice
/// pipeline and the hub (`host/say`); the link's reader hands it the board's
/// `playback_end` notifications.
#[derive(Default)]
pub struct Speaker {
    /// Held while a reply plays; the id of the last stream.
    playing: Mutex<u8>,
    ended: Mutex<Option<Value>>,
    ended_cv: Condvar,
}

impl Speaker {
    /// `playback_end` from the board.
    pub fn playback_end(&self, params: Value) {
        *self.ended.lock().unwrap() = Some(params);
        self.ended_cv.notify_all();
    }

    /// The board said stream `id` ended (it may do so before the last frame:
    /// stopped, barge-in).
    fn has_ended(&self, id: u8) -> bool {
        self.ended.lock().unwrap().as_ref().is_some_and(|p| p["id"].as_u64() == Some(id as u64))
    }

    /// Synthesize `text` in `language` and play it, a sentence at a time.
    /// Returns once the board has played it.
    pub fn speak(&self, link: &Link, tts: &Tts, text: &str, language: &str) -> Result<Spoken, String> {
        let started = Instant::now();
        let (tx, rx) = mpsc::sync_channel::<Result<Vec<i16>, String>>(2);
        let sentences = tts::sentences(text);
        thread::scope(|scope| {
            scope.spawn(move || {
                for s in sentences {
                    let audio = tts.synthesize(&s, language).map_err(|e| e.to_string());
                    let failed = audio.is_err();
                    if tx.send(audio).is_err() || failed {
                        return;
                    }
                }
            });
            let mut first_audio_ms = None;
            let played = self.play(link, rx.into_iter().inspect(|_| {
                first_audio_ms.get_or_insert(started.elapsed().as_millis() as u64);
            }))?;
            Ok(Spoken { text: text.to_string(), first_audio_ms: first_audio_ms.unwrap_or(0), ..played })
        })
    }

    /// Stream audio to the board as it comes, paced in real time, then wait
    /// for the board to finish playing it.
    pub fn play(&self, link: &Link, audio: impl IntoIterator<Item = Result<Vec<i16>, String>>) -> Result<Spoken, String> {
        let mut id = self.playing.lock().unwrap();
        *id = id.wrapping_add(1);
        let id = *id;
        *self.ended.lock().unwrap() = None;
        let mut seq = 0u16;
        let mut pending: Vec<i16> = Vec::new();
        // Where the board's speaker is: `sent` samples are due at `clock + sent / rate`.
        let mut clock: Option<Instant> = None;
        let mut sent = 0u64;
        let mut failure = None;
        let mut send = |samples: &[i16], last: bool, clock: &mut Option<Instant>, sent: &mut u64| -> io::Result<()> {
            let start = *clock.get_or_insert_with(Instant::now);
            let due = |n: u64| start + Duration::from_micros(n * 1_000_000 / SAMPLE_RATE as u64);
            // Keep at most PLAY_LEAD in the board's buffer.
            let now = Instant::now();
            if due(*sent) < now {
                // We fell behind (a slow sentence): the board played
                // silence meanwhile, so its clock moved on.
                *clock = Some(now - Duration::from_micros(*sent * 1_000_000 / SAMPLE_RATE as u64));
            } else if let Some(wait) = due(*sent).checked_duration_since(now + PLAY_LEAD) {
                thread::sleep(wait);
            }
            let mut frame = Vec::with_capacity(4 + samples.len() * 2);
            frame.extend_from_slice(&[id, u8::from(last)]);
            frame.extend_from_slice(&seq.to_le_bytes());
            for s in samples {
                frame.extend_from_slice(&s.to_le_bytes());
            }
            seq = seq.wrapping_add(1);
            *sent += samples.len() as u64;
            link.send(Channel::AudioDown, &frame)
        };
        let mut cut_short = false;
        for chunk in audio {
            if self.has_ended(id) {
                cut_short = true;
                break;
            }
            match chunk {
                Ok(samples) => pending.extend(samples),
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
            while pending.len() > PLAY_FRAME && !cut_short {
                let rest = pending.split_off(PLAY_FRAME);
                send(&pending, false, &mut clock, &mut sent).map_err(|e| e.to_string())?;
                pending = rest;
                cut_short = self.has_ended(id);
            }
            if cut_short {
                break;
            }
        }
        if !cut_short {
            send(&pending, true, &mut clock, &mut sent).map_err(|e| e.to_string())?;
        }
        let deadline = clock.unwrap_or_else(Instant::now) + Duration::from_micros(sent * 1_000_000 / SAMPLE_RATE as u64) + PLAY_GRACE;
        let mut ended = self.ended.lock().unwrap();
        loop {
            if let Some(p) = ended.as_ref().filter(|p| p["id"].as_u64() == Some(id as u64)) {
                if let Some(e) = failure {
                    return Err(e);
                }
                let n = |k: &str| p[k].as_u64().unwrap_or(0);
                return Ok(Spoken {
                    text: String::new(),
                    first_audio_ms: 0,
                    played_ms: n("ms"),
                    reason: p["reason"].as_str().unwrap_or("done").to_string(),
                    underruns: n("underruns") as u32,
                    lost: n("lost") as u32,
                });
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(failure.unwrap_or_else(|| "the board didn't say it had played the reply".into()));
            }
            ended = self.ended_cv.wait_timeout(ended, deadline - now).unwrap().0;
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
    pipeline: Sender<Hearing>,
}

impl Receiving {
    pub fn new(pipeline: Sender<Hearing>) -> Self {
        Self { capture: Capture::default(), pipeline }
    }

    pub fn start(&mut self, params: &Value) {
        self.capture.start(params);
        let _ = self.pipeline.send(Hearing::Started);
    }

    pub fn audio(&mut self, payload: &[u8]) {
        self.capture.audio(payload);
    }

    pub fn end(&mut self, params: &Value) {
        if let Some(u) = self.capture.end(params) {
            let _ = self.pipeline.send(Hearing::Utterance(u));
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
