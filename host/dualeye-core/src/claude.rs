//! Claude Code usage for the `claude` and `clawd` faces, from two local sources:
//!
//! - Transcripts, `<config>/projects/<project>/**/*.jsonl`: every assistant
//!   message carries its token usage, and a file being written means Claude is
//!   working. The format is internal to Claude Code, so it is read leniently
//!   and anything unexpected is skipped.
//! - The status line: once connected (see [`statusline`]), Claude Code pipes
//!   its documented status JSON to `dualeye-app --claude-statusline`, which
//!   keeps the latest copy in [`status_file`]. It carries the plan's 5-hour and
//!   weekly usage, which the transcripts can't tell.
//!
//! [`alerts`] tells the person at the desk when Claude needs them, finished
//! or is running out of its limits, with Claude Code's [`hooks`].

pub mod alerts;
pub mod hooks;
pub mod statusline;

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local, TimeZone};
use serde::{Deserialize, Serialize};

/// Claude Code's usage window.
const BLOCK_SECS: i64 = 5 * 3600;
/// Entries older than this can't be in the current window or today.
const KEEP_SECS: i64 = 26 * 3600;
/// A transcript or the status line written this recently: Claude is working.
const WORKING_SECS: u64 = 20;
/// Past this without activity Claude is asleep rather than idle.
const SLEEP_SECS: u64 = 30 * 60;
const RESCAN: Duration = Duration::from_secs(2);

/// What the board shows on the Claude faces.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClaudeMetrics {
    /// Tokens in the current 5-hour window: input, output and cache writes.
    /// Cache reads are left out: they are cheap and would swamp the figure.
    pub tok: u64,
    /// The same since local midnight.
    pub today: u64,
    /// Minutes until the 5-hour window resets; absent when none is open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_min: Option<u32>,
    /// Share of the plan's 5-hour limit used, from the status line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s_pct: Option<f32>,
    /// Share of the plan's weekly limit used, from the status line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w_pct: Option<f32>,
    pub state: ClaudeState,
    /// Latest model, short and upper case, e.g. `OPUS 5.5`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaudeState {
    /// A session wrote to its transcript in the last few seconds.
    Work,
    Idle,
    /// Nothing for half an hour.
    #[default]
    Sleep,
}

/// Where Claude Code keeps its settings and transcripts.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    home_dir().map(|h| h.join(".claude"))
}

/// Where the status line helper leaves Claude Code's latest status JSON.
pub fn status_file() -> Option<PathBuf> {
    data_dir().map(|d| d.join("claude-status.json"))
}

/// DualEye's own folder for the status line files.
pub(crate) fn data_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home_dir().map(|h| h.join("Library/Application Support"))
    } else if cfg!(windows) {
        env("APPDATA")
    } else {
        env("XDG_CONFIG_HOME").or_else(|| home_dir().map(|h| h.join(".config")))
    };
    base.map(|b| b.join("dualeye"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).filter(|h| !h.is_empty()).map(PathBuf::from)
}

#[derive(Debug)]
struct Entry {
    ts: i64,
    tokens: u64,
    /// Message and request id, to count a message once: Claude Code writes
    /// one line per content block, each repeating the usage.
    key: Option<String>,
}

#[derive(Debug, Default)]
struct Tracked {
    offset: u64,
    modified: Option<SystemTime>,
}

/// Rate limits from the status line; percentages go back to 0 once their
/// window has reset.
#[derive(Debug, Default, Clone, Copy)]
struct Limits {
    five_hour: Option<(f32, i64)>,
    seven_day: Option<(f32, i64)>,
}

pub struct ClaudeUsage {
    projects: Option<PathBuf>,
    status_path: Option<PathBuf>,
    files: HashMap<PathBuf, Tracked>,
    entries: Vec<Entry>,
    seen: HashSet<String>,
    /// Model of the latest main-thread message, with its time.
    model: Option<(i64, String)>,
    status_model: Option<String>,
    limits: Limits,
    status_modified: Option<SystemTime>,
    last_write: Option<SystemTime>,
    last_scan: Option<Instant>,
}

impl Default for ClaudeUsage {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeUsage {
    pub fn new() -> Self {
        Self::with_paths(config_dir().map(|d| d.join("projects")), status_file())
    }

    pub fn with_paths(projects: Option<PathBuf>, status_path: Option<PathBuf>) -> Self {
        Self {
            projects,
            status_path,
            files: HashMap::new(),
            entries: Vec::new(),
            seen: HashSet::new(),
            model: None,
            status_model: None,
            limits: Limits::default(),
            status_modified: None,
            last_write: None,
            last_scan: None,
        }
    }

    /// `None` until Claude Code has left anything to read on this machine.
    pub fn sample(&mut self) -> Option<ClaudeMetrics> {
        if self.last_scan.is_none_or(|t| t.elapsed() >= RESCAN) {
            self.last_scan = Some(Instant::now());
            self.scan();
            self.read_status();
        }
        if self.files.is_empty() && self.status_modified.is_none() {
            return None;
        }
        Some(self.metrics(SystemTime::now()))
    }

    fn metrics(&self, now: SystemTime) -> ClaudeMetrics {
        let now_s = unix(now);
        let midnight = local_midnight(now_s);
        let today = self.entries.iter().filter(|e| e.ts >= midnight).map(|e| e.tokens).sum();

        // The status line knows when the window resets; otherwise work it out
        // the way Claude Code does.
        let window = match self.limits.five_hour {
            Some((_, reset)) if reset > now_s => Some((reset - BLOCK_SECS, reset)),
            _ => current_block(&self.entries, now_s),
        };
        let (tok, left_min) = match window {
            Some((start, end)) => {
                let tok = self.entries.iter().filter(|e| e.ts >= start && e.ts <= now_s).map(|e| e.tokens).sum();
                (tok, Some(((end - now_s).max(0) / 60) as u32))
            }
            None => (0, None),
        };
        let pct = |limit: Option<(f32, i64)>| limit.map(|(pct, reset)| if reset > now_s { pct } else { 0.0 });

        let idle = self.last_write.and_then(|t| now.duration_since(t).ok()).map(|d| d.as_secs());
        let state = match idle {
            Some(s) if s <= WORKING_SECS => ClaudeState::Work,
            Some(s) if s <= SLEEP_SECS => ClaudeState::Idle,
            // A write "from the future" (clock skew): treat as fresh.
            None if self.last_write.is_some() => ClaudeState::Work,
            _ => ClaudeState::Sleep,
        };

        ClaudeMetrics {
            tok,
            today,
            left_min,
            s_pct: pct(self.limits.five_hour),
            w_pct: pct(self.limits.seven_day),
            state,
            model: self.status_model.clone().or_else(|| self.model.as_ref().map(|(_, m)| m.clone())),
        }
    }

    fn scan(&mut self) {
        let Some(root) = self.projects.clone() else { return };
        let cutoff = SystemTime::now() - Duration::from_secs(KEEP_SECS as u64);
        let mut paths = Vec::new();
        collect_jsonl(&root, 0, &mut paths);
        for path in paths {
            let Ok(meta) = fs::metadata(&path) else { continue };
            let modified = meta.modified().ok();
            if let Some(m) = modified {
                self.last_write = self.last_write.max(Some(m));
            }
            let tracked = self.files.contains_key(&path);
            if !tracked && modified.is_some_and(|m| m < cutoff) {
                continue;
            }
            let state = self.files.entry(path.clone()).or_default();
            if state.modified == modified && state.offset == meta.len() {
                continue;
            }
            if meta.len() < state.offset {
                state.offset = 0;
            }
            state.modified = modified;
            let offset = state.offset;
            let (consumed, lines) = read_new_lines(&path, offset);
            if let Some(state) = self.files.get_mut(&path) {
                state.offset = offset + consumed;
            }
            for line in lines {
                self.ingest(&line);
            }
        }
        self.prune(unix(SystemTime::now()) - KEEP_SECS);
    }

    fn ingest(&mut self, line: &str) {
        // Most lines are tool output or user text; skip them before parsing.
        if !line.contains("\"usage\"") || !line.contains("\"assistant\"") {
            return;
        }
        let Ok(parsed) = serde_json::from_str::<Line>(line) else { return };
        let Some(msg) = parsed.message else { return };
        let (Some(usage), Some(ts)) = (msg.usage, parsed.timestamp.as_deref().and_then(parse_ts)) else { return };
        let key = match (&msg.id, &parsed.request_id) {
            (None, None) => None,
            (id, req) => Some(format!("{}:{}", id.as_deref().unwrap_or(""), req.as_deref().unwrap_or(""))),
        };
        if let Some(k) = &key
            && !self.seen.insert(k.clone())
        {
            return;
        }
        // Subagents often run a smaller model; name the one the user talks to.
        if !parsed.is_sidechain
            && self.model.as_ref().is_none_or(|(t, _)| ts >= *t)
            && let Some(model) = msg.model.as_deref().and_then(short_model)
        {
            self.model = Some((ts, model));
        }
        let tokens = usage.input_tokens + usage.output_tokens + usage.cache_creation_input_tokens;
        self.entries.push(Entry { ts, tokens, key });
    }

    fn prune(&mut self, oldest: i64) {
        if self.entries.iter().all(|e| e.ts >= oldest) {
            return;
        }
        self.entries.retain(|e| e.ts >= oldest);
        self.seen = self.entries.iter().filter_map(|e| e.key.clone()).collect();
    }

    fn read_status(&mut self) {
        let Some(path) = &self.status_path else { return };
        let Some(modified) = fs::metadata(path).and_then(|m| m.modified()).ok() else { return };
        self.last_write = self.last_write.max(Some(modified));
        if self.status_modified == Some(modified) {
            return;
        }
        self.status_modified = Some(modified);
        let Ok(text) = fs::read_to_string(path) else { return };
        let Ok(status) = serde_json::from_str::<StatusJson>(&text) else { return };
        let limit = |w: Option<Window>| w.and_then(|w| Some((w.used_percentage?, w.resets_at?)));
        if let Some(limits) = status.rate_limits {
            self.limits = Limits { five_hour: limit(limits.five_hour), seven_day: limit(limits.seven_day) };
        }
        let model = status.model.and_then(|m| m.id.as_deref().and_then(short_model).or(m.display_name.map(|n| n.to_uppercase())));
        if model.is_some() {
            self.status_model = model;
        }
    }
}

/// Project folders hold `<session>.jsonl`, and subagents' transcripts one or
/// two levels further down.
fn collect_jsonl(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() && depth < 3 {
            collect_jsonl(&path, depth + 1, out);
        } else if kind.is_file() && path.extension().is_some_and(|e| e == "jsonl") {
            out.push(path);
        }
    }
}

/// Complete lines from `offset` on, and how many bytes they took; a line
/// still being written is left for the next scan.
fn read_new_lines(path: &Path, offset: u64) -> (u64, Vec<String>) {
    let mut buf = Vec::new();
    let read = File::open(path).and_then(|mut f| {
        f.seek(SeekFrom::Start(offset))?;
        f.read_to_end(&mut buf)
    });
    if read.is_err() {
        return (0, Vec::new());
    }
    let Some(end) = buf.iter().rposition(|&b| b == b'\n') else { return (0, Vec::new()) };
    let lines = buf[..end].split(|&b| b == b'\n').map(|l| String::from_utf8_lossy(l).into_owned()).collect();
    (end as u64 + 1, lines)
}

/// The open 5-hour window, as Claude Code counts it: it starts at the hour of
/// the first message after a quiet spell and runs five hours from there.
fn current_block(entries: &[Entry], now: i64) -> Option<(i64, i64)> {
    let mut times: Vec<i64> = entries.iter().map(|e| e.ts).filter(|&t| t <= now).collect();
    times.sort_unstable();
    let mut block: Option<(i64, i64)> = None;
    let mut last = i64::MIN;
    for t in times {
        let open = block.is_some_and(|(_, end)| t < end) && t - last < BLOCK_SECS;
        if !open {
            let start = t - t.rem_euclid(3600);
            block = Some((start, start + BLOCK_SECS));
        }
        last = t;
    }
    block.filter(|&(_, end)| now < end)
}

/// `claude-opus-5-5` → `OPUS 5.5`, `claude-sonnet-4-5-20250929` → `SONNET 4.5`.
fn short_model(id: &str) -> Option<String> {
    let rest = id.strip_prefix("claude-")?;
    let mut parts = rest.split('-');
    let family = parts.next().filter(|f| f.chars().all(|c| c.is_ascii_alphabetic()) && !f.is_empty())?;
    let version: Vec<&str> =
        parts.take_while(|p| p.len() <= 2 && !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())).collect();
    let mut name = family.to_uppercase();
    if !version.is_empty() {
        name.push(' ');
        name.push_str(&version.join("."));
    }
    Some(name)
}

fn parse_ts(ts: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(ts).ok().map(|t| t.timestamp())
}

fn unix(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn local_midnight(now: i64) -> i64 {
    let Some(local) = Local.timestamp_opt(now, 0).single() else { return now - now.rem_euclid(86_400) };
    local
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|m| Local.from_local_datetime(&m).earliest())
        .map_or(now - now.rem_euclid(86_400), |m| m.timestamp())
}

#[derive(Deserialize)]
struct Line {
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    #[serde(rename = "isSidechain", default)]
    is_sidechain: bool,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
}

/// The parts of the status line JSON used here.
#[derive(Deserialize)]
struct StatusJson {
    model: Option<StatusModel>,
    rate_limits: Option<RateLimits>,
}

#[derive(Deserialize)]
struct StatusModel {
    id: Option<String>,
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct RateLimits {
    five_hour: Option<Window>,
    seven_day: Option<Window>,
}

#[derive(Deserialize)]
struct Window {
    used_percentage: Option<f32>,
    resets_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn assistant(ts: &str, id: &str, input: u64, output: u64) -> String {
        format!(
            "{{\"type\":\"assistant\",\"timestamp\":\"{ts}\",\"requestId\":\"req_{id}\",\"message\":{{\"id\":\"msg_{id}\",\
             \"model\":\"claude-opus-5-5\",\"usage\":{{\"input_tokens\":{input},\"output_tokens\":{output},\
             \"cache_creation_input_tokens\":100,\"cache_read_input_tokens\":90000}}}}}}\n"
        )
    }

    #[test]
    fn model_names() {
        assert_eq!(short_model("claude-opus-5-5").as_deref(), Some("OPUS 5.5"));
        assert_eq!(short_model("claude-sonnet-4-5-20250929").as_deref(), Some("SONNET 4.5"));
        assert_eq!(short_model("claude-haiku-4-5-20251001").as_deref(), Some("HAIKU 4.5"));
        assert_eq!(short_model("claude-fable-5-1").as_deref(), Some("FABLE 5.1"));
        assert_eq!(short_model("<synthetic>"), None);
    }

    #[test]
    fn blocks_start_on_the_hour_and_last_five_hours() {
        let e = |ts| Entry { ts, tokens: 1, key: None };
        let h = 3600;
        // 10:20 and 12:00 share a block from 10:00; 16:30 opens a new one at 16:00.
        let entries = [e(10 * h + 1200), e(12 * h), e(16 * h + 1800)];
        assert_eq!(current_block(&entries, 17 * h), Some((16 * h, 21 * h)));
        assert_eq!(current_block(&entries[..2], 13 * h), Some((10 * h, 15 * h)));
        assert_eq!(current_block(&entries[..2], 15 * h), None);
    }

    #[test]
    fn counts_each_message_once_and_reads_appended_lines() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("projects/-home-me-app");
        fs::create_dir_all(&project).unwrap();
        let now = chrono::Utc::now();
        let ts = (now - chrono::Duration::minutes(10)).to_rfc3339();
        let path = project.join("s.jsonl");
        let mut f = File::create(&path).unwrap();
        // Two content blocks of one message repeat the usage.
        f.write_all(assistant(&ts, "a", 10, 5).as_bytes()).unwrap();
        f.write_all(assistant(&ts, "a", 10, 5).as_bytes()).unwrap();
        f.write_all(b"{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n").unwrap();

        let mut usage = ClaudeUsage::with_paths(Some(dir.path().join("projects")), None);
        let m = usage.sample().unwrap();
        assert_eq!(m.tok, 115);
        assert_eq!(m.model.as_deref(), Some("OPUS 5.5"));
        assert_eq!(m.state, ClaudeState::Work);
        assert!(m.left_min.is_some());

        f.write_all(assistant(&ts, "b", 1, 1).as_bytes()).unwrap();
        // A half-written line waits for its newline.
        f.write_all(b"{\"type\":\"assistant\"").unwrap();
        usage.last_scan = None;
        assert_eq!(usage.sample().unwrap().tok, 217);
    }

    #[test]
    fn status_line_limits() {
        let dir = tempfile::tempdir().unwrap();
        let status = dir.path().join("claude-status.json");
        let reset = unix(SystemTime::now()) + 3600;
        fs::write(
            &status,
            format!(
                "{{\"model\":{{\"id\":\"claude-sonnet-5\",\"display_name\":\"Sonnet 5\"}},\"rate_limits\":{{\
                 \"five_hour\":{{\"used_percentage\":42.5,\"resets_at\":{reset}}},\
                 \"seven_day\":{{\"used_percentage\":18,\"resets_at\":{}}}}}}}",
                reset - 7200
            ),
        )
        .unwrap();
        let mut usage = ClaudeUsage::with_paths(None, Some(status));
        let m = usage.sample().unwrap();
        assert_eq!(m.s_pct, Some(42.5));
        // The weekly window already reset.
        assert_eq!(m.w_pct, Some(0.0));
        assert!(matches!(m.left_min, Some(59 | 60)));
        assert_eq!(m.model.as_deref(), Some("SONNET 5"));
    }
}
