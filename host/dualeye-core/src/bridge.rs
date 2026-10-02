//! Sample sensors at a fixed rate and send each snapshot to the board,
//! reconnecting when the USB cable or the board goes away.
//!
//! Frontends observe the loop through [`BridgeEvent`]s: the CLI prints them,
//! a Tauri app can forward them to the webview with `app.emit(..)`.
//!
//! Each connection starts with the protocol v2 handshake (see [`crate::link`]).
//! Faces and rotation live on the board (NVS), so by default the bridge then
//! reads them with `get_state` and adopts them, raising [`BridgeEvent::Settings`]:
//! a change made while no bridge ran (say, over MCP) survives. From then on,
//! whatever the frontend picks is pushed with board tools.
//!
//! With a [`Hub`] other processes (the MCP server, `dualeye call`) use the
//! board through this connection. When one of them changes a face or a
//! rotation, the bridge reads the board's state back into `faces` and
//! `rotation` and raises [`BridgeEvent::Settings`], so the frontend follows.
//!
//! The hub also brings Claude Code's hook events (`host/claude_hook`). With
//! the Claude usage of each snapshot they go to a thread of their own, which
//! tells the person on the board when Claude needs them, finished a long turn
//! or is running out of its limits ([`crate::claude::alerts`]), once the board
//! is out of any voice conversation.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;

use crate::claude::alerts::{self, Alert, AlertSettings, Alerts, HookEvent};
use crate::claude::{ClaudeMetrics, ClaudeUsage};
use crate::firmware::{self, BoardFirmware};
use crate::hub::Hub;
use crate::link::{CallError, Link, LinkEvent};
use crate::protocol::Channel;
use crate::sensors::Collector;
use crate::serial;
use crate::snapshot::{Face, Faces, Rotation, Rotations, Snapshot};
use crate::stt::Transcript;
use crate::voice::{self, Hearing, Receiving, Session, Speaker, Spoken, Utterance, VoiceConfig};
use crate::intents;

#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// Serial port; `None` auto-detects the Espressif board on every (re)connect.
    pub port: Option<String>,
    /// Time between snapshots. The firmware marks data stale after 3 s.
    pub interval: Duration,
    /// Watch face of each screen. Shared, so a frontend can change it while
    /// the bridge runs; the board gets it by the next snapshot.
    pub faces: Arc<Mutex<Faces>>,
    /// How each screen is turned, shared the same way as `faces`.
    pub rotation: Arc<Mutex<Rotations>>,
    /// Lets other processes use the board through this bridge.
    pub hub: Option<Arc<Hub>>,
    /// On each handshake, take `faces` and `rotation` from the board instead
    /// of pushing ours. Off when the user asked for specific ones.
    pub adopt_board_settings: bool,
    /// What happens to what the board hears after its wake word. Shared, so
    /// a frontend can turn transcription on or change the model while the
    /// bridge runs; each utterance uses the config of the moment it ends.
    pub voice: Arc<Mutex<VoiceConfig>>,
    /// Which Claude Code alerts to give, and how. Shared like `voice`.
    pub claude_alerts: Arc<Mutex<AlertSettings>>,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            port: None,
            interval: Duration::from_secs(1),
            faces: Arc::default(),
            rotation: Arc::default(),
            hub: None,
            adopt_board_settings: true,
            voice: Arc::default(),
            claude_alerts: Arc::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BridgeEvent {
    /// No board to talk to yet; the bridge keeps retrying.
    Waiting { reason: String },
    Connected { port: String },
    /// A snapshot was taken; `sent` is false when it had no temperature to
    /// show or the board isn't ready for it.
    Snapshot { snapshot: Snapshot, sent: bool },
    /// A line from the board's log, or console text from outside the
    /// protocol (boot messages, a crash).
    BoardLog { line: String },
    /// What the board runs; raised once per connection, and again if it reboots into another version.
    Firmware { firmware: BoardFirmware },
    /// The board heard its wake word. `volume_db` is the input level (dBFS).
    Wake { word: String, volume_db: Option<f64> },
    /// What the board's "eyes" overlay shows: `idle`, `listening`, `thinking` or `speaking`.
    VoiceState { state: String },
    /// The board started streaming what it hears: after the wake word
    /// (`trigger` `wake`) or `voice/listen` (`host`).
    Listening { id: u8, trigger: String },
    /// An utterance ended and arrived. `wav` is where a copy was saved, when
    /// [`VoiceConfig::dump_dir`] is set.
    Utterance { utterance: Utterance, duration_ms: u64, peak_db: Option<f64>, wav: Option<String> },
    /// What was said in utterance `id`; `None` when Whisper heard no words.
    Transcript { id: u8, transcript: Option<Transcript> },
    /// What the host made of transcript `id`: the board tools it called
    /// (`actions`, with what each said) and the answer, in `language`.
    /// `understood` is false when no command matched. `by` is `llm` (the
    /// local language model) or `rules` (without one, or when it failed).
    Reply { id: u8, text: String, language: String, actions: Vec<String>, understood: bool, by: String, elapsed_ms: u64 },
    /// The answer to `id` was spoken through the board's speaker.
    Spoken { id: u8, spoken: Spoken },
    /// Speech-to-text or text-to-speech failed, or its sidecar couldn't start.
    VoiceError { message: String },
    /// Faces or rotation came from the board: at the handshake, or because a
    /// hub client changed them. `faces` and `rotation` in the config hold the
    /// new values already.
    Settings { faces: Faces, rotation: Rotations },
    /// An alert about Claude Code, and how it went: `error` says why it
    /// didn't reach the board (or wasn't spoken).
    ClaudeAlert { alert: Alert, text: String, error: Option<String> },
    Disconnected { port: String, reason: String, permission_denied: bool },
}

pub type EventSink = Arc<dyn Fn(BridgeEvent) + Send + Sync>;

/// The bridge loop on its own thread. Dropping it stops the loop.
pub struct Bridge {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Bridge {
    pub fn spawn(config: BridgeConfig, on_event: impl Fn(BridgeEvent) + Send + Sync + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let sink: EventSink = Arc::new(on_event);
        let thread = thread::Builder::new()
            .name("dualeye-bridge".into())
            .spawn(move || run(&config, &flag, sink))
            .expect("spawn bridge thread");
        Self { stop, thread: Some(thread) }
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// How long each `hello` of the handshake waits; the handshake repeats it every tick.
const HELLO_TIMEOUT: Duration = Duration::from_millis(500);
/// Unanswered `hello`s before also asking the protocol 1 way.
const LEGACY_QUERY_AFTER: u32 = 3;
/// Once a board has gone that long without answering, ask only this often.
const HELLO_BACKOFF: Duration = Duration::from_secs(5);
const TOOL_TIMEOUT: Duration = Duration::from_secs(1);

/// Run the bridge on the current thread until `stop` is set.
pub fn run(config: &BridgeConfig, stop: &AtomicBool, on_event: EventSink) {
    // Whisper, Piper and llama.cpp load their models meanwhile, so the first utterance doesn't wait.
    let (stt, tts, agent) = {
        let voice = config.voice.lock().unwrap();
        (voice.stt.clone(), voice.tts.clone(), voice.agent.clone())
    };
    if let Some(agent) = agent {
        let sink = on_event.clone();
        let _ = thread::Builder::new().name("dualeye-llm-start".into()).spawn(move || match agent.llm().warm_up() {
            Ok(()) => sink(BridgeEvent::BoardLog { line: format!("host: language model ready ({})", agent.llm().config().model.display()) }),
            Err(e) => sink(BridgeEvent::VoiceError { message: e.to_string() }),
        });
    }
    if let Some(stt) = stt {
        let sink = on_event.clone();
        let _ = thread::Builder::new().name("dualeye-stt-start".into()).spawn(move || match stt.warm_up() {
            Ok(()) => sink(BridgeEvent::BoardLog { line: format!("host: speech-to-text ready ({})", stt.config().model.display()) }),
            Err(e) => sink(BridgeEvent::VoiceError { message: e.to_string() }),
        });
    }
    if let Some(tts) = tts {
        let sink = on_event.clone();
        let _ = thread::Builder::new().name("dualeye-tts-start".into()).spawn(move || match tts.warm_up() {
            Ok(()) => {
                let voices: Vec<&str> = tts.config().voices.values().map(String::as_str).collect();
                sink(BridgeEvent::BoardLog { line: format!("host: text-to-speech ready ({})", voices.join(", ")) })
            }
            Err(e) => sink(BridgeEvent::VoiceError { message: e.to_string() }),
        });
    }
    let (alert_tx, alert_rx) = mpsc::channel::<AlertInput>();
    let target = Mutex::new(None::<AlertTarget>);
    if let Some(hub) = &config.hub {
        let tx = Mutex::new(alert_tx.clone());
        hub.set_claude_hook(Some(Arc::new(move |json| {
            if let Some(event) = alerts::parse_hook(&json) {
                let _ = tx.lock().unwrap().send(AlertInput::Hook(event, Instant::now()));
            }
        })));
    }
    thread::scope(|scope| {
        let _ = thread::Builder::new()
            .name("dualeye-alerts".into())
            .spawn_scoped(scope, || alert_loop(config, stop, &target, alert_rx, &on_event));
        connect_loop(config, stop, &alert_tx, &target, &on_event);
    });
    if let Some(hub) = &config.hub {
        hub.set_claude_hook(None);
    }
}

/// Connect to the board, run a session, and again when it ends, until `stop`.
fn connect_loop(config: &BridgeConfig, stop: &AtomicBool, alert_tx: &Sender<AlertInput>, target: &Mutex<Option<AlertTarget>>, on_event: &EventSink) {
    let mut collector = Collector::new();
    let mut claude = ClaudeUsage::new();
    let mut delay = Duration::from_secs(1);
    while !stop.load(Ordering::Relaxed) {
        let Some(port) = config.port.clone().or_else(serial::detect_board) else {
            let reason = "board not found on USB";
            if let Some(hub) = &config.hub {
                hub.set_unavailable(reason);
            }
            on_event(BridgeEvent::Waiting { reason: reason.into() });
            sleep_unless_stopped(delay, stop);
            delay = (delay * 2).min(Duration::from_secs(5));
            continue;
        };
        let started = Instant::now();
        let result = session(&port, config, stop, &mut collector, &mut claude, alert_tx, target, on_event);
        *target.lock().unwrap() = None;
        if let Some(hub) = &config.hub {
            let why = result.as_ref().err().map_or_else(|| "the bridge stopped".to_string(), |e| format!("board disconnected: {e}"));
            hub.set_unavailable(&why);
            hub.set_say(None);
        }
        if let Err(err) = result {
            on_event(BridgeEvent::Disconnected {
                port,
                reason: err.to_string(),
                permission_denied: err.kind() == io::ErrorKind::PermissionDenied,
            });
            if started.elapsed() > Duration::from_secs(10) {
                delay = Duration::from_secs(1);
            }
            sleep_unless_stopped(delay, stop);
            delay = (delay * 2).min(Duration::from_secs(15));
        }
    }
}

/// What the reader thread learns for the session loop.
#[derive(Default)]
struct Heard {
    /// The board announced a reboot: handshake and push settings again.
    rebooted: AtomicBool,
    /// What a board that doesn't speak protocol v2 printed about itself.
    old_firmware: Mutex<Option<BoardFirmware>>,
}

#[allow(clippy::too_many_arguments)]
fn session(
    port: &str,
    config: &BridgeConfig,
    stop: &AtomicBool,
    collector: &mut Collector,
    claude: &mut ClaudeUsage,
    alert_tx: &Sender<AlertInput>,
    target: &Mutex<Option<AlertTarget>>,
    on_event: &EventSink,
) -> io::Result<()> {
    let heard = Arc::new(Heard::default());
    let speaker = Arc::new(Speaker::default());
    let voice_state = Arc::new(Mutex::new("idle".to_string()));
    let latest = Arc::new(Mutex::new(None::<Snapshot>));
    let voice_changed_settings = Arc::new(AtomicBool::new(false));
    // The pipeline needs the link to answer the board; it gets it once open.
    let (link_slot_tx, link_slot_rx) = std::sync::mpsc::channel::<Session>();
    let pipeline = {
        let config = config.voice.clone();
        let sink = on_event.clone();
        let (tx, rx) = std::sync::mpsc::channel::<Hearing>();
        thread::Builder::new().name("dualeye-voice".into()).spawn(move || {
            if let Ok(session) = link_slot_rx.recv() {
                voice::run(&config, &session, &sink, rx);
            }
        })?;
        tx
    };
    let receiving = Mutex::new(Receiving::new(pipeline));
    let link = Arc::new({
        let heard = heard.clone();
        let sink = on_event.clone();
        let speaker = speaker.clone();
        let voice_state = voice_state.clone();
        Link::open(port, move |event| match event {
            LinkEvent::Log(line) => sink(BridgeEvent::BoardLog { line }),
            LinkEvent::Text(line) => {
                let old = firmware::parse_version_line(&line)
                    .or_else(|| firmware::is_missing_app_log(&line).then_some(BoardFirmware::Missing));
                if old.is_some() {
                    *heard.old_firmware.lock().unwrap() = old;
                }
                sink(BridgeEvent::BoardLog { line });
            }
            LinkEvent::Notification { method, .. } if method == "ready" => heard.rebooted.store(true, Ordering::Relaxed),
            LinkEvent::Notification { method, params } if method == "wake" => sink(BridgeEvent::Wake {
                word: params["word"].as_str().unwrap_or_default().to_string(),
                volume_db: params["volume_db"].as_f64(),
            }),
            LinkEvent::Notification { method, params } if method == "voice_state" => {
                if let Some(state) = params["state"].as_str() {
                    *voice_state.lock().unwrap() = state.to_string();
                    sink(BridgeEvent::VoiceState { state: state.to_string() });
                }
            }
            LinkEvent::Notification { method, params } if method == "utterance_start" => {
                receiving.lock().unwrap().start(&params);
                sink(BridgeEvent::Listening {
                    id: params["id"].as_u64().unwrap_or(0) as u8,
                    trigger: params["trigger"].as_str().unwrap_or("wake").to_string(),
                });
            }
            LinkEvent::Notification { method, params } if method == "utterance_end" => receiving.lock().unwrap().end(&params),
            LinkEvent::Notification { method, params } if method == "playback_end" => speaker.playback_end(params),
            LinkEvent::Audio(payload) => receiving.lock().unwrap().audio(&payload),
            LinkEvent::Notification { .. } | LinkEvent::Closed(_) => {}
        })?
    });
    let _ = link_slot_tx.send(Session {
        link: Arc::downgrade(&link),
        speaker: speaker.clone(),
        snapshot: latest.clone(),
        settings_changed: voice_changed_settings.clone(),
        tools: Mutex::default(),
    });
    on_event(BridgeEvent::Connected { port: port.to_string() });

    let mut reported: Option<BoardFirmware> = None;
    let mut report = |firmware: BoardFirmware| {
        if reported.as_ref() != Some(&firmware) {
            reported = Some(firmware.clone());
            on_event(BridgeEvent::Firmware { firmware });
        }
    };
    let closed = |link: &Link| io::Error::new(io::ErrorKind::BrokenPipe, link.closed_reason().unwrap_or_else(|| "port closed".into()));

    let mut ready = false;
    let mut unanswered = 0u32;
    let mut next_hello = Instant::now();
    let mut pushed: Option<(Faces, Rotations)> = None;
    let mut next = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        if !link.is_alive() {
            return Err(closed(&link));
        }
        if heard.rebooted.swap(false, Ordering::Relaxed) {
            if let Some(hub) = &config.hub {
                hub.set_unavailable("the board is restarting");
            }
            ready = false;
            pushed = None;
            next_hello = Instant::now();
        }
        if !ready && Instant::now() >= next_hello {
            match link.hello(HELLO_TIMEOUT) {
                Ok(hello) => {
                    ready = true;
                    unanswered = 0;
                    if let Some(hub) = &config.hub {
                        hub.attach(link.clone(), hello.clone(), port);
                        hub.set_say(Some(say_fn(&link, &speaker, config)));
                    }
                    *target.lock().unwrap() =
                        Some(AlertTarget { link: Arc::downgrade(&link), speaker: speaker.clone(), voice_state: voice_state.clone() });
                    report(BoardFirmware::Version { version: hello.firmware, idf: hello.idf, protocol: hello.protocol });
                    if config.adopt_board_settings {
                        // Otherwise `pushed` stays empty and ours are pushed below.
                        adopt_settings(&link, config, &mut pushed, on_event)?;
                    }
                }
                Err(CallError::Io(e)) => return Err(e),
                Err(CallError::Closed) => return Err(closed(&link)),
                Err(_) => {
                    unanswered += 1;
                    if unanswered == LEGACY_QUERY_AFTER {
                        link.send_raw(firmware::VERSION_QUERY.as_bytes())?;
                    }
                    if unanswered > 2 * LEGACY_QUERY_AFTER {
                        next_hello = Instant::now() + HELLO_BACKOFF;
                    }
                }
            }
            if let Some(old) = heard.old_firmware.lock().unwrap().take().filter(|_| !ready) {
                report(old);
            }
        }

        let hub_changed = config.hub.as_ref().is_some_and(|h| h.take_settings_changed());
        if ready && (hub_changed | voice_changed_settings.swap(false, Ordering::Relaxed)) {
            adopt_settings(&link, config, &mut pushed, on_event)?;
        }

        if ready {
            let want = (*config.faces.lock().unwrap(), *config.rotation.lock().unwrap());
            if pushed != Some(want) {
                match push_settings(&link, want.0, want.1) {
                    Ok(()) => pushed = Some(want),
                    Err(CallError::Io(e)) => return Err(e),
                    // Retried on the next tick.
                    Err(e) => on_event(BridgeEvent::BoardLog { line: format!("host: settings not applied: {e}") }),
                }
            }
        }

        let mut snapshot = collector.sample();
        snapshot.face = Some(*config.faces.lock().unwrap());
        snapshot.rot = Some(*config.rotation.lock().unwrap()).filter(|r| !r.is_upright());
        snapshot.claude = claude.sample();
        if let Some(metrics) = &snapshot.claude {
            let _ = alert_tx.send(AlertInput::Metrics(metrics.clone()));
        }
        if let Some(hub) = &config.hub {
            hub.set_snapshot(&snapshot);
        }
        *latest.lock().unwrap() = Some(snapshot.clone());
        let sent = ready && snapshot.is_sendable();
        if sent {
            link.send(Channel::Metrics, &snapshot.to_payload())?;
        }
        on_event(BridgeEvent::Snapshot { snapshot, sent });

        next += config.interval;
        let now = Instant::now();
        if next < now {
            next = now;
        }
        sleep_unless_stopped(next - now, stop);
    }
    Ok(())
}

/// What the alerts thread gets: a hook event (with when it came) or the
/// Claude usage of a snapshot.
enum AlertInput {
    Hook(HookEvent, Instant),
    Metrics(ClaudeMetrics),
}

/// The board the alerts go to, while a session has one.
struct AlertTarget {
    link: Weak<Link>,
    speaker: Arc<Speaker>,
    /// What the board's voice overlay shows, from its `voice_state`.
    voice_state: Arc<Mutex<String>>,
}

/// An alert waits this long at most for a voice conversation to end.
const CONVERSATION_WAIT: Duration = Duration::from_secs(20);

fn alert_loop(config: &BridgeConfig, stop: &AtomicBool, target: &Mutex<Option<AlertTarget>>, rx: Receiver<AlertInput>, on_event: &EventSink) {
    let mut alerts = Alerts::default();
    while !stop.load(Ordering::Relaxed) {
        let input = match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(input) => input,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let settings = config.claude_alerts.lock().unwrap().clone();
        let alert = match &input {
            AlertInput::Hook(event, at) => alerts.on_hook(event, &settings, *at),
            AlertInput::Metrics(metrics) => alerts.on_metrics(metrics, &settings),
        };
        let Some(alert) = alert else { continue };
        let error = give_alert(config, stop, target, &settings, &alert).err();
        let text = alert.words(&settings.language).spoken;
        on_event(BridgeEvent::ClaudeAlert { alert, text, error });
    }
}

/// Wait for the board to be out of a conversation, then show and say `alert`.
fn give_alert(config: &BridgeConfig, stop: &AtomicBool, target: &Mutex<Option<AlertTarget>>, settings: &AlertSettings, alert: &Alert) -> Result<(), String> {
    let board = || {
        let target = target.lock().unwrap();
        let t = target.as_ref()?;
        Some((t.link.upgrade()?, t.speaker.clone(), t.voice_state.clone()))
    };
    let (link, speaker, voice_state) = board().ok_or("the board isn't connected")?;
    let until = Instant::now() + CONVERSATION_WAIT;
    while *voice_state.lock().unwrap() != "idle" && Instant::now() < until && !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(250));
    }
    let tts = if settings.speak { config.voice.lock().unwrap().tts.clone() } else { None };
    alerts::deliver(&link, &speaker, tts.as_deref(), alert, &settings.language)
}

/// `host/say` for the hub: speak with the voice config of the moment.
fn say_fn(link: &Arc<Link>, speaker: &Arc<Speaker>, config: &BridgeConfig) -> crate::hub::SayFn {
    let (link, speaker, voice) = (Arc::downgrade(link), speaker.clone(), config.voice.clone());
    Arc::new(move |text: &str, language: Option<&str>| {
        let tts = voice.lock().unwrap().tts.clone().ok_or("the bridge isn't speaking: turn on spoken replies in the app, or run dualeye --tts")?;
        let link = link.upgrade().ok_or("the board disconnected")?;
        let language = language.unwrap_or_else(|| intents::guess_language(text));
        let spoken = speaker.speak(&link, &tts, text, language)?;
        Ok(serde_json::to_value(spoken).unwrap_or_default())
    })
}

fn push_settings(link: &Link, faces: Faces, rotation: Rotations) -> Result<(), CallError> {
    for (screen, face, rot) in [("left", faces.cpu, rotation.cpu), ("right", faces.gpu, rotation.gpu)] {
        let calls = [
            ("set_face", json!({"screen": screen, "face": face.name()})),
            ("set_rotation", json!({"screen": screen, "degrees": rot.degrees()})),
        ];
        for (tool, args) in calls {
            let result = link.call_tool(tool, args, TOOL_TIMEOUT)?;
            if result.is_error {
                return Err(CallError::Invalid(format!("{tool}: {}", result.text())));
            }
        }
    }
    Ok(())
}

/// Take the board's faces and rotation as ours. Only an I/O error is returned;
/// on any other failure `pushed` is left alone.
fn adopt_settings(link: &Link, config: &BridgeConfig, pushed: &mut Option<(Faces, Rotations)>, on_event: &EventSink) -> io::Result<()> {
    match read_settings(link) {
        Ok((faces, rotation)) => {
            *config.faces.lock().unwrap() = faces;
            *config.rotation.lock().unwrap() = rotation;
            *pushed = Some((faces, rotation));
            on_event(BridgeEvent::Settings { faces, rotation });
        }
        Err(CallError::Io(e)) => return Err(e),
        Err(e) => on_event(BridgeEvent::BoardLog { line: format!("host: could not read the board's settings: {e}") }),
    }
    Ok(())
}

/// Faces and rotation as the board has them, from `get_state`.
fn read_settings(link: &Link) -> Result<(Faces, Rotations), CallError> {
    let state = link.call_tool("get_state", json!({}), TOOL_TIMEOUT)?;
    let screens = state.structured_content.as_ref().and_then(|s| s.get("screens")).ok_or_else(|| CallError::Invalid("get_state: no screens".into()))?;
    let screen = |name: &str| -> Result<(Face, Rotation), CallError> {
        let s = &screens[name];
        let face = s["face"].as_str().and_then(|f| f.parse().ok());
        let rotation = s["rotation"].as_u64().and_then(|d| Rotation::from_degrees(d as u16));
        face.zip(rotation).ok_or_else(|| CallError::Invalid(format!("get_state: {name} screen")))
    };
    let (left, right) = (screen("left")?, screen("right")?);
    Ok((Faces { cpu: left.0, gpu: right.0 }, Rotations { cpu: left.1, gpu: right.1 }))
}

fn sleep_unless_stopped(duration: Duration, stop: &AtomicBool) {
    let end = Instant::now() + duration;
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now >= end {
            return;
        }
        thread::sleep((end - now).min(Duration::from_millis(50)));
    }
}
