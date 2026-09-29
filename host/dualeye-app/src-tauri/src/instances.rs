//! One DualEye at a time, and always the newest one.
//!
//! The single-instance plugin hands a second launch to the process already
//! running, which would keep the old build on the port after an update. So the
//! running instance watches its own executable: once an installer has replaced
//! it (or the user launches a different copy) it starts the new binary and
//! exits. The successor waits for it to be gone before claiming the instance,
//! then terminates any leftover copies from builds that predate the plugin.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use dualeye_core::claude::statusline;
use dualeye_core::mcp;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind};
use tauri::{AppHandle, Manager};

/// Set on the successor: the pid it must outlive.
const WAIT_PID_ENV: &str = "DUALEYE_WAIT_PID";
/// Set on the successor when the old window was hidden in the tray.
const HIDDEN_ENV: &str = "DUALEYE_START_HIDDEN";
const CHECK_EVERY: Duration = Duration::from_secs(5);
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);

/// The executable this process started from, and what it looked like then.
pub struct SelfBinary {
    path: PathBuf,
    stamp: Option<Stamp>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    modified: SystemTime,
    len: u64,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = fs::metadata(path).ok()?;
    Some(Stamp { modified: meta.modified().ok()?, len: meta.len() })
}

impl SelfBinary {
    pub fn capture(app: &AppHandle) -> Option<Self> {
        let path = tauri::process::current_binary(&app.env()).ok()?;
        let stamp = stamp(&path);
        Some(Self { path, stamp })
    }

    /// The file on disk is no longer the one we run (an update was installed).
    /// A missing file means an installer is mid-way, so not yet.
    fn replaced(&self) -> Option<Stamp> {
        let now = stamp(&self.path)?;
        (Some(now) != self.stamp).then_some(now)
    }
}

/// Before the Tauri builder runs: if a predecessor handed over to us, wait for
/// it to release the single-instance lock and the serial port.
pub fn wait_for_predecessor() {
    let Some(pid) = std::env::var(WAIT_PID_ENV).ok().and_then(|p| p.parse::<usize>().ok()) else { return };
    let pid = Pid::from(pid);
    let mut sys = System::new();
    let deadline = Instant::now() + EXIT_TIMEOUT;
    while Instant::now() < deadline {
        sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        if sys.process(pid).is_none() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// The successor starts hidden when the old window was.
pub fn start_hidden() -> bool {
    std::env::var_os(HIDDEN_ENV).is_some()
}

/// Terminate other processes of this app that are not the single instance:
/// copies from builds without the plugin, which would hold the port forever.
/// The MCP server and the status line helper run the same binary; they are
/// Claude Code's, and go through the running app rather than the port.
pub fn stop_strays() {
    let Ok(me) = std::env::current_exe() else { return };
    let Some(name) = me.file_name().map(|n| n.to_owned()) else { return };
    let own = sysinfo::get_current_pid().ok();
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_exe(UpdateKind::Always).with_cmd(UpdateKind::Always));
    let strays: Vec<Pid> = sys
        .processes()
        .iter()
        .filter(|(pid, p)| Some(**pid) != own && p.exe().and_then(|e| e.file_name()) == Some(name.as_os_str()))
        .filter(|(_, p)| !p.cmd().iter().any(|arg| arg == mcp::FLAG || arg == statusline::FLAG))
        .map(|(pid, p)| {
            if p.kill_with(Signal::Term).is_none() {
                p.kill();
            }
            *pid
        })
        .collect();
    if strays.is_empty() {
        return;
    }
    // Give them a moment to close the port; the bridge retries anyway.
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        sys.refresh_processes(ProcessesToUpdate::Some(&strays), true);
        if strays.iter().all(|pid| sys.process(*pid).is_none()) {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Start `binary` as our successor and exit.
fn hand_over(app: &AppHandle, binary: &Path) {
    let hidden = app.get_webview_window("main").is_some_and(|w| !w.is_visible().unwrap_or(true));
    let mut cmd = Command::new(binary);
    cmd.env(WAIT_PID_ENV, std::process::id().to_string());
    if hidden {
        cmd.env(HIDDEN_ENV, "1");
    }
    match cmd.spawn() {
        Ok(_) => app.exit(0),
        Err(e) => eprintln!("dualeye: could not start {}: {e}", binary.display()),
    }
}

/// Another launch reached us (single-instance callback) or macOS reopened the
/// app. Returns `false` when this instance stays, so the caller shows it.
pub fn on_relaunch(app: &AppHandle, launched: Option<&str>) -> bool {
    let Some(me) = app.try_state::<SelfBinary>() else { return false };
    if me.replaced().is_some() {
        hand_over(app, &me.path);
        return true;
    }
    // A different copy was started by its full path: that is the one wanted.
    let other = launched.map(PathBuf::from).filter(|p| p.is_absolute() && p.is_file());
    if let Some(other) = other {
        let same = match (fs::canonicalize(&other), fs::canonicalize(&me.path)) {
            (Ok(a), Ok(b)) => a == b,
            _ => true,
        };
        if !same {
            hand_over(app, &other);
            return true;
        }
    }
    false
}

/// Restart into an update as soon as it is installed, even if nobody relaunches
/// the app: the replaced file must look the same on two checks in a row, so a
/// half-written binary is never started.
pub fn watch_for_updates(app: AppHandle) {
    // `tauri dev` rebuilds and restarts the binary itself.
    if cfg!(debug_assertions) {
        return;
    }
    thread::spawn(move || {
        let mut pending = None;
        loop {
            thread::sleep(CHECK_EVERY);
            let Some(me) = app.try_state::<SelfBinary>() else { continue };
            match me.replaced() {
                Some(now) if pending == Some(now) => {
                    hand_over(&app, &me.path);
                    return;
                }
                now => pending = now,
            }
        }
    });
}
