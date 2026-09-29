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

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;

use crate::claude::ClaudeUsage;
use crate::firmware::{self, BoardFirmware};
use crate::hub::Hub;
use crate::link::{CallError, Link, LinkEvent};
use crate::protocol::Channel;
use crate::sensors::Collector;
use crate::serial;
use crate::snapshot::{Face, Faces, Rotation, Rotations, Snapshot};

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
    /// Faces or rotation came from the board: at the handshake, or because a
    /// hub client changed them. `faces` and `rotation` in the config hold the
    /// new values already.
    Settings { faces: Faces, rotation: Rotations },
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
        let result = session(&port, config, stop, &mut collector, &mut claude, &on_event);
        if let Some(hub) = &config.hub {
            let why = result.as_ref().err().map_or_else(|| "the bridge stopped".to_string(), |e| format!("board disconnected: {e}"));
            hub.set_unavailable(&why);
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

fn session(
    port: &str,
    config: &BridgeConfig,
    stop: &AtomicBool,
    collector: &mut Collector,
    claude: &mut ClaudeUsage,
    on_event: &EventSink,
) -> io::Result<()> {
    let heard = Arc::new(Heard::default());
    let link = Arc::new({
        let heard = heard.clone();
        let sink = on_event.clone();
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
            LinkEvent::Notification { .. } | LinkEvent::Closed(_) => {}
        })?
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
                    }
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

        if ready && config.hub.as_ref().is_some_and(|h| h.take_settings_changed()) {
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
        if let Some(hub) = &config.hub {
            hub.set_snapshot(&snapshot);
        }
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
