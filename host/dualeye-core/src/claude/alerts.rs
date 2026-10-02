//! Tell the person at the desk what Claude Code is up to: it needs them (a
//! permission or a question), it finished a long turn, or the plan's limits
//! are running out.
//!
//! The first two come from Claude Code's hooks ([`super::hooks`]): each hook
//! runs `<exe> --claude-hook`, which hands its JSON to the bridge through the
//! hub (`host/claude_hook`). The limits come from the status line, through
//! [`super::ClaudeMetrics`]. [`Alerts`] decides what is worth saying;
//! [`deliver`] says it on the board: a scene of the eyes, the words out loud
//! when the bridge speaks, then a message over the watch face.

use std::collections::HashMap;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ClaudeMetrics;
use crate::link::Link;
use crate::tts::Tts;
use crate::voice::Speaker;

/// Thresholds for the limits, as on the `claude` face: orange, then red.
const USAGE_STEPS: [f32; 2] = [80.0, 95.0];
/// A second "needs you" from the same session this soon says nothing new.
const REPEAT: Duration = Duration::from_secs(30);
/// How long the message stays over the watch face.
const TEXT_SECONDS: u32 = 10;
const TOOL_TIMEOUT: Duration = Duration::from_secs(2);

/// What the person wants to hear about. Shared with the bridge, so a
/// frontend can change it while it runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertSettings {
    /// Claude asks for a permission or asks a question.
    pub needs_you: bool,
    /// Claude finished a turn that took at least `done_after_s`.
    pub done: bool,
    pub done_after_s: u32,
    /// The 5-hour or weekly limit passed 80 % or 95 %.
    pub usage: bool,
    /// Also say it out loud (when the bridge has text-to-speech).
    pub speak: bool,
    /// `it` or `en`.
    pub language: String,
}

impl Default for AlertSettings {
    fn default() -> Self {
        Self { needs_you: true, done: true, done_after_s: 30, usage: true, speak: true, language: system_language().into() }
    }
}

/// `it` when the system speaks Italian, otherwise `en`.
pub fn system_language() -> &'static str {
    let italian = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .next()
        .is_some_and(|v| v.starts_with("it"));
    if italian { "it" } else { "en" }
}

/// What a Claude Code hook sends on stdin; only the fields used here.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HookEvent {
    pub hook_event_name: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    /// `Notification`: `permission_prompt`, `idle_prompt`, `elicitation_dialog`...
    #[serde(default)]
    pub notification_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Limit {
    FiveHour,
    Week,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Alert {
    /// Claude waits for a permission or an answer in `project`.
    NeedsYou { project: Option<String> },
    /// Claude finished a turn of `secs` in `project`.
    Done { project: Option<String>, secs: u64 },
    /// `limit` passed `pct`; the 5-hour one resets in `left_min`.
    Usage { limit: Limit, pct: u8, left_min: Option<u32> },
}

/// The words for an alert.
pub struct Words {
    /// For the screen: short, without accents.
    pub screen: String,
    pub spoken: String,
    /// The eyes' scene (`play_eyes`).
    pub eyes: &'static str,
}

impl Alert {
    pub fn words(&self, language: &str) -> Words {
        let it = language == "it";
        match self {
            Alert::NeedsYou { project } => Words {
                screen: with_project(if it { "Claude ti aspetta" } else { "Claude needs you" }, project),
                spoken: match (it, project) {
                    (true, Some(p)) => format!("Claude ha bisogno di te su {p}."),
                    (true, None) => "Claude ha bisogno di te.".into(),
                    (false, Some(p)) => format!("Claude needs you in {p}."),
                    (false, None) => "Claude needs you.".into(),
                },
                eyes: "surprised",
            },
            Alert::Done { project, .. } => Words {
                screen: with_project(if it { "Claude ha finito" } else { "Claude is done" }, project),
                spoken: match (it, project) {
                    (true, Some(p)) => format!("Claude ha finito su {p}."),
                    (true, None) => "Claude ha finito.".into(),
                    (false, Some(p)) => format!("Claude is done in {p}."),
                    (false, None) => "Claude is done.".into(),
                },
                eyes: "happy",
            },
            Alert::Usage { limit, pct, left_min } => {
                // "l'80", "il 95".
                let the = if matches!(pct, 1 | 8 | 11 | 80..=89) { "l'" } else { "il " };
                let (screen, mut spoken) = match (it, limit) {
                    (true, Limit::FiveHour) => (format!("Limite 5 ore: {pct}%"), format!("Hai usato {the}{pct} per cento del limite di cinque ore.")),
                    (true, Limit::Week) => (format!("Limite settimana: {pct}%"), format!("Hai usato {the}{pct} per cento del limite settimanale.")),
                    (false, Limit::FiveHour) => (format!("5-hour limit: {pct}%"), format!("You've used {pct} percent of your five-hour limit.")),
                    (false, Limit::Week) => (format!("Weekly limit: {pct}%"), format!("You've used {pct} percent of your weekly limit.")),
                };
                if let (Limit::FiveHour, Some(min)) = (limit, left_min) {
                    spoken.push(' ');
                    spoken.push_str(&if it { format!("Si azzera tra {}.", duration_it(*min)) } else { format!("It resets in {}.", duration_en(*min)) });
                }
                Words { screen, spoken, eyes: if *pct >= 95 { "sad" } else { "suspicious" } }
            }
        }
    }
}

fn with_project(title: &str, project: &Option<String>) -> String {
    match project {
        Some(p) => format!("{title}\n{p}"),
        None => title.to_string(),
    }
}

fn duration_it(min: u32) -> String {
    let (h, m) = (min / 60, min % 60);
    let hours = match h {
        0 => None,
        1 => Some("un'ora".to_string()),
        h => Some(format!("{h} ore")),
    };
    let minutes = match m {
        0 => None,
        1 => Some("un minuto".to_string()),
        m => Some(format!("{m} minuti")),
    };
    match (hours, minutes) {
        (Some(h), Some(m)) => format!("{h} e {m}"),
        (Some(h), None) => h,
        (None, Some(m)) => m,
        (None, None) => "meno di un minuto".into(),
    }
}

fn duration_en(min: u32) -> String {
    let plural = |n: u32, unit: &str| if n == 1 { format!("1 {unit}") } else { format!("{n} {unit}s") };
    match (min / 60, min % 60) {
        (0, 0) => "less than a minute".into(),
        (0, m) => plural(m, "minute"),
        (h, 0) => plural(h, "hour"),
        (h, m) => format!("{} and {}", plural(h, "hour"), plural(m, "minute")),
    }
}

/// What has been said already. Kept by the bridge across reconnects.
#[derive(Debug, Default)]
pub struct Alerts {
    /// When each session's current turn started (`UserPromptSubmit`).
    turns: HashMap<String, Instant>,
    /// The last "needs you" of each session.
    asked: HashMap<String, Instant>,
    /// The highest step passed by each limit in its current window; `None`
    /// until the first sample, which is taken as it is: no alert at launch
    /// for a limit passed earlier.
    five_hour_step: Option<usize>,
    week_step: Option<usize>,
}

impl Alerts {
    pub fn on_hook(&mut self, event: &HookEvent, settings: &AlertSettings, now: Instant) -> Option<Alert> {
        let project = event.cwd.as_deref().and_then(project_name);
        match event.hook_event_name.as_str() {
            "UserPromptSubmit" => {
                self.turns.insert(event.session_id.clone(), now);
                self.asked.remove(&event.session_id);
                None
            }
            "Stop" => {
                // A turn that started before the hooks were installed has no start: say nothing.
                let started = self.turns.remove(&event.session_id)?;
                let secs = now.duration_since(started).as_secs();
                (settings.done && secs >= u64::from(settings.done_after_s)).then_some(Alert::Done { project, secs })
            }
            "SessionEnd" => {
                self.turns.remove(&event.session_id);
                self.asked.remove(&event.session_id);
                None
            }
            "Notification" => {
                let waiting = matches!(event.notification_type.as_deref(), Some("permission_prompt" | "elicitation_dialog"));
                if !settings.needs_you || !waiting {
                    return None;
                }
                if self.asked.get(&event.session_id).is_some_and(|t| now.duration_since(*t) < REPEAT) {
                    return None;
                }
                self.asked.insert(event.session_id.clone(), now);
                Some(Alert::NeedsYou { project })
            }
            _ => None,
        }
    }

    /// The limits, on every sample: an alert the first time one passes a step
    /// in its window. A percentage going down means a new window.
    pub fn on_metrics(&mut self, metrics: &ClaudeMetrics, settings: &AlertSettings) -> Option<Alert> {
        let five = step(&mut self.five_hour_step, metrics.s_pct);
        let week = step(&mut self.week_step, metrics.w_pct);
        if !settings.usage {
            return None;
        }
        // The 5-hour one says when it resets: the more useful of the two.
        match (five, week) {
            (Some(pct), _) => Some(Alert::Usage { limit: Limit::FiveHour, pct, left_min: metrics.left_min }),
            (None, Some(pct)) => Some(Alert::Usage { limit: Limit::Week, pct, left_min: None }),
            (None, None) => None,
        }
    }
}

/// Move `announced` to the steps `pct` has passed; the step to announce, if
/// it passed a new one.
fn step(announced: &mut Option<usize>, pct: Option<f32>) -> Option<u8> {
    let pct = pct?;
    let passed = USAGE_STEPS.iter().filter(|&&s| pct >= s).count();
    let new = announced.is_some_and(|a| passed > a);
    *announced = Some(passed);
    new.then(|| USAGE_STEPS[passed - 1] as u8)
}

/// The folder Claude runs in, as the person knows the project.
fn project_name(cwd: &str) -> Option<String> {
    Path::new(cwd).file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty())
}

/// Say `alert` on the board: the eyes' scene, the words out loud with `tts`,
/// then the message over the watch face. Blocks until it's done.
pub fn deliver(link: &Link, speaker: &Speaker, tts: Option<&Tts>, alert: &Alert, language: &str) -> Result<(), String> {
    let words = alert.words(language);
    // Refused while the board is in a conversation or showing the ring: fine.
    let scene = link.call_tool("play_eyes", json!({"name": words.eyes}), TOOL_TIMEOUT).is_ok_and(|r| !r.is_error);
    if scene {
        // Speaking would cut the scene short.
        thread::sleep(Duration::from_millis(1500));
    }
    let spoken = tts.map(|tts| speaker.speak(link, tts, &words.spoken, language));
    let shown = link
        .call_tool("show_text", json!({"text": words.screen, "seconds": TEXT_SECONDS}), TOOL_TIMEOUT)
        .map_err(|e| e.to_string())
        .and_then(|r| if r.is_error { Err(r.text()) } else { Ok(()) });
    match spoken {
        Some(Err(e)) => Err(e),
        _ => shown,
    }
}

/// The hook's JSON from stdin, as `host/claude_hook` gets it.
pub fn parse_hook(json: &Value) -> Option<HookEvent> {
    serde_json::from_value(json.clone()).ok().filter(|e: &HookEvent| !e.hook_event_name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(name: &str, session: &str) -> HookEvent {
        HookEvent { hook_event_name: name.into(), session_id: session.into(), cwd: Some("/home/me/dualeye".into()), notification_type: None }
    }

    #[test]
    fn done_only_after_a_long_turn() {
        let settings = AlertSettings { done_after_s: 30, ..Default::default() };
        let mut alerts = Alerts::default();
        let t0 = Instant::now();
        assert_eq!(alerts.on_hook(&hook("UserPromptSubmit", "a"), &settings, t0), None);
        assert_eq!(alerts.on_hook(&hook("Stop", "a"), &settings, t0 + Duration::from_secs(5)), None);
        alerts.on_hook(&hook("UserPromptSubmit", "a"), &settings, t0);
        let done = alerts.on_hook(&hook("Stop", "a"), &settings, t0 + Duration::from_secs(45));
        assert_eq!(done, Some(Alert::Done { project: Some("dualeye".into()), secs: 45 }));
        // No start seen: nothing.
        assert_eq!(alerts.on_hook(&hook("Stop", "b"), &settings, t0 + Duration::from_secs(90)), None);
    }

    #[test]
    fn needs_you_once_per_wait() {
        let settings = AlertSettings::default();
        let mut alerts = Alerts::default();
        let t0 = Instant::now();
        let permission = HookEvent { notification_type: Some("permission_prompt".into()), ..hook("Notification", "a") };
        let idle = HookEvent { notification_type: Some("idle_prompt".into()), ..hook("Notification", "a") };
        assert_eq!(alerts.on_hook(&idle, &settings, t0), None);
        assert_eq!(alerts.on_hook(&permission, &settings, t0), Some(Alert::NeedsYou { project: Some("dualeye".into()) }));
        assert_eq!(alerts.on_hook(&permission, &settings, t0 + Duration::from_secs(5)), None);
        // A new prompt: the next wait is news again.
        alerts.on_hook(&hook("UserPromptSubmit", "a"), &settings, t0 + Duration::from_secs(6));
        assert!(alerts.on_hook(&permission, &settings, t0 + Duration::from_secs(7)).is_some());
        let off = AlertSettings { needs_you: false, ..Default::default() };
        assert_eq!(alerts.on_hook(&permission, &off, t0 + Duration::from_secs(60)), None);
    }

    #[test]
    fn usage_steps_once_per_window() {
        let settings = AlertSettings::default();
        let mut alerts = Alerts::default();
        let m = |s: f32, w: f32| ClaudeMetrics { s_pct: Some(s), w_pct: Some(w), left_min: Some(80), ..Default::default() };
        // At launch a limit already passed is no news.
        assert_eq!(alerts.on_metrics(&m(85.0, 10.0), &settings), None);
        assert_eq!(alerts.on_metrics(&m(50.0, 10.0), &settings), None);
        assert_eq!(alerts.on_metrics(&m(81.0, 10.0), &settings), Some(Alert::Usage { limit: Limit::FiveHour, pct: 80, left_min: Some(80) }));
        assert_eq!(alerts.on_metrics(&m(85.0, 10.0), &settings), None);
        assert_eq!(alerts.on_metrics(&m(96.0, 10.0), &settings), Some(Alert::Usage { limit: Limit::FiveHour, pct: 95, left_min: Some(80) }));
        // The window reset: 80 % is news again.
        assert_eq!(alerts.on_metrics(&m(0.0, 10.0), &settings), None);
        assert!(alerts.on_metrics(&m(82.0, 10.0), &settings).is_some());
        assert_eq!(alerts.on_metrics(&m(82.0, 81.0), &settings), Some(Alert::Usage { limit: Limit::Week, pct: 80, left_min: None }));
    }

    #[test]
    fn words() {
        let w = Alert::Usage { limit: Limit::FiveHour, pct: 80, left_min: Some(80) }.words("it");
        assert_eq!(w.spoken, "Hai usato l'80 per cento del limite di cinque ore. Si azzera tra un'ora e 20 minuti.");
        let w = Alert::Usage { limit: Limit::FiveHour, pct: 95, left_min: Some(120) }.words("en");
        assert_eq!(w.spoken, "You've used 95 percent of your five-hour limit. It resets in 2 hours.");
        assert_eq!(w.eyes, "sad");
        let w = Alert::NeedsYou { project: Some("app".into()) }.words("it");
        assert_eq!(w.screen, "Claude ti aspetta\napp");
    }

    #[test]
    fn parses_a_hook() {
        let json = json!({"session_id": "s", "hook_event_name": "Notification", "cwd": "/x/y", "notification_type": "permission_prompt", "message": "m"});
        let e = parse_hook(&json).unwrap();
        assert_eq!(e.notification_type.as_deref(), Some("permission_prompt"));
        assert!(parse_hook(&json!({"session_id": "s"})).is_none());
    }
}
