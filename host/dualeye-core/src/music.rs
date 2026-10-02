//! What's playing on this computer, for the music face (firmware 1.3).
//!
//! [`now_playing`] asks the OS: Spotify and Apple Music over AppleScript on
//! macOS, the system's media session on Windows (any app that shows in the
//! volume flyout: Spotify, browsers, ...), MPRIS through `playerctl` on Linux.
//! [`control`] plays, pauses and skips the same way.
//!
//! [`Music`] polls while a screen shows the face ([`Music::want`]), and keeps
//! the cover ready for the board: cropped to the middle square, 240 × 240
//! RGB565, sent with `music/art` (see [`upload_cover`]) and named in the
//! snapshot's `music.art` ([`BoardMusic`]).

use std::fmt;
use std::io::Cursor;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::link::{CallError, Tool};
use crate::media;

/// Between two looks while the face is on.
const POLL: Duration = Duration::from_secs(2);
/// After the last [`Music::want`], polling goes on this long.
const WANT_FOR: Duration = Duration::from_secs(10);
/// The board's fonts are ASCII; titles and artists are cut to its buffers.
const TITLE_MAX: usize = 63;
const ARTIST_MAX: usize = 47;
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(2);

/// A track playing or paused.
#[derive(Clone, PartialEq, Serialize)]
pub struct NowPlaying {
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Seconds into the track when it was read.
    pub position_s: Option<f64>,
    pub duration_s: Option<f64>,
    /// Spotify, Music, or the app the OS names.
    pub player: String,
    #[serde(skip)]
    cover: Option<CoverSource>,
    #[serde(skip)]
    read_at: Instant,
}

impl fmt::Debug for NowPlaying {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NowPlaying")
            .field("playing", &self.playing)
            .field("title", &self.title)
            .field("artist", &self.artist)
            .field("player", &self.player)
            .finish_non_exhaustive()
    }
}

/// Where the cover of a track can be had, once it changes. Each OS uses some.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
enum CoverSource {
    /// `https://` (Spotify) or `file://` (MPRIS).
    Url(String),
    /// Ask Apple Music for its artwork.
    AppleMusic,
    /// The thumbnail of Windows' current media session.
    WindowsSession,
}

impl NowPlaying {
    /// The position now: counted on from when it was read while playing.
    pub fn position_now(&self) -> Option<f64> {
        let pos = self.position_s? + if self.playing { self.read_at.elapsed().as_secs_f64() } else { 0.0 };
        Some(self.duration_s.filter(|d| *d > 0.0).map_or(pos, |d| pos.min(d)))
    }

    /// Same track (and so the same cover) as `other`.
    fn same_track(&self, other: &NowPlaying) -> bool {
        (&self.player, &self.title, &self.artist, &self.album, &self.cover) == (&other.player, &other.title, &other.artist, &other.album, &other.cover)
    }

    /// What `now_playing` and the voice say about it.
    pub fn to_json(&self) -> Value {
        json!({
            "state": if self.playing { "playing" } else { "paused" },
            "title": self.title,
            "artist": self.artist,
            "album": self.album,
            "player": self.player,
            "position_s": self.position_now().map(|p| p.round()),
            "duration_s": self.duration_s.map(f64::round),
        })
    }
}

/// The track for the board's music face; absent when nothing plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardMusic {
    /// `play` or `pause`.
    pub state: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub artist: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos_s: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dur_s: Option<f32>,
    /// The cover sent with `music/art`, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art: Option<u32>,
}

/// A cover ready for the board and the app's mirror.
#[derive(Clone)]
pub struct Cover {
    /// Never 0: the board's "no cover".
    pub id: u32,
    /// 240 × 240 RGB565, little-endian: what `music/art` sends.
    pub rgb565: Arc<Vec<u8>>,
    /// The same square as a PNG `data:` URL, for a webview.
    pub data_url: Arc<String>,
}

impl fmt::Debug for Cover {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cover").field("id", &self.id).finish_non_exhaustive()
    }
}

/// What [`control`] does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
}

impl std::str::FromStr for Control {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "play" | "resume" => Ok(Control::Play),
            "pause" | "stop" => Ok(Control::Pause),
            "toggle" | "play_pause" => Ok(Control::Toggle),
            "next" | "skip" => Ok(Control::Next),
            "previous" | "back" => Ok(Control::Previous),
            _ => Err(format!("unknown action `{s}`, expected play, pause, toggle, next or previous")),
        }
    }
}

/// ASCII the board's fonts have: accents dropped, quotes and dashes plain,
/// anything else left out, at most `max` characters.
pub fn board_text(text: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let plain: &str = match c {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => "a",
            'À' | 'Á' | 'Â' | 'Ä' | 'Ã' | 'Å' => "A",
            'è' | 'é' | 'ê' | 'ë' => "e",
            'È' | 'É' | 'Ê' | 'Ë' => "E",
            'ì' | 'í' | 'î' | 'ï' => "i",
            'Ì' | 'Í' | 'Î' | 'Ï' => "I",
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' | 'ø' => "o",
            'Ò' | 'Ó' | 'Ô' | 'Ö' | 'Õ' | 'Ø' => "O",
            'ù' | 'ú' | 'û' | 'ü' => "u",
            'Ù' | 'Ú' | 'Û' | 'Ü' => "U",
            'ñ' => "n",
            'Ñ' => "N",
            'ç' => "c",
            'Ç' => "C",
            'ß' => "ss",
            'æ' => "ae",
            'Æ' => "AE",
            'œ' => "oe",
            '‘' | '’' | '´' | '`' => "'",
            '“' | '”' | '«' | '»' => "\"",
            '–' | '—' | '‐' => "-",
            '…' => "...",
            '×' => "x",
            c if c.is_whitespace() => " ",
            c if (' '..='~').contains(&c) => {
                out.push(c);
                continue;
            }
            _ => "",
        };
        out.push_str(plain);
    }
    let words = out.split_whitespace().collect::<Vec<_>>().join(" ");
    words.chars().take(max).collect()
}

impl NowPlaying {
    /// The snapshot's `music`, with `art` when that cover is ready.
    pub fn board_view(&self, art: Option<u32>) -> BoardMusic {
        let title = board_text(&self.title, TITLE_MAX);
        BoardMusic {
            state: if self.playing { "play" } else { "pause" }.into(),
            title: if title.is_empty() { "Unknown".into() } else { title },
            artist: board_text(&self.artist, ARTIST_MAX),
            pos_s: self.position_now().map(crate::snapshot::round1),
            dur_s: self.duration_s.filter(|d| *d > 0.0).map(crate::snapshot::round1),
            art,
        }
    }
}

/// Polls what's playing while someone looks, with its cover; shared by the
/// bridge (the board's face), the voice and the MCP server (controls).
#[derive(Default)]
pub struct Music {
    state: Mutex<State>,
    wake: Condvar,
    started: OnceLock<()>,
}

#[derive(Default)]
struct State {
    now: Option<NowPlaying>,
    cover: Option<Cover>,
    /// The track the cover is for, `None` until one was looked for.
    cover_of: Option<NowPlaying>,
    wanted_until: Option<Instant>,
    /// Look again at once (after a control).
    poke: bool,
}

impl fmt::Debug for Music {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Music").field("now", &self.state.lock().unwrap().now).finish_non_exhaustive()
    }
}

impl Music {
    /// Keep looking for the next little while: a screen shows the face, or
    /// the app's mirror does.
    pub fn want(self: &Arc<Self>) {
        let mut state = self.state.lock().unwrap();
        let first = state.wanted_until.is_none_or(|t| t < Instant::now());
        state.wanted_until = Some(Instant::now() + WANT_FOR);
        drop(state);
        if first {
            self.wake.notify_all();
        }
        self.started.get_or_init(|| {
            let me = self.clone();
            let _ = thread::Builder::new().name("dualeye-music".into()).spawn(move || me.run());
        });
    }

    /// What was playing at the last look.
    pub fn now(&self) -> Option<NowPlaying> {
        self.state.lock().unwrap().now.clone()
    }

    /// The cover of what's playing, once it's ready.
    pub fn cover(&self) -> Option<Cover> {
        let state = self.state.lock().unwrap();
        let now = state.now.as_ref()?;
        state.cover_of.as_ref().filter(|t| t.same_track(now))?;
        state.cover.clone()
    }

    /// The snapshot's `music`, naming the cover when it's ready.
    pub fn board_view(&self) -> Option<BoardMusic> {
        let art = self.cover().map(|c| c.id);
        Some(self.now()?.board_view(art))
    }

    /// Look now, on this thread: for a question about what's playing.
    pub fn refresh(&self) -> Option<NowPlaying> {
        let now = now_playing();
        self.state.lock().unwrap().now = now.clone();
        now
    }

    /// Play, pause or skip, then look again soon.
    pub fn control(&self, action: Control) -> Result<String, String> {
        let current = self.now().or_else(now_playing);
        let player = control(action, current.as_ref())?;
        self.state.lock().unwrap().poke = true;
        self.wake.notify_all();
        Ok(player)
    }

    fn run(&self) {
        let mut probe = Probe::default();
        loop {
            {
                let mut state = self.state.lock().unwrap();
                while state.wanted_until.is_none_or(|t| t < Instant::now()) && !state.poke {
                    state = self.wake.wait(state).unwrap();
                }
                state.poke = false;
            }
            let now = probe.now_playing();
            let stale = {
                let state = self.state.lock().unwrap();
                match (&now, &state.cover_of) {
                    (Some(n), Some(of)) => !n.same_track(of),
                    (Some(_), None) => true,
                    (None, _) => false,
                }
            };
            if stale {
                let n = now.clone().expect("a track");
                let cover = fetch_cover(&n).and_then(|bytes| prepare_cover(&bytes).ok());
                let mut state = self.state.lock().unwrap();
                state.cover = cover;
                state.cover_of = Some(n);
            }
            let mut state = self.state.lock().unwrap();
            state.now = now;
            // Sooner after a control.
            drop(self.wake.wait_timeout_while(state, POLL, |s| !s.poke).unwrap());
        }
    }

    /// The host's music tools, for the voice and MCP.
    pub fn tools() -> Vec<Tool> {
        vec![
            Tool {
                name: "media_control".into(),
                description: "Play, pause or skip the music playing on this computer (Spotify, Apple Music, a browser...).".into(),
                input_schema: json!({"type": "object", "properties": {
                    "action": {"type": "string", "enum": ["play", "pause", "toggle", "next", "previous"],
                               "description": "play or resume, pause, toggle between them, next track, previous track"}
                }, "required": ["action"]}),
            },
            Tool {
                name: "now_playing".into(),
                description: "What music is playing on this computer: title, artist, album, player, position.".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
        ]
    }

    /// Run one of [`Music::tools`]; `None` for another tool.
    pub fn call_tool(&self, name: &str, args: &Value) -> Option<Result<String, String>> {
        match name {
            "media_control" => Some((|| {
                let action: Control = args["action"].as_str().unwrap_or("toggle").parse()?;
                let player = self.control(action)?;
                let did = match action {
                    Control::Play => "playing",
                    Control::Pause => "paused",
                    Control::Toggle => "toggled",
                    Control::Next => "skipped to the next track",
                    Control::Previous => "back to the previous track",
                };
                Ok(format!("{player}: {did}"))
            })()),
            "now_playing" => Some(Ok(self.refresh().map_or_else(|| json!({"state": "nothing playing"}), |n| n.to_json()).to_string())),
            _ => None,
        }
    }
}

/// Send `cover` for the music face with `call` (a JSON-RPC request to the board).
pub fn upload_cover(call: impl Fn(&str, Value, Duration) -> Result<Value, CallError>, cover: &Cover) -> Result<(), CallError> {
    let engine = base64::engine::general_purpose::STANDARD;
    for (i, part) in cover.rgb565.chunks(ART_CHUNK).enumerate() {
        call("music/art", json!({"id": cover.id, "offset": i * ART_CHUNK, "data": engine.encode(part)}), UPLOAD_TIMEOUT)?;
    }
    Ok(())
}

/// `ART_CHUNK` in `main/art.h`.
const ART_CHUNK: usize = 2880;

/// Decode a cover and make the board's square of it.
fn prepare_cover(bytes: &[u8]) -> Result<Cover, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
    let square = media::square(&img);
    let rgb565: Vec<u8> = media::rgb565(&square).iter().flat_map(|p| p.to_le_bytes()).collect();
    let mut png = Vec::new();
    square.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let data_url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png));
    // The board takes it as a JSON number: keep it well inside 2^31, and never 0.
    let id = (crc32fast::hash(&rgb565) & 0x3FFF_FFFF).max(1);
    Ok(Cover { id, rgb565: Arc::new(rgb565), data_url: Arc::new(data_url) })
}

fn fetch_cover(n: &NowPlaying) -> Option<Vec<u8>> {
    match n.cover.as_ref()? {
        CoverSource::Url(url) => fetch_url(url),
        CoverSource::AppleMusic => apple_music_cover(),
        CoverSource::WindowsSession => windows_cover(),
    }
}

fn fetch_url(url: &str) -> Option<Vec<u8>> {
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read(percent_decode(path)).ok();
    }
    #[cfg(feature = "download")]
    if url.starts_with("https://") || url.starts_with("http://") {
        let mut response = ureq::get(url).call().ok()?;
        return response.body_mut().with_config().limit(8 << 20).read_to_vec().ok();
    }
    None
}

/// `%20` and the like in a `file://` URL.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A number as AppleScript or `playerctl` prints it; a decimal comma too.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn number(s: &str) -> Option<f64> {
    s.trim().replace(',', ".").parse().ok().filter(|v: &f64| v.is_finite() && *v >= 0.0)
}

/// What's playing right now, if anything is.
pub fn now_playing() -> Option<NowPlaying> {
    Probe::default().now_playing()
}

/// Plays, pauses or skips on `current`'s player (or the one playing);
/// returns the player's name.
pub fn control(action: Control, current: Option<&NowPlaying>) -> Result<String, String> {
    platform::control(action, current)
}

/// Keeps what asking takes between looks (the process list on macOS).
#[derive(Default)]
struct Probe {
    #[cfg(target_os = "macos")]
    system: Option<sysinfo::System>,
}

impl Probe {
    fn now_playing(&mut self) -> Option<NowPlaying> {
        platform::now_playing(self)
    }
}

fn apple_music_cover() -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    return platform::apple_music_cover();
    #[cfg(not(target_os = "macos"))]
    None
}

fn windows_cover() -> Option<Vec<u8>> {
    #[cfg(target_os = "windows")]
    return platform::windows_cover();
    #[cfg(not(target_os = "windows"))]
    None
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::OsStr;
    use std::process::Command;
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    /// The apps asked, by process name, as AppleScript knows them.
    const PLAYERS: &[&str] = &["Spotify", "Music"];

    /// Run an AppleScript; its output, or `None` when it failed.
    fn osascript(script: &str) -> Option<String> {
        let out = Command::new("/usr/bin/osascript").arg("-e").arg(script).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
    }

    /// The players running: only those are asked, so AppleScript never
    /// starts one, nor asks where an app that isn't installed is.
    fn running(probe: &mut Probe) -> Vec<&'static str> {
        let system = probe.system.get_or_insert_with(System::new);
        system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
        PLAYERS.iter().copied().filter(|p| system.processes_by_exact_name(OsStr::new(p)).next().is_some()).collect()
    }

    /// One tab-separated line per player that has a track: name, state,
    /// title, artist, album, position (s), duration (Spotify: ms), artwork URL.
    fn script(players: &[&str]) -> String {
        let mut s = String::from("set res to \"\"\n");
        for p in players {
            let art = if *p == "Spotify" { "& tab & (artwork url of t)" } else { "" };
            s += &format!(
                "tell application \"{p}\"\n\
                 \tset pstate to player state as string\n\
                 \tif pstate is \"playing\" or pstate is \"paused\" then\n\
                 \t\tset t to current track\n\
                 \t\tset res to res & \"{p}\" & tab & pstate & tab & (name of t) & tab & (artist of t) & tab & (album of t) & tab & (player position as string) & tab & ((duration of t) as string) {art} & linefeed\n\
                 \tend if\n\
                 end tell\n"
            );
        }
        s + "return res"
    }

    fn parse(line: &str) -> Option<NowPlaying> {
        let f: Vec<&str> = line.split('\t').collect();
        let (&player, &state) = (f.first()?, f.get(1)?);
        let duration = f.get(6).and_then(|d| number(d)).map(|d| if player == "Spotify" { d / 1000.0 } else { d });
        let cover = match player {
            "Spotify" => f.get(7).filter(|u| u.starts_with("http")).map(|u| CoverSource::Url(u.to_string())),
            _ => Some(CoverSource::AppleMusic),
        };
        Some(NowPlaying {
            playing: state == "playing",
            title: f.get(2)?.to_string(),
            artist: f.get(3).unwrap_or(&"").to_string(),
            album: f.get(4).unwrap_or(&"").to_string(),
            position_s: f.get(5).and_then(|p| number(p)),
            duration_s: duration,
            player: if player == "Music" { "Apple Music".into() } else { player.into() },
            cover,
            read_at: Instant::now(),
        })
    }

    pub(super) fn now_playing(probe: &mut Probe) -> Option<NowPlaying> {
        let players = running(probe);
        if players.is_empty() {
            return None;
        }
        let out = osascript(&script(&players))?;
        let tracks: Vec<NowPlaying> = out.lines().filter_map(parse).collect();
        // The one playing, else the first paused.
        let first = tracks.iter().position(|t| t.playing).unwrap_or(0);
        tracks.into_iter().nth(first)
    }

    pub(super) fn apple_music_cover() -> Option<Vec<u8>> {
        // Printed as «data JPEG FFD8FF…»: the type, then the bytes in hex.
        let out = osascript("tell application \"Music\" to get raw data of artwork 1 of current track")?;
        let start = out.find("«data ")? + "«data ".len();
        let hex = out[start..].get(4..)?.split('»').next()?;
        (0..hex.len() / 2).map(|i| u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()).collect()
    }

    pub(super) fn control(action: Control, current: Option<&NowPlaying>) -> Result<String, String> {
        let player = match current.map(|c| c.player.as_str()) {
            Some("Apple Music") => "Music",
            Some("Spotify") => "Spotify",
            _ => *running(&mut Probe::default()).first().ok_or("no music player is open (Spotify or Apple Music)")?,
        };
        let verb = match action {
            Control::Play => "play",
            Control::Pause => "pause",
            Control::Toggle => "playpause",
            Control::Next => "next track",
            Control::Previous => "previous track",
        };
        osascript(&format!("tell application \"{player}\" to {verb}")).ok_or_else(|| format!("{player} didn't answer"))?;
        Ok(if player == "Music" { "Apple Music".into() } else { player.into() })
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };
    use windows::Storage::Streams::DataReader;

    /// 100 ns ticks, as WinRT counts.
    const TICKS_PER_S: f64 = 10_000_000.0;
    /// From 1601 (WinRT's epoch) to 1970, in ticks.
    const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

    fn session() -> windows::core::Result<Session> {
        Manager::RequestAsync()?.get()?.GetCurrentSession()
    }

    fn read(session: &Session) -> windows::core::Result<Option<NowPlaying>> {
        let status = session.GetPlaybackInfo()?.PlaybackStatus()?;
        if status != Status::Playing && status != Status::Paused {
            return Ok(None);
        }
        let playing = status == Status::Playing;
        let props = session.TryGetMediaPropertiesAsync()?.get()?;
        let timeline = session.GetTimelineProperties()?;
        let start = timeline.StartTime()?.Duration;
        let end = timeline.EndTime()?.Duration;
        let mut position = (timeline.Position()?.Duration - start) as f64 / TICKS_PER_S;
        // The position is as of the last update the app gave.
        if playing {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as i64 / 100) + UNIX_EPOCH_TICKS;
            let updated = timeline.LastUpdatedTime()?.UniversalTime;
            if updated > 0 && now > updated {
                position += (now - updated) as f64 / TICKS_PER_S;
            }
        }
        let duration = (end - start) as f64 / TICKS_PER_S;
        let app = session.SourceAppUserModelId()?.to_string();
        Ok(Some(NowPlaying {
            playing,
            title: props.Title()?.to_string(),
            artist: props.Artist()?.to_string(),
            album: props.AlbumTitle()?.to_string(),
            position_s: (duration > 0.0).then_some(position.max(0.0)),
            duration_s: (duration > 0.0).then_some(duration),
            player: app_name(&app),
            cover: props.Thumbnail().is_ok().then_some(CoverSource::WindowsSession),
            read_at: Instant::now(),
        }))
    }

    /// "Spotify.exe", "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic" → a name to say.
    fn app_name(id: &str) -> String {
        let base = id.rsplit(['\\', '!']).next().unwrap_or(id);
        let base = base.trim_end_matches(".exe").trim_end_matches(".EXE");
        match base.to_ascii_lowercase().as_str() {
            "spotify" => "Spotify".into(),
            "chrome" => "Chrome".into(),
            "msedge" => "Edge".into(),
            "firefox" => "Firefox".into(),
            "microsoft.zunemusic" => "Media Player".into(),
            _ => base.rsplit('.').next().unwrap_or(base).to_string(),
        }
    }

    pub(super) fn now_playing(_probe: &mut Probe) -> Option<NowPlaying> {
        read(&session().ok()?).ok().flatten()
    }

    pub(super) fn windows_cover() -> Option<Vec<u8>> {
        let read = || -> windows::core::Result<Vec<u8>> {
            let props = session()?.TryGetMediaPropertiesAsync()?.get()?;
            let stream = props.Thumbnail()?.OpenReadAsync()?.get()?;
            let size = stream.Size()? as u32;
            let reader = DataReader::CreateDataReader(&stream)?;
            reader.LoadAsync(size)?.get()?;
            let mut bytes = vec![0u8; size as usize];
            reader.ReadBytes(&mut bytes)?;
            Ok(bytes)
        };
        read().ok().filter(|b| !b.is_empty())
    }

    pub(super) fn control(action: Control, _current: Option<&NowPlaying>) -> Result<String, String> {
        let session = session().map_err(|_| "nothing is playing".to_string())?;
        let done = match action {
            Control::Play => session.TryPlayAsync(),
            Control::Pause => session.TryPauseAsync(),
            Control::Toggle => session.TryTogglePlayPauseAsync(),
            Control::Next => session.TrySkipNextAsync(),
            Control::Previous => session.TrySkipPreviousAsync(),
        }
        .and_then(|op| op.get())
        .map_err(|e| e.to_string())?;
        let app = app_name(&session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default());
        if done { Ok(app) } else { Err(format!("{app} didn't take it")) }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::process::Command;

    const FORMAT: &str = "{{status}}\t{{xesam:title}}\t{{xesam:artist}}\t{{xesam:album}}\t{{position}}\t{{mpris:length}}\t{{mpris:artUrl}}\t{{playerName}}";

    fn playerctl(args: &[&str]) -> Result<String, String> {
        let out = Command::new("playerctl")
            .args(args)
            .output()
            .map_err(|_| "playerctl isn't installed: the music face needs it on Linux".to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
    }

    pub(super) fn now_playing(_probe: &mut Probe) -> Option<NowPlaying> {
        let out = playerctl(&["metadata", "--format", FORMAT]).ok()?;
        let f: Vec<&str> = out.lines().next()?.split('\t').collect();
        let state = *f.first()?;
        if state != "Playing" && state != "Paused" {
            return None;
        }
        // Microseconds.
        let us = |i: usize| f.get(i).and_then(|v| number(v)).map(|v| v / 1e6);
        let duration = us(5).filter(|d| *d > 0.0);
        let name = f.get(7).unwrap_or(&"");
        let mut chars = name.chars();
        let player = chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect());
        Some(NowPlaying {
            playing: state == "Playing",
            title: f.get(1)?.to_string(),
            artist: f.get(2).unwrap_or(&"").to_string(),
            album: f.get(3).unwrap_or(&"").to_string(),
            position_s: us(4),
            duration_s: duration,
            player,
            cover: f.get(6).filter(|u| !u.is_empty()).map(|u| CoverSource::Url(u.to_string())),
            read_at: Instant::now(),
        })
    }

    pub(super) fn control(action: Control, current: Option<&NowPlaying>) -> Result<String, String> {
        let verb = match action {
            Control::Play => "play",
            Control::Pause => "pause",
            Control::Toggle => "play-pause",
            Control::Next => "next",
            Control::Previous => "previous",
        };
        playerctl(&[verb])?;
        Ok(current.map_or_else(|| "the player".into(), |c| c.player.clone()))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod platform {
    use super::*;

    pub(super) fn now_playing(_probe: &mut Probe) -> Option<NowPlaying> {
        None
    }

    pub(super) fn control(_action: Control, _current: Option<&NowPlaying>) -> Result<String, String> {
        Err("music controls aren't available on this system".into())
    }
}

/// A track for other modules' tests.
#[cfg(test)]
pub(crate) fn tests_track() -> NowPlaying {
    NowPlaying {
        playing: true,
        title: "Zitti e buoni".into(),
        artist: "Måneskin".into(),
        album: String::new(),
        position_s: None,
        duration_s: None,
        player: "Spotify".into(),
        cover: None,
        read_at: Instant::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, playing: bool) -> NowPlaying {
        NowPlaying {
            playing,
            title: title.into(),
            artist: "Måneskin".into(),
            album: "Teatro d’ira".into(),
            position_s: Some(61.24),
            duration_s: Some(200.0),
            player: "Spotify".into(),
            cover: None,
            read_at: Instant::now(),
        }
    }

    #[test]
    fn board_text_is_ascii() {
        assert_eq!(board_text("Zitti e buoni", 63), "Zitti e buoni");
        assert_eq!(board_text("Perché  l’amore — è “così”…", 63), "Perche l'amore - e \"cosi\"...");
        assert_eq!(board_text("Björk 夜", 63), "Bjork");
        assert_eq!(board_text("abcdef", 3), "abc");
    }

    #[test]
    fn board_view_on_the_wire() {
        let view = track("Zitti e buoni", false).board_view(Some(42));
        assert_eq!(
            serde_json::to_string(&view).unwrap(),
            "{\"state\":\"pause\",\"title\":\"Zitti e buoni\",\"artist\":\"Maneskin\",\"pos_s\":61.2,\"dur_s\":200.0,\"art\":42}"
        );
        let mut silent = track("", true);
        silent.artist.clear();
        silent.position_s = None;
        silent.duration_s = None;
        assert_eq!(serde_json::to_string(&silent.board_view(None)).unwrap(), "{\"state\":\"play\",\"title\":\"Unknown\"}");
    }

    #[test]
    fn position_stops_at_the_end() {
        let mut t = track("x", true);
        t.position_s = Some(199.9);
        t.read_at = Instant::now() - Duration::from_secs(5);
        assert_eq!(t.position_now(), Some(200.0));
        t.playing = false;
        assert_eq!(t.position_now(), Some(199.9));
    }

    #[test]
    fn covers_are_board_squares() {
        let img = image::RgbaImage::from_pixel(640, 480, image::Rgba([0, 0, 255, 255]));
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let cover = prepare_cover(&png).unwrap();
        assert_eq!(cover.rgb565.len(), 240 * 240 * 2);
        assert_eq!(&cover.rgb565[..2], &0x001Fu16.to_le_bytes());
        assert!(cover.id > 0 && cover.id < 1 << 30);
        assert!(cover.data_url.starts_with("data:image/png;base64,"));
        assert_eq!(cover.rgb565.len() % ART_CHUNK, 0);
    }

    #[test]
    fn controls_parse() {
        assert_eq!("next".parse::<Control>(), Ok(Control::Next));
        assert_eq!("resume".parse::<Control>(), Ok(Control::Play));
        assert!("louder".parse::<Control>().is_err());
        assert_eq!(number("12,5"), Some(12.5));
        assert_eq!(percent_decode("/home/me/My%20Cover.jpg"), "/home/me/My Cover.jpg");
    }
}
