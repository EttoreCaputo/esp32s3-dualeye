//! Tauri shell around `dualeye-core`: the bridge runs on its own thread for the
//! whole life of the process, and every event it raises is kept here (so a
//! freshly loaded webview can catch up) and forwarded to the UI as `bridge`.
//!
//! Closing the window only hides it; the tray icon brings it back or quits, and
//! launching the app again shows the running instance, unless a newer build
//! was installed meanwhile: then that one takes over (see `instances`).
//!
//! Identifying and flashing the board go through esptool, which the app sets
//! up by itself on first use (Python + virtualenv in its data folder); the
//! bridge is stopped meanwhile so esptool can own the port, then started again.
//!
//! For the Claude faces the app can also point Claude Code's status line at
//! itself (`main.rs` handles that invocation); see `dualeye_core::claude`.
//! Likewise Claude Code's hooks, for the alerts: Claude needs you, it's done,
//! the limits are running out (`dualeye_core::claude::alerts`).
//!
//! Voice (opt-in, off by default): with it on, what the board hears after its
//! wake word is transcribed by a whisper.cpp `whisper-server` sidecar with a
//! model the app downloads (`dualeye_core::models`), understood by a small
//! language model in a llama.cpp `llama-server` sidecar that calls the
//! board's tools (`dualeye_core::agent`; without one, a few fixed phrases,
//! `dualeye_core::intents`) and answered out loud by a Piper sidecar through
//! the board's speaker. The app installs Piper itself, into
//! a virtualenv made with esptool's Python. Turning voice on, or picking
//! another model, language or voice, swaps the bridge's shared voice config:
//! no reconnect.
//!
//! The same binary with `--mcp` is an MCP server for Claude Code and Claude
//! Desktop (`dualeye_core::mcp`). It reaches the board through this app's
//! [`Hub`], which lives as long as the app, across bridge restarts.
//!
//! Timers, reminders and a pomodoro (`dualeye_core::timers`) live here too,
//! in `timers.json` next to the hub file: the bridge ticks them and the board
//! counts the first one down and rings; the Timers tab, the voice and MCP
//! clients set them.
//!
//! On macOS the app can install a root helper that reads the exact CPU power,
//! which macOS 27 hides from ordinary apps (`power_helper`).

mod instances;
mod power_helper;

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use dualeye_core::claude::alerts::AlertSettings;
use dualeye_core::claude::hooks::{self, HooksStatus};
use dualeye_core::claude::statusline::{self, LinkStatus};
use dualeye_core::flasher::setup;
use dualeye_core::hardware::{self, Hardware, Recommendation};
use dualeye_core::llm::{self, Llm, LlmConfig};
use dualeye_core::models::{self, Kind, Model};
use dualeye_core::stt::{self, Stt, SttConfig, SttLanguage, Transcript};
use dualeye_core::tts::{self, Tts, TtsConfig};
use dualeye_core::timers::{self, Pomodoro, ShowOn, TimerInfo, Timers};
use dualeye_core::voice::{self, Spoken, VoiceConfig};
use dualeye_core::{
    Agent, Board, BoardFirmware, Bridge, BridgeConfig, BridgeEvent, ChipInfo, Collector, Esptool, Faces, FlashEvent, Hub, HubStatus, ImageInfo, PortInfo, Reading, Rotations, Snapshot, firmware,
    mcp, media, serial,
};
use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

const LOG_LINES: usize = 300;
const TRANSCRIPTS: usize = 50;
const TRAY_ID: &str = "main";
/// The image in the repository's `build/` folder, shipped inside the app.
const FIRMWARE: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../build/merged-binary.bin"));

#[derive(Default, Clone, Serialize, Deserialize)]
struct Settings {
    /// `None` auto-detects the board.
    port: Option<String>,
    #[serde(default)]
    faces: Faces,
    #[serde(default)]
    rotation: Rotations,
    #[serde(default)]
    voice: VoiceSettings,
    #[serde(default)]
    claude_alerts: AlertSettings,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct VoiceSettings {
    /// Transcribe what the board hears. Off until the person turns it on.
    enabled: bool,
    /// A `models::MODELS` id.
    model: String,
    language: SttLanguage,
    /// Keep each utterance as a WAV file (`voice/` in DualEye's data folder).
    keep_recordings: bool,
    /// Answer out loud through the board's speaker.
    speak: bool,
    /// After a spoken answer, listen a few seconds more without the wake word.
    follow_up: bool,
    /// Voice by language (`it`, `en`): `models::MODELS` ids.
    voices: BTreeMap<String, String>,
    /// Understand commands with a local language model; without it, a few
    /// fixed phrases.
    llm: bool,
    /// A `models::MODELS` id of kind `llm`.
    llm_model: String,
}

/// This computer and the models for it, with what llama-server says it can
/// run on once [`check_hardware`] has asked it; until then without.
fn hardware() -> (Hardware, Recommendation) {
    let hw = CHECKED.get().cloned().unwrap_or_else(Hardware::detect_quick);
    let rec = hardware::recommend(&hw);
    (hw, rec)
}

static CHECKED: OnceLock<Hardware> = OnceLock::new();

/// Ask llama-server for its devices in the background: the first time on a
/// Mac it takes seconds.
fn check_hardware() {
    let _ = thread::Builder::new().name("dualeye-hardware".into()).spawn(|| {
        let _ = CHECKED.set(Hardware::detect());
    });
}

impl Default for VoiceSettings {
    fn default() -> Self {
        let (_, rec) = hardware();
        let voices = ["it", "en"].into_iter().filter_map(|l| models::default_voice(l).map(|v| (l.to_string(), v.to_string()))).collect();
        Self {
            enabled: false,
            model: rec.whisper.into(),
            language: SttLanguage::Auto,
            keep_recordings: false,
            speak: true,
            follow_up: true,
            voices,
            llm: rec.llm.is_some(),
            llm_model: rec.llm.unwrap_or(models::DEFAULT_LLM).into(),
        }
    }
}

/// What the host did about a transcript.
#[derive(Clone, Serialize)]
struct ReplyEntry {
    text: String,
    actions: Vec<String>,
    understood: bool,
    /// `llm` or `rules`.
    by: String,
}

/// One line of the Voice tab's log.
#[derive(Clone, Serialize)]
struct TranscriptEntry {
    id: u8,
    /// Unix time in ms.
    at: u64,
    /// `None`: Whisper heard no words.
    transcript: Option<Transcript>,
    reply: Option<ReplyEntry>,
    spoken: Option<Spoken>,
}

#[derive(Default)]
struct Link {
    kind: &'static str,
    port: Option<String>,
    message: Option<String>,
    last: Option<Snapshot>,
    sent: Option<Snapshot>,
    sent_at: Option<Instant>,
    connected_at: Option<Instant>,
    /// What the board said it runs, since it connected.
    firmware: Option<BoardFirmware>,
    logs: VecDeque<String>,
    /// The latest Claude alert: what it said, why it didn't reach the board, when (Unix ms).
    last_alert: Option<(String, Option<String>, u64)>,
    /// What the board's voice overlay shows (`idle` until it says otherwise).
    voice: Option<String>,
    transcripts: VecDeque<TranscriptEntry>,
}

impl Link {
    fn record(&mut self, event: &BridgeEvent) {
        match event {
            BridgeEvent::Waiting { reason } => {
                self.kind = "searching";
                self.message = Some(reason.clone());
            }
            BridgeEvent::Connected { port } => {
                self.kind = "connected";
                self.port = Some(port.clone());
                self.message = None;
                self.sent = None;
                self.firmware = None;
                self.voice = None;
                self.connected_at = Some(Instant::now());
            }
            BridgeEvent::Snapshot { snapshot, sent } => {
                self.last = Some(snapshot.clone());
                if *sent {
                    self.sent = Some(snapshot.clone());
                    self.sent_at = Some(Instant::now());
                }
            }
            BridgeEvent::BoardLog { line } => {
                if self.logs.len() == LOG_LINES {
                    self.logs.pop_front();
                }
                self.logs.push_back(line.clone());
            }
            BridgeEvent::Firmware { firmware } => self.firmware = Some(firmware.clone()),
            // `AppState` keeps faces and rotation, and the timers.
            BridgeEvent::Settings { .. } | BridgeEvent::Wake { .. } | BridgeEvent::TimerFired { .. } => {}
            BridgeEvent::Transcript { id, transcript } => {
                if self.transcripts.len() == TRANSCRIPTS {
                    self.transcripts.pop_front();
                }
                let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
                self.transcripts.push_back(TranscriptEntry { id: *id, at, transcript: transcript.clone(), reply: None, spoken: None });
            }
            BridgeEvent::Reply { id, text, actions, understood, by, .. } => {
                if let Some(entry) = self.transcripts.iter_mut().rev().find(|e| e.id == *id) {
                    entry.reply = Some(ReplyEntry { text: text.clone(), actions: actions.clone(), understood: *understood, by: by.clone() });
                }
            }
            BridgeEvent::Spoken { id, spoken } => {
                if let Some(entry) = self.transcripts.iter_mut().rev().find(|e| e.id == *id) {
                    entry.spoken = Some(spoken.clone());
                }
            }
            // `AppState::stt_error` keeps the last error.
            BridgeEvent::Listening { .. } | BridgeEvent::Utterance { .. } | BridgeEvent::VoiceError { .. } => {}
            BridgeEvent::VoiceState { state } => self.voice = Some(state.clone()),
            BridgeEvent::ClaudeAlert { text, error, .. } => {
                let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
                self.last_alert = Some((text.clone(), error.clone(), at));
            }
            BridgeEvent::Disconnected { reason, .. } => {
                self.kind = "offline";
                self.message = Some(reason.clone());
            }
        }
    }
}

struct AppState {
    link: Mutex<Link>,
    bridge: Mutex<Option<Bridge>>,
    settings: Mutex<Settings>,
    settings_path: Option<PathBuf>,
    /// Handed to every bridge, so a face change reaches the running one.
    faces: Arc<Mutex<Faces>>,
    /// Likewise for the rotation of each screen.
    rotation: Arc<Mutex<Rotations>>,
    /// Shares the bridge's connection with MCP servers; `None` if it couldn't listen.
    hub: Option<Arc<Hub>>,
    hub_error: Option<String>,
    /// Separate from the bridge's own collector, for the sensor list.
    collector: Mutex<Option<Collector>>,
    /// esptool holds the port (identify or flash in progress).
    device_busy: AtomicBool,
    /// Where esptool's Python and virtualenv live.
    esptool_dir: PathBuf,
    /// Handed to every bridge; `apply_voice` swaps what's in it.
    voice: Arc<Mutex<VoiceConfig>>,
    /// Handed to every bridge, like `faces`.
    claude_alerts: Arc<Mutex<AlertSettings>>,
    /// Handed to every bridge; kept in their own file.
    timers: Arc<Timers>,
    /// Speech-to-text: `off`, `starting`, `ready` or `error`, and why.
    stt_state: Arc<Mutex<(&'static str, Option<String>)>>,
    /// Text-to-speech, likewise.
    tts_state: Arc<Mutex<(&'static str, Option<String>)>>,
    /// The language model, likewise.
    llm_state: Arc<Mutex<(&'static str, Option<String>)>>,
    /// Piper being installed: the latest line of its output.
    piper_install: Mutex<Option<String>>,
    /// The model being downloaded and how far along (percent).
    download: Mutex<Option<(String, f32)>>,
    cancel_download: AtomicBool,
}

impl AppState {
    fn save_settings(&self) {
        let Some(path) = &self.settings_path else { return };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let settings = self.settings.lock().unwrap().clone();
        if let Ok(json) = serde_json::to_string_pretty(&settings) {
            let _ = fs::write(path, json);
        }
    }
}

/// Make the bridge's voice config match the settings: start (or keep)
/// the whisper-server, Piper and llama-server sidecars, or stop them.
fn apply_voice(state: &AppState) {
    let settings = state.settings.lock().unwrap().voice.clone();
    let (current_stt, current_tts, current_agent) = {
        let voice = state.voice.lock().unwrap();
        (voice.stt.clone(), voice.tts.clone(), voice.agent.clone())
    };
    let stt = sidecar(
        settings.enabled.then(|| stt_config(&settings)),
        current_stt.filter(|s| settings.enabled && stt_config(&settings).is_ok_and(|c| s.config().model == c.model && s.config().language == c.language)),
        &state.stt_state,
        |config| Arc::new(Stt::new(config)),
        |stt: &Arc<Stt>| stt.warm_up().map_err(|e| e.to_string()),
    );
    let wanted_tts = (settings.enabled && settings.speak).then(|| tts_config(&settings));
    let keep_tts = current_tts.filter(|t| matches!(&wanted_tts, Some(Ok(c)) if t.config() == c));
    let tts = sidecar(wanted_tts, keep_tts, &state.tts_state, |config| Arc::new(Tts::new(config)), |tts: &Arc<Tts>| tts.warm_up().map_err(|e| e.to_string()));
    let wanted_llm = (settings.enabled && settings.llm).then(|| llm_config(&settings));
    let keep_agent = current_agent.filter(|a| matches!(&wanted_llm, Some(Ok(c)) if a.llm().config() == c));
    let agent = sidecar(
        wanted_llm,
        keep_agent,
        &state.llm_state,
        |config| Arc::new(Agent::new(Arc::new(Llm::new(config)))),
        |agent: &Arc<Agent>| agent.llm().warm_up().map_err(|e| e.to_string()),
    );
    let dump_dir = settings.keep_recordings.then(voice::default_dump_dir).flatten();
    *state.voice.lock().unwrap() = VoiceConfig { dump_dir, stt, tts, agent, follow_up: settings.follow_up };
}

/// One sidecar for [`apply_voice`]: off (`wanted` is `None`), not possible
/// (`Some(Err)`), the running one when `keep` has it, or a new one warmed up
/// in the background. `status` follows.
fn sidecar<C, T: Send + Sync + 'static>(
    wanted: Option<Result<C, String>>,
    keep: Option<Arc<T>>,
    status: &Arc<Mutex<(&'static str, Option<String>)>>,
    make: impl FnOnce(C) -> Arc<T>,
    warm_up: impl FnOnce(&Arc<T>) -> Result<(), String> + Send + 'static,
) -> Option<Arc<T>> {
    match wanted {
        None => {
            *status.lock().unwrap() = ("off", None);
            None
        }
        Some(Err(why)) => {
            *status.lock().unwrap() = ("error", Some(why));
            None
        }
        Some(Ok(_)) if keep.is_some() => keep,
        Some(Ok(config)) => {
            let sidecar = make(config);
            *status.lock().unwrap() = ("starting", None);
            let (warm, status) = (sidecar.clone(), status.clone());
            thread::spawn(move || {
                *status.lock().unwrap() = match warm_up(&warm) {
                    Ok(()) => ("ready", None),
                    Err(e) => ("error", Some(e)),
                }
            });
            Some(sidecar)
        }
    }
}

fn tts_config(settings: &VoiceSettings) -> Result<TtsConfig, String> {
    let python = tts::find_python().ok_or("Piper isn't installed yet")?;
    let voices: BTreeMap<String, String> = settings
        .voices
        .iter()
        .filter(|(_, id)| Model::by_id(id).is_some_and(|m| m.kind == Kind::Voice && m.is_installed()))
        .map(|(l, id)| (l.clone(), id.clone()))
        .collect();
    if voices.is_empty() {
        return Err("no voice downloaded yet".into());
    }
    Ok(TtsConfig { python, voices })
}

fn llm_config(settings: &VoiceSettings) -> Result<LlmConfig, String> {
    let server = llm::find_server().ok_or("llama-server not found: reinstall the app, or install llama.cpp (brew install llama.cpp on macOS)")?;
    let model = Model::by_id(&settings.llm_model).filter(|m| m.kind == Kind::Llm).ok_or_else(|| format!("unknown language model {}", settings.llm_model))?;
    if !model.is_installed() {
        return Err(format!("the {} model isn't downloaded yet", model.id));
    }
    Ok(LlmConfig { server, model: model.path().ok_or("no data folder for the models")?, gpu_layers: None })
}

fn stt_config(settings: &VoiceSettings) -> Result<SttConfig, String> {
    let server = stt::find_server().ok_or("whisper-server not found: reinstall the app, or install whisper.cpp (brew install whisper-cpp on macOS)")?;
    let model = Model::by_id(&settings.model).ok_or_else(|| format!("unknown model {}", settings.model))?;
    if !model.is_installed() {
        return Err(format!("the {} model isn't downloaded yet", model.id));
    }
    Ok(SttConfig { server, model: model.path().ok_or("no data folder for the models")?, language: settings.language })
}

#[derive(Serialize)]
struct ModelInfo {
    id: &'static str,
    kind: Kind,
    language: Option<&'static str>,
    bytes: u64,
    note: &'static str,
    license: &'static str,
    installed: bool,
}

#[derive(Serialize)]
struct VoiceInfo {
    settings: VoiceSettings,
    /// The whisper-server binary, if found.
    server: Option<String>,
    models: Vec<ModelInfo>,
    stt: &'static str,
    stt_error: Option<String>,
    /// Piper's virtualenv interpreter, if installed.
    piper: Option<String>,
    /// While Piper installs: the latest line of its output.
    piper_install: Option<String>,
    tts: &'static str,
    tts_error: Option<String>,
    /// The llama-server binary, if found.
    llm_server: Option<String>,
    llm: &'static str,
    llm_error: Option<String>,
    download: Option<(String, f32)>,
    hardware: Hardware,
    recommendation: Recommendation,
}

fn voice_info_of(state: &AppState) -> VoiceInfo {
    let (hardware, recommendation) = hardware();
    let (stt, stt_error) = state.stt_state.lock().unwrap().clone();
    let (tts, tts_error) = state.tts_state.lock().unwrap().clone();
    let (llm, llm_error) = state.llm_state.lock().unwrap().clone();
    VoiceInfo {
        settings: state.settings.lock().unwrap().voice.clone(),
        server: stt::find_server().map(|p| p.display().to_string()),
        models: models::MODELS
            .iter()
            .map(|m| ModelInfo { id: m.id, kind: m.kind, language: m.language, bytes: m.bytes(), note: m.note, license: m.license, installed: m.is_installed() })
            .collect(),
        stt,
        stt_error,
        piper: tts::find_python().map(|p| p.display().to_string()),
        piper_install: state.piper_install.lock().unwrap().clone(),
        tts,
        tts_error,
        llm_server: llm::find_server().map(|p| p.display().to_string()),
        llm,
        llm_error,
        download: state.download.lock().unwrap().clone(),
        hardware,
        recommendation,
    }
}

#[tauri::command]
fn voice_info(state: State<AppState>) -> VoiceInfo {
    voice_info_of(&state)
}

#[tauri::command]
fn set_voice(state: State<AppState>, settings: VoiceSettings) -> VoiceInfo {
    state.settings.lock().unwrap().voice = settings;
    state.save_settings();
    apply_voice(&state);
    voice_info_of(&state)
}

#[tauri::command]
async fn download_model(app: AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let model = Model::by_id(&id).ok_or_else(|| format!("unknown model {id}"))?;
        {
            let mut download = state.download.lock().unwrap();
            if download.is_some() {
                return Err("another model is downloading".to_string());
            }
            *download = Some((id.clone(), 0.0));
        }
        state.cancel_download.store(false, Ordering::SeqCst);
        let result = model.download(&state.cancel_download, |p| *state.download.lock().unwrap() = Some((id.clone(), p.unwrap_or(0.0))));
        *state.download.lock().unwrap() = None;
        result.map_err(|e| e.to_string())?;
        // The model voice was waiting for.
        apply_voice(&state);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn cancel_download(state: State<AppState>) {
    state.cancel_download.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn delete_model(state: State<AppState>, id: String) -> Result<VoiceInfo, String> {
    let model = Model::by_id(&id).ok_or_else(|| format!("unknown model {id}"))?;
    {
        // Stop the server that has it open first.
        let mut voice = state.voice.lock().unwrap();
        if voice.stt.as_ref().is_some_and(|s| Some(s.config().model.clone()) == model.path()) {
            voice.stt = None;
        }
        if voice.tts.as_ref().is_some_and(|t| t.config().voices.values().any(|v| v == model.id)) {
            voice.tts = None;
        }
        if let Some(agent) = voice.agent.take_if(|a| Some(a.llm().config().model.clone()) == model.path()) {
            agent.llm().shutdown();
        }
    }
    model.remove().map_err(|e| e.to_string())?;
    apply_voice(&state);
    Ok(voice_info_of(&state))
}

/// Set Piper up: a virtualenv made with esptool's Python (downloaded first
/// if the machine has none), then `pip install piper-tts`.
#[tauri::command]
async fn install_piper(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        {
            let mut installing = state.piper_install.lock().unwrap();
            if installing.is_some() {
                return Err("Piper is already being installed".to_string());
            }
            *installing = Some("Getting Python".into());
        }
        let result = setup::python(&state.esptool_dir, |e| {
            if let FlashEvent::Setup { message, percent } = e {
                let line = percent.map_or(message.clone(), |p| format!("{message} {p:.0}%"));
                *state.piper_install.lock().unwrap() = Some(line);
            }
        })
        .and_then(|python| tts::install(&python, |line| *state.piper_install.lock().unwrap() = Some(line)));
        *state.piper_install.lock().unwrap() = None;
        result.map_err(|e| e.to_string())?;
        apply_voice(&state);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn board() -> Board {
    Board::new(None, &format!("dualeye-app/{}", env!("CARGO_PKG_VERSION")))
}

/// The board's own voice settings (it keeps them in NVS); `None` where it
/// doesn't say (`eyes`: a firmware before 1.0.1; `idle_eyes`: before 1.0.2).
#[derive(serde::Serialize)]
struct BoardVoice {
    volume: Option<u8>,
    eyes: Option<bool>,
    idle_eyes: Option<bool>,
}

#[tauri::command]
async fn board_voice() -> Result<BoardVoice, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let state = board().call_tool("get_state", serde_json::json!({})).map_err(|e| e.to_string())?;
        let s = state.structured_content.unwrap_or_default();
        Ok(BoardVoice {
            volume: s.pointer("/audio/volume").and_then(|v| v.as_u64()).map(|v| v as u8),
            eyes: s.pointer("/voice/eyes").and_then(|v| v.as_bool()),
            idle_eyes: s.pointer("/voice/idle_eyes").and_then(|v| v.as_bool()),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

fn call_board(name: &'static str, args: serde_json::Value) -> Result<(), String> {
    let result = board().call_tool(name, args).map_err(|e| e.to_string())?;
    if result.is_error { Err(result.text()) } else { Ok(()) }
}

#[tauri::command]
async fn set_board_volume(percent: u8) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || call_board("set_volume", serde_json::json!({"percent": percent})))
        .await
        .map_err(|e| e.to_string())?
}

/// Animated eyes (or the ring) while the board talks with you.
#[tauri::command]
async fn set_board_eyes(on: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || call_board("set_eyes", serde_json::json!({"on": on})))
        .await
        .map_err(|e| e.to_string())?
}

/// The eyes' short scenes, now and then while nobody is talking.
#[tauri::command]
async fn set_board_idle_eyes(on: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || call_board("set_eyes", serde_json::json!({"idle": on})))
        .await
        .map_err(|e| e.to_string())?
}

fn side(side: &str) -> Result<&'static str, String> {
    match side {
        "left" => Ok("left"),
        "right" => Ok("right"),
        other => Err(format!("unknown screen {other}")),
    }
}

/// Put a picture or a GIF on `side`'s image face, emitting `image`
/// `{side, progress}` as it goes.
#[tauri::command]
async fn send_image(app: AppHandle, side: String, bytes: Vec<u8>) -> Result<media::Prepared, String> {
    let side = self::side(&side)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut last = -1.0f32;
        media::send(&board(), side, &bytes, |p| {
            if p - last >= 0.02 || p >= 1.0 {
                last = p;
                let _ = app.emit("image", serde_json::json!({"side": side, "progress": p}));
            }
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn clear_image(side: String) -> Result<(), String> {
    let side = self::side(&side)?;
    tauri::async_runtime::spawn_blocking(move || media::clear(&board(), side)).await.map_err(|e| e.to_string())?
}

/// What was last sent to `side`, as a data URL for the mirror.
#[tauri::command]
async fn image_preview(side: String) -> Result<Option<String>, String> {
    let side = self::side(&side)?;
    tauri::async_runtime::spawn_blocking(move || media::copy_data_url(side)).await.map_err(|e| e.to_string())
}

/// Say a test sentence through the board, with the voice of `language`.
#[tauri::command]
async fn test_voice(language: String) -> Result<(), String> {
    let text = if language == "it" { "Ciao! Questa è la mia voce." } else { "Hello! This is my voice." };
    tauri::async_runtime::spawn_blocking(move || board().say(text, Some(&language)).map(drop).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
struct Status {
    link: &'static str,
    port: Option<String>,
    message: Option<String>,
    last: Option<Snapshot>,
    sent: Option<Snapshot>,
    sent_age_ms: Option<u64>,
    connected_age_ms: Option<u64>,
    firmware: Option<BoardFirmware>,
    logs: Vec<String>,
    voice: Option<String>,
    transcripts: Vec<TranscriptEntry>,
    port_setting: Option<String>,
    faces: Faces,
    rotation: Rotations,
}

#[tauri::command]
fn status(state: State<AppState>) -> Status {
    let link = state.link.lock().unwrap();
    let age = |t: Option<Instant>| t.map(|t| t.elapsed().as_millis() as u64);
    Status {
        link: link.kind,
        port: link.port.clone(),
        message: link.message.clone(),
        last: link.last.clone(),
        sent: link.sent.clone(),
        sent_age_ms: age(link.sent_at),
        connected_age_ms: age(link.connected_at),
        firmware: link.firmware.clone(),
        logs: link.logs.iter().cloned().collect(),
        voice: link.voice.clone(),
        transcripts: link.transcripts.iter().cloned().collect(),
        port_setting: state.settings.lock().unwrap().port.clone(),
        faces: *state.faces.lock().unwrap(),
        rotation: *state.rotation.lock().unwrap(),
    }
}

#[tauri::command]
fn list_ports() -> Vec<PortInfo> {
    serial::list_ports()
}

#[tauri::command]
async fn set_port(app: AppHandle, port: Option<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state.settings.lock().unwrap().port = port.clone();
        state.save_settings();
        if state.device_busy.load(Ordering::SeqCst) {
            // The bridge comes back with the new setting once esptool is done.
            return;
        }
        // Dropping the old bridge joins its thread and releases the port.
        let old = state.bridge.lock().unwrap().take();
        drop(old);
        *state.link.lock().unwrap() = Link { kind: "searching", ..Default::default() };
        let bridge = start_bridge(&app, port);
        *state.bridge.lock().unwrap() = Some(bridge);
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_faces(state: State<AppState>, faces: Faces) {
    *state.faces.lock().unwrap() = faces;
    state.settings.lock().unwrap().faces = faces;
    state.save_settings();
}

#[tauri::command]
fn set_rotation(state: State<AppState>, rotation: Rotations) {
    *state.rotation.lock().unwrap() = rotation;
    state.settings.lock().unwrap().rotation = rotation;
    state.save_settings();
}

#[tauri::command]
async fn readings(app: AppHandle) -> Result<Vec<Reading>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut slot = state.collector.lock().unwrap();
        let collector = slot.get_or_insert_with(Collector::new);
        collector.readings()
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn power_helper_status() -> power_helper::HelperStatus {
    tauri::async_runtime::spawn_blocking(power_helper::status).await.unwrap_or(power_helper::HelperStatus {
        state: power_helper::HelperState::Unavailable,
        error: None,
    })
}

#[tauri::command]
async fn set_power_helper(on: bool) -> Result<power_helper::HelperStatus, String> {
    tauri::async_runtime::spawn_blocking(move || power_helper::set(on)).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn open_power_helper_settings() {
    power_helper::open_settings();
}

#[derive(Serialize)]
struct FirmwareInfo {
    size: usize,
    /// The bundled image's app descriptor: its version is what the board gets when flashed.
    bundled: Option<ImageInfo>,
    /// `None` until esptool has been set up (done on first identify/flash).
    esptool: Option<Esptool>,
}

#[tauri::command]
async fn firmware_info(app: AppHandle) -> Result<FirmwareInfo, String> {
    let dir = app.state::<AppState>().esptool_dir.clone();
    let esptool = tauri::async_runtime::spawn_blocking(move || Esptool::installed(&dir)).await.map_err(|e| e.to_string())?;
    Ok(FirmwareInfo { size: FIRMWARE.len(), bundled: firmware::image_info(FIRMWARE), esptool })
}

#[tauri::command]
async fn identify_board(app: AppHandle, port: Option<String>) -> Result<ChipInfo, String> {
    with_device(app, port, |app, tool, port| tool.chip_info(port, |e| emit_flash(app, e))).await
}

#[tauri::command]
async fn flash_board(app: AppHandle, port: Option<String>) -> Result<(), String> {
    with_device(app, port, |app, tool, port| {
        let path = std::env::temp_dir().join("dualeye-merged-binary.bin");
        fs::write(&path, FIRMWARE)?;
        let result = tool.flash(port, &path, |e| emit_flash(app, e));
        let _ = fs::remove_file(&path);
        result
    })
    .await
}

#[derive(Serialize)]
struct McpInfo {
    /// What MCP clients run: this executable with `args`.
    command: Option<String>,
    args: Vec<&'static str>,
    hub: Option<HubStatus>,
    hub_error: Option<String>,
}

#[tauri::command]
fn mcp_info(state: State<AppState>) -> McpInfo {
    McpInfo {
        command: std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()),
        args: vec![mcp::FLAG],
        hub: state.hub.as_ref().map(|h| h.status()),
        hub_error: state.hub_error.clone(),
    }
}

#[tauri::command]
fn claude_link() -> LinkStatus {
    statusline::status()
}

/// Make this binary Claude Code's status line, chaining to the one it replaces.
#[tauri::command]
fn claude_connect() -> Result<LinkStatus, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    statusline::connect(&exe).map_err(|e| e.to_string())
}

#[tauri::command]
fn claude_disconnect() -> Result<LinkStatus, String> {
    statusline::disconnect().map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct LastAlert {
    text: String,
    error: Option<String>,
    age_s: u64,
}

#[derive(Serialize)]
struct ClaudeAlertsInfo {
    settings: AlertSettings,
    hooks: HooksStatus,
    /// The bridge can speak (voice on with spoken replies, and Piper ready).
    can_speak: bool,
    last: Option<LastAlert>,
}

fn claude_alerts_info_of(state: &AppState) -> ClaudeAlertsInfo {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let last = state.link.lock().unwrap().last_alert.clone();
    ClaudeAlertsInfo {
        settings: state.claude_alerts.lock().unwrap().clone(),
        hooks: hooks::status(),
        can_speak: state.voice.lock().unwrap().tts.is_some(),
        last: last.map(|(text, error, at)| LastAlert { text, error, age_s: now.saturating_sub(at) / 1000 }),
    }
}

#[tauri::command]
fn claude_alerts_info(state: State<AppState>) -> ClaudeAlertsInfo {
    claude_alerts_info_of(&state)
}

#[tauri::command]
fn set_claude_alerts(state: State<AppState>, settings: AlertSettings) -> ClaudeAlertsInfo {
    *state.claude_alerts.lock().unwrap() = settings.clone();
    state.settings.lock().unwrap().claude_alerts = settings;
    state.save_settings();
    claude_alerts_info_of(&state)
}

/// Add this binary to Claude Code's hooks, or take it out.
#[tauri::command]
fn claude_hooks(state: State<AppState>, connect: bool) -> Result<ClaudeAlertsInfo, String> {
    if connect {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        hooks::connect(&exe).map_err(|e| e.to_string())?;
    } else {
        hooks::disconnect().map_err(|e| e.to_string())?;
    }
    Ok(claude_alerts_info_of(&state))
}

/// Give a sample "Claude needs you" the way a hook would.
#[tauri::command]
async fn test_claude_alert() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
        // A session of its own, so it isn't taken for a repeat.
        let event = serde_json::json!({
            "hook_event_name": "Notification",
            "notification_type": "permission_prompt",
            "session_id": format!("dualeye-test-{now}"),
            "cwd": "/DualEye",
        });
        board().claude_hook(event).map(drop).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The Timers tab: what's running and where it shows.
#[derive(Serialize)]
struct TimersInfo {
    timers: Vec<TimerInfo>,
    pomodoro: Option<Pomodoro>,
    show_on: ShowOn,
    /// The bridge says what a timer was for when it's up.
    can_speak: bool,
}

fn timers_info_of(state: &AppState) -> TimersInfo {
    let t = &state.timers;
    TimersInfo { timers: t.list(), pomodoro: t.pomodoro(), show_on: t.show_on(), can_speak: state.voice.lock().unwrap().tts.is_some() }
}

#[tauri::command]
fn timers_info(state: State<AppState>) -> TimersInfo {
    timers_info_of(&state)
}

/// Run one of `Timers::tools` (`set_timer`, `control_timer`...), in the alerts' language.
#[tauri::command]
fn timer_tool(state: State<AppState>, name: String, arguments: serde_json::Value) -> Result<TimersInfo, String> {
    let language = state.claude_alerts.lock().unwrap().language.clone();
    state.timers.call_tool(&name, &arguments, &language).ok_or_else(|| format!("unknown timer tool {name}"))??;
    Ok(timers_info_of(&state))
}

#[tauri::command]
fn set_timer_screen(state: State<AppState>, show_on: ShowOn) -> TimersInfo {
    state.timers.set_show_on(show_on);
    timers_info_of(&state)
}

/// Silence whatever rings.
#[tauri::command]
fn dismiss_timers(state: State<AppState>) -> TimersInfo {
    state.timers.dismiss();
    timers_info_of(&state)
}

fn emit_flash(app: &AppHandle, event: FlashEvent) {
    let _ = app.emit("flash", &event);
}

/// Set esptool up if needed, stop the bridge, hand the port to esptool, then
/// start the bridge again.
async fn with_device<T: Send + 'static>(
    app: AppHandle,
    port: Option<String>,
    job: impl FnOnce(&AppHandle, &Esptool, &str) -> std::io::Result<T> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if state.device_busy.swap(true, Ordering::SeqCst) {
            return Err("esptool is already talking to the board".to_string());
        }
        let result = (|| -> Result<T, String> {
            // Streaming carries on while Python and esptool download.
            let tool = setup::ensure(&state.esptool_dir, |e| emit_flash(&app, e)).map_err(|e| format!("setting up esptool: {e}"))?;
            let port = port
                .or_else(|| state.settings.lock().unwrap().port.clone())
                .or_else(serial::detect_board)
                .ok_or("no single DualEye found on USB: pick its port")?;
            let old = state.bridge.lock().unwrap().take();
            drop(old);
            let paused = BridgeEvent::Waiting { reason: "esptool is using the port".into() };
            if let Some(hub) = &state.hub {
                hub.set_unavailable("esptool is using the port (the DualEye app is identifying or flashing the board)");
            }
            let mut link = Link { kind: "searching", ..Default::default() };
            link.record(&paused);
            *state.link.lock().unwrap() = link;
            let _ = app.emit("bridge", &paused);
            let result = job(&app, &tool, &port).map_err(|e| e.to_string());
            let bridge = start_bridge(&app, state.settings.lock().unwrap().port.clone());
            *state.bridge.lock().unwrap() = Some(bridge);
            result
        })();
        state.device_busy.store(false, Ordering::SeqCst);
        result
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|result| result)
}

fn start_bridge(app: &AppHandle, port: Option<String>) -> Bridge {
    let handle = app.clone();
    let state = app.state::<AppState>();
    let (faces, rotation, hub, voice) = (state.faces.clone(), state.rotation.clone(), state.hub.clone(), state.voice.clone());
    let (claude_alerts, timers) = (state.claude_alerts.clone(), state.timers.clone());
    Bridge::spawn(BridgeConfig { port, faces, rotation, hub, voice, claude_alerts, timers, ..Default::default() }, move |event| {
        if let Some(state) = handle.try_state::<AppState>() {
            state.link.lock().unwrap().record(&event);
            if let BridgeEvent::VoiceError { message } = &event {
                // Which sidecar failed: Piper's and llama.cpp's errors say so.
                let status = if message.starts_with("piper") {
                    &state.tts_state
                } else if message.starts_with("llama-server") {
                    &state.llm_state
                } else {
                    &state.stt_state
                };
                *status.lock().unwrap() = ("error", Some(message.clone()));
            }
            // Faces and rotation came from the board (at connect, or an MCP
            // client changed them): keep them for the next launch too.
            if let BridgeEvent::Settings { faces, rotation } = &event {
                let mut settings = state.settings.lock().unwrap();
                settings.faces = *faces;
                settings.rotation = *rotation;
                drop(settings);
                state.save_settings();
            }
        }
        if let BridgeEvent::Snapshot { snapshot, .. } = &event {
            update_tray(&handle, snapshot);
        }
        let _ = handle.emit("bridge", &event);
    })
}

fn update_tray(app: &AppHandle, s: &Snapshot) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let temp = |t: Option<f32>| t.map_or_else(|| "—".to_string(), |t| format!("{t:.0}°"));
    let text = format!("CPU {} · GPU {}", temp(s.cpu.temp_c), temp(s.gpu.temp_c));
    let _ = tray.set_tooltip(Some(format!("DualEye — {text}")));
}

fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show DualEye", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("DualEye")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    instances::wait_for_predecessor();
    tauri::Builder::default()
        // Closing the window leaves the app in the tray; launching it again must
        // not start a second bridge fighting over the serial port.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !instances::on_relaunch(app, args.first().map(String::as_str)) {
                show_main(app);
            }
        }))
        .setup(|app| {
            // We hold the single instance now; older copies must let go of the port.
            instances::stop_strays();
            check_hardware();
            if let Some(me) = instances::SelfBinary::capture(app.handle()) {
                app.manage(me);
            }
            instances::watch_for_updates(app.handle().clone());
            if instances::start_hidden() {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
            }
            let settings_path = app.path().app_config_dir().ok().map(|d| d.join("settings.json"));
            let esptool_dir = app.path().app_local_data_dir()?.join("esptool");
            let settings: Settings = settings_path
                .as_ref()
                .and_then(|p| fs::read_to_string(p).ok())
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let port = settings.port.clone();
            let faces = Arc::new(Mutex::new(settings.faces));
            let rotation = Arc::new(Mutex::new(settings.rotation));
            let claude_alerts = Arc::new(Mutex::new(settings.claude_alerts.clone()));
            let (hub, hub_error) = match Hub::start() {
                Ok(hub) => (Some(hub), None),
                Err(e) => (None, Some(e.to_string())),
            };
            app.manage(AppState {
                link: Mutex::new(Link { kind: "searching", ..Default::default() }),
                bridge: Mutex::new(None),
                settings: Mutex::new(settings),
                settings_path,
                faces,
                rotation,
                hub,
                hub_error,
                collector: Mutex::new(None),
                device_busy: AtomicBool::new(false),
                esptool_dir,
                voice: Arc::default(),
                claude_alerts,
                timers: Arc::new(Timers::open(timers::default_file())),
                stt_state: Arc::new(Mutex::new(("off", None))),
                tts_state: Arc::new(Mutex::new(("off", None))),
                llm_state: Arc::new(Mutex::new(("off", None))),
                piper_install: Mutex::new(None),
                download: Mutex::new(None),
                cancel_download: AtomicBool::new(false),
            });
            apply_voice(&app.state::<AppState>());
            let handle = app.handle();
            // A logout or `kill` quits like the tray's Quit, so the port is
            // released and the whisper-server sidecar stopped.
            let quitter = handle.clone();
            let _ = ctrlc::set_handler(move || quitter.exit(0));
            let bridge = start_bridge(handle, port);
            *app.state::<AppState>().bridge.lock().unwrap() = Some(bridge);
            build_tray(handle)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Keep streaming to the board when the window is closed.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![status, list_ports, set_port, set_faces, set_rotation, readings, firmware_info, identify_board, flash_board, claude_link, claude_connect, claude_disconnect, claude_alerts_info, set_claude_alerts, claude_hooks, test_claude_alert, mcp_info, voice_info, set_voice, download_model, cancel_download, delete_model, install_piper, board_voice, set_board_volume, set_board_eyes, set_board_idle_eyes, test_voice, send_image, clear_image, image_preview, power_helper_status, set_power_helper, open_power_helper_settings, timers_info, timer_tool, set_timer_screen, dismiss_timers])
        .build(tauri::generate_context!())
        .expect("failed to build the DualEye app")
        .run(|app, event| match event {
            tauri::RunEvent::Exit => {
                // Release the serial port before the process goes away.
                if let Some(state) = app.try_state::<AppState>() {
                    state.bridge.lock().unwrap().take();
                    // Stops the whisper-server, Piper and llama-server sidecars,
                    // even if a reply being spoken still holds them.
                    let mut voice = state.voice.lock().unwrap();
                    voice.stt.take().inspect(|s| s.shutdown());
                    voice.tts.take().inspect(|t| t.shutdown());
                    voice.agent.take().inspect(|a| a.llm().shutdown());
                    drop(voice);
                    state.cancel_download.store(true, Ordering::SeqCst);
                    if let Some(hub) = &state.hub {
                        hub.set_unavailable("the DualEye app is quitting");
                    }
                }
            }
            // macOS: the Dock icon, or opening the app while it runs.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                if !instances::on_relaunch(app, None) {
                    show_main(app);
                }
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The update offer compares the board against this; a merged image without it never offers one.
    #[test]
    fn bundled_firmware_reports_its_version() {
        let info = firmware::image_info(FIRMWARE).expect("app descriptor in build/merged-binary.bin");
        let want = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../version.txt")).trim();
        assert_eq!(info.version, want, "rebuild the firmware: idf.py build merge-bin");
    }
}
