//! The Claude Code status line hook-up.
//!
//! [`connect`] points `statusLine` in Claude Code's settings.json at
//! `<exe> --claude-statusline`. Claude Code then runs that on every update with
//! its status JSON on stdin; [`run`] keeps a copy for [`ClaudeUsage`] and prints
//! the status line: the user's previous command's, which keeps working
//! through us, or a short default. [`disconnect`] puts the previous one back.
//!
//! [`ClaudeUsage`]: super::ClaudeUsage

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use serde::Serialize;
use serde_json::{Map, Value};

use super::{config_dir, data_dir, status_file};

/// The argument that turns the app binary into the status line helper.
pub const FLAG: &str = "--claude-statusline";

#[derive(Debug, Clone, Serialize)]
pub struct LinkStatus {
    /// settings.json has our status line.
    pub connected: bool,
    /// The status line that was there before, still shown through ours.
    pub chained: Option<String>,
    /// Seconds since Claude Code last ran the helper.
    pub last_update_s: Option<u64>,
    pub settings_path: Option<PathBuf>,
}

pub fn status() -> LinkStatus {
    Paths::default_locations().status()
}

/// Install `exe` as Claude Code's status line, keeping whatever was there to
/// chain to and a one-off backup of settings.json.
pub fn connect(exe: &Path) -> io::Result<LinkStatus> {
    Paths::default_locations().connect(exe)
}

/// Put back the status line that was there before [`connect`], if any.
pub fn disconnect() -> io::Result<LinkStatus> {
    Paths::default_locations().disconnect()
}

/// Claude Code's settings.json and the files the helper keeps.
struct Paths {
    settings: Option<PathBuf>,
    previous: Option<PathBuf>,
    status: Option<PathBuf>,
}

impl Paths {
    fn default_locations() -> Self {
        Self {
            settings: config_dir().map(|d| d.join("settings.json")),
            previous: previous_path(),
            status: status_file(),
        }
    }

    fn status(&self) -> LinkStatus {
        let connected =
            self.settings.as_deref().and_then(read_settings).is_some_and(|s| is_ours(s.get("statusLine")));
        let last_update_s = self
            .status
            .as_ref()
            .and_then(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .map(|d| d.as_secs());
        let chained = self.previous().as_ref().and_then(command_of).map(str::to_string);
        LinkStatus { connected, chained, last_update_s, settings_path: self.settings.clone() }
    }

    fn previous(&self) -> Option<Value> {
        serde_json::from_str(&fs::read_to_string(self.previous.as_ref()?).ok()?).ok()
    }

    fn settings_path(&self) -> io::Result<&Path> {
        self.settings.as_deref().ok_or_else(|| io::Error::other("can't find the Claude Code config folder"))
    }

    fn connect(&self, exe: &Path) -> io::Result<LinkStatus> {
        let path = self.settings_path()?;
        let mut settings = match fs::read_to_string(path) {
            Ok(text) => parse_object(&text)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Map::new(),
            Err(e) => return Err(e),
        };
        let current = settings.get("statusLine").cloned();
        if !is_ours(current.as_ref()) {
            let prev = self.previous.as_deref().ok_or_else(|| io::Error::other("no data folder"))?;
            match &current {
                Some(line) => write_atomic(prev, serde_json::to_string_pretty(line)?.as_bytes())?,
                None => remove_if_exists(prev)?,
            }
            backup(path)?;
        }
        let mut line = Map::new();
        line.insert("type".into(), "command".into());
        line.insert("command".into(), format!("\"{}\" {FLAG}", exe.display()).into());
        if let Some(padding) = current.as_ref().and_then(|c| c.get("padding")) {
            line.insert("padding".into(), padding.clone());
        }
        settings.insert("statusLine".into(), Value::Object(line));
        write_settings(path, &settings)?;
        Ok(self.status())
    }

    fn disconnect(&self) -> io::Result<LinkStatus> {
        let path = self.settings_path()?;
        if let Ok(text) = fs::read_to_string(path) {
            let mut settings = parse_object(&text)?;
            if is_ours(settings.get("statusLine")) {
                match self.previous() {
                    Some(prev) => settings.insert("statusLine".into(), prev),
                    None => settings.remove("statusLine"),
                };
                write_settings(path, &settings)?;
            }
        }
        if let Some(prev) = &self.previous {
            remove_if_exists(prev)?;
        }
        Ok(self.status())
    }
}

/// The helper itself: save the status JSON from `input`, then print a status
/// line to `out`. Never fails loudly; Claude Code would show the error.
pub fn run(mut input: impl Read, mut out: impl Write) {
    let mut json = Vec::new();
    if input.read_to_end(&mut json).is_err() {
        return;
    }
    if let Some(path) = status_file() {
        let _ = write_atomic(&path, &json);
    }
    let text = match Paths::default_locations().previous().as_ref().and_then(command_of) {
        Some(cmd) => run_previous(cmd, &json).unwrap_or_default(),
        None => default_line(&json),
    };
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

fn run_previous(cmd: &str, json: &[u8]) -> Option<String> {
    let mut child = if cfg!(windows) {
        Command::new("cmd").args(["/C", cmd]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()
    } else {
        Command::new("sh").args(["-c", cmd]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()
    }
    .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(json);
    }
    let output = child.wait_with_output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `Opus 5.5 · 5h 42% · 7d 18%`, with whatever the JSON has.
fn default_line(json: &[u8]) -> String {
    let Ok(v) = serde_json::from_slice::<Value>(json) else { return String::new() };
    let mut parts = Vec::new();
    if let Some(name) = v.pointer("/model/display_name").and_then(Value::as_str) {
        parts.push(name.to_string());
    }
    for (label, key) in [("5h", "five_hour"), ("7d", "seven_day")] {
        if let Some(pct) = v.pointer(&format!("/rate_limits/{key}/used_percentage")).and_then(Value::as_f64) {
            parts.push(format!("{label} {pct:.0}%"));
        }
    }
    parts.join(" · ")
}

fn previous_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("claude-statusline-previous.json"))
}

fn command_of(line: &Value) -> Option<&str> {
    line.get("command").and_then(Value::as_str).filter(|c| !c.contains(FLAG))
}

fn is_ours(line: Option<&Value>) -> bool {
    line.and_then(|l| l.get("command")).and_then(Value::as_str).is_some_and(|c| c.contains(FLAG))
}

pub(super) fn read_settings(path: &Path) -> Option<Map<String, Value>> {
    parse_object(&fs::read_to_string(path).ok()?).ok()
}

pub(super) fn parse_object(text: &str) -> io::Result<Map<String, Value>> {
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str(text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(io::Error::other("settings.json is not a JSON object")),
        Err(e) => Err(io::Error::other(format!("settings.json is not valid JSON ({e}); fix it first"))),
    }
}

pub(super) fn write_settings(path: &Path, settings: &Map<String, Value>) -> io::Result<()> {
    let mut text = serde_json::to_string_pretty(settings)?;
    text.push('\n');
    write_atomic(path, text.as_bytes())
}

/// settings.json as it was before DualEye first touched it.
pub(super) fn backup(path: &Path) -> io::Result<()> {
    let backup = path.with_extension("json.dualeye-backup");
    if path.exists() && !backup.exists() {
        fs::copy(path, backup)?;
    }
    Ok(())
}

pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_line_shows_model_and_limits() {
        let json = br#"{"model":{"display_name":"Opus 5.5"},"rate_limits":{"five_hour":{"used_percentage":42.4},"seven_day":{"used_percentage":18}}}"#;
        assert_eq!(default_line(json), "Opus 5.5 · 5h 42% · 7d 18%");
        assert_eq!(default_line(br#"{"model":{"display_name":"Opus 5.5"}}"#), "Opus 5.5");
    }

    #[test]
    fn connect_chains_and_disconnect_restores() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("claude/settings.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        fs::write(&settings, r#"{"model":"opus","statusLine":{"type":"command","command":"~/bin/line.sh","padding":1}}"#)
            .unwrap();
        let paths = Paths {
            settings: Some(settings.clone()),
            previous: Some(dir.path().join("dualeye/prev.json")),
            status: Some(dir.path().join("dualeye/status.json")),
        };

        let link = paths.connect(Path::new("/opt/DualEye/dualeye-app")).unwrap();
        assert!(link.connected);
        assert_eq!(link.chained.as_deref(), Some("~/bin/line.sh"));
        let now: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(now["model"], "opus");
        assert_eq!(now["statusLine"]["command"], "\"/opt/DualEye/dualeye-app\" --claude-statusline");
        assert_eq!(now["statusLine"]["padding"], 1);
        assert!(settings.with_extension("json.dualeye-backup").exists());

        // Connecting again (a moved app) must not chain to ourselves.
        let link = paths.connect(Path::new("/Applications/DualEye.app/dualeye-app")).unwrap();
        assert_eq!(link.chained.as_deref(), Some("~/bin/line.sh"));

        let link = paths.disconnect().unwrap();
        assert!(!link.connected);
        let back: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(back["statusLine"]["command"], "~/bin/line.sh");
        assert_eq!(link.chained, None);
    }

    #[test]
    fn connect_without_settings_file_and_disconnect_removes() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let paths = Paths { settings: Some(settings.clone()), previous: Some(dir.path().join("prev.json")), status: None };
        assert!(paths.connect(Path::new("/x/dualeye-app")).unwrap().connected);
        paths.disconnect().unwrap();
        let back: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert!(back.get("statusLine").is_none());
    }

    #[test]
    fn recognises_our_command() {
        let ours = serde_json::json!({"type": "command", "command": "\"/Applications/DualEye.app/Contents/MacOS/dualeye-app\" --claude-statusline"});
        let theirs = serde_json::json!({"type": "command", "command": "~/.claude/statusline.sh"});
        assert!(is_ours(Some(&ours)));
        assert!(!is_ours(Some(&theirs)));
        assert_eq!(command_of(&theirs), Some("~/.claude/statusline.sh"));
        assert_eq!(command_of(&ours), None);
    }
}
