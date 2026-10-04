//! Timers, a pomodoro and reminders, kept on the host.
//!
//! [`Timers`] holds them, by the wall clock, in `timers.json` in DualEye's
//! data folder, so they outlive the app and a reminder set for 17:00 still
//! comes. Whoever has one (the bridge, the app, the voice agent, the hub for
//! `dualeye timer` and MCP) starts, pauses and cancels them; the bridge calls
//! [`Timers::tick`] once a second and puts [`Timers::board_view`], the timer
//! that ends first, in each snapshot: the board's `timer` face counts it
//! down, takes over the screen [`ShowOn`] picks while it runs and rings when
//! the host says it's up. The host then says why out loud ([`Fired::spoken`]).
//!
//! A timer rings for ten seconds, or until someone says the wake word or
//! dismisses it ([`Timers::dismiss`]). A pomodoro is work and break timers
//! in turn: when one ends the next starts at once and the old one rings for
//! a few seconds.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{Local, NaiveTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::link::Tool;

/// A timer or reminder rings this long unless it's dismissed.
pub const RING_MS: i64 = 10_000;
/// A pomodoro's phase change rings this long.
const PHASE_RING_MS: i64 = 6_000;
/// A reminder missed while the app was closed still comes this late.
const LATE_REMINDER_MS: i64 = 12 * 3600 * 1000;
const MAX_TIMERS: usize = 12;
const MAX_SECS: u64 = 24 * 3600;
/// The board's bold font: ASCII capitals, digits and punctuation.
const BOARD_LABEL_MAX: usize = 27;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Timer,
    /// A pomodoro's work phase.
    Work,
    /// A pomodoro's break.
    Break,
    Reminder,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Timer => "timer",
            Kind::Work => "work",
            Kind::Break => "break",
            Kind::Reminder => "reminder",
        }
    }

    fn is_pomodoro(self) -> bool {
        matches!(self, Kind::Work | Kind::Break)
    }
}

/// The screen a running timer takes over; `none` leaves the faces alone
/// (the `timer` face still shows it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShowOn {
    Left,
    #[default]
    Right,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timer {
    pub id: u32,
    pub kind: Kind,
    /// What it's for: "pasta", or a reminder's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub total_s: u64,
    /// When it ends, in ms since the epoch; `None` while paused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<i64>,
    /// While paused, the ms it has left.
    #[serde(default)]
    pub paused_left_ms: i64,
    /// Since when it rings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ringing_since: Option<i64>,
    /// What to say when it ends in: `it` or `en`.
    pub language: String,
}

impl Timer {
    pub fn left_ms(&self, now: i64) -> i64 {
        match self.ends_at {
            Some(end) => (end - now).max(0),
            None => self.paused_left_ms,
        }
    }

    pub fn is_paused(&self) -> bool {
        self.ends_at.is_none()
    }

    pub fn is_ringing(&self) -> bool {
        self.ringing_since.is_some()
    }

    pub fn state(&self) -> &'static str {
        if self.is_ringing() {
            "ring"
        } else if self.is_paused() {
            "pause"
        } else {
            "run"
        }
    }

    fn matches(&self, which: &str) -> bool {
        let which = which.to_lowercase();
        let label = self.label.as_deref().unwrap_or("").to_lowercase();
        (!label.is_empty() && (label.contains(&which) || which.contains(&label)))
            || which == self.kind.name()
            || (self.kind.is_pomodoro() && which.starts_with("pomodor"))
            || which == self.id.to_string()
    }
}

/// The pomodoro running, if any: its timers are the `work` and `break` ones.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pomodoro {
    pub work_s: u64,
    pub break_s: u64,
    pub rounds: u32,
    /// From 1.
    pub round: u32,
}

/// A timer as the tools and the app list it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimerInfo {
    pub id: u32,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub total_s: u64,
    pub left_s: u64,
    /// `run`, `pause` or `ring`.
    pub state: String,
    /// Local time it ends at, "HH:MM", while it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<String>,
}

/// The `timer` of a snapshot: what the board's timer face shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardTimer {
    pub kind: Kind,
    /// `run`, `pause` or `ring`.
    pub state: String,
    pub left_s: f32,
    pub total_s: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Other timers running besides it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub more: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rounds: Option<u32>,
    /// `left` or `right`: the screen it takes over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<String>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// A timer that just ended, for what to say about it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fired {
    pub timer: Timer,
    /// The pomodoro after the change: the next phase has started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pomodoro: Option<Pomodoro>,
}

impl Fired {
    /// What to say, in the timer's language.
    pub fn spoken(&self) -> String {
        let it = self.timer.language == "it";
        let label = self.timer.label.as_deref();
        match (self.timer.kind, &self.pomodoro, it) {
            (Kind::Reminder, _, true) => format!("Promemoria: {}.", label.unwrap_or("è l'ora")),
            (Kind::Reminder, _, false) => format!("Reminder: {}.", label.unwrap_or("it's time")),
            (Kind::Timer, _, true) => match label {
                Some(l) => format!("Il timer {l} è finito."),
                None => format!("Il timer di {} è finito.", span_words(self.timer.total_s, true)),
            },
            (Kind::Timer, _, false) => match label {
                Some(l) => format!("Your {l} timer is done."),
                None => format!("Your timer for {} is done.", span_words(self.timer.total_s, false)),
            },
            (Kind::Work, Some(p), true) => format!("Fine del pomodoro {}. Pausa di {}.", p.round, span_words(p.break_s, true)),
            (Kind::Work, Some(p), false) => format!("Pomodoro {} done. Break for {}.", p.round, span_words(p.break_s, false)),
            (Kind::Work, None, true) => "Pomodoro completato, ottimo lavoro!".into(),
            (Kind::Work, None, false) => "That's your pomodoro done, nice work!".into(),
            (Kind::Break, Some(p), true) => format!("Pausa finita: pomodoro {} di {}.", p.round, p.rounds),
            (Kind::Break, Some(p), false) => format!("Break's over: pomodoro {} of {}.", p.round, p.rounds),
            (Kind::Break, None, true) => "Pausa finita.".into(),
            (Kind::Break, None, false) => "Break's over.".into(),
        }
    }
}

/// "10 minuti", "1 ora e 30 minuti", "45 seconds".
pub fn span_words(secs: u64, it: bool) -> String {
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    let unit = |n: u64, one_it: &str, many_it: &str, one_en: &str, many_en: &str| {
        let word = match (it, n == 1) {
            (true, true) => one_it,
            (true, false) => many_it,
            (false, true) => one_en,
            (false, false) => many_en,
        };
        format!("{n} {word}")
    };
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(unit(h, "ora", "ore", "hour", "hours"));
    }
    if m > 0 {
        parts.push(unit(m, "minuto", "minuti", "minute", "minutes"));
    }
    if s > 0 || parts.is_empty() {
        parts.push(unit(s, "secondo", "secondi", "second", "seconds"));
    }
    let and = if it { " e " } else { " and " };
    match parts.len() {
        1 => parts.remove(0),
        _ => {
            let last = parts.pop().unwrap_or_default();
            format!("{}{and}{last}", parts.join(", "))
        }
    }
}

/// When a reminder is due.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum When {
    In(u64),
    /// The next time the clock says it: today, or tomorrow once it's passed.
    At(NaiveTime),
}

/// What [`Timers::control`] does to the timers it picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Cancel,
    Pause,
    Resume,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    show_on: ShowOn,
    #[serde(default)]
    timers: Vec<Timer>,
    #[serde(default)]
    pomodoro: Option<Pomodoro>,
    #[serde(default)]
    next_id: u32,
}

impl State {
    fn add(&mut self, kind: Kind, label: Option<String>, total_s: u64, ends_at: i64, language: &str) -> Timer {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let timer = Timer { id: self.next_id, kind, label, total_s, ends_at: Some(ends_at), paused_left_ms: 0, ringing_since: None, language: language.into() };
        self.timers.push(timer.clone());
        timer
    }
}

pub struct Timers {
    state: Mutex<State>,
    file: Option<PathBuf>,
    /// When something ringing was last silenced.
    dismissed: Mutex<Option<Instant>>,
}

impl std::fmt::Debug for Timers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Timers").field("file", &self.file).finish_non_exhaustive()
    }
}

impl Default for Timers {
    /// Kept in memory only.
    fn default() -> Self {
        Self { state: Mutex::default(), file: None, dismissed: Mutex::default() }
    }
}

/// Where the timers are kept by default.
pub fn default_file() -> Option<PathBuf> {
    crate::claude::data_dir().map(|d| d.join("timers.json"))
}

pub fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

/// ASCII capitals for the board's font: accents dropped, the rest kept if the font has it.
pub fn board_label(text: &str) -> String {
    let plain: String = text
        .chars()
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ä' | 'À' | 'Á' => 'A',
            'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' => 'E',
            'ì' | 'í' | 'î' | 'ï' => 'I',
            'ò' | 'ó' | 'ô' | 'ö' => 'O',
            'ù' | 'ú' | 'û' | 'ü' => 'U',
            '’' => '\'',
            c => c.to_ascii_uppercase(),
        })
        .filter(|c| (' '..='Z').contains(c))
        .collect();
    let words = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    words.chars().take(BOARD_LABEL_MAX).collect()
}

fn clean_label(label: Option<&str>) -> Option<String> {
    let l = label?.trim().trim_end_matches(['.', '!', '?']).trim();
    (!l.is_empty()).then(|| l.chars().take(80).collect())
}

impl Timers {
    /// The timers kept in `file`, if any; changes are written back to it.
    pub fn open(file: Option<PathBuf>) -> Self {
        let mut state: State = file.as_ref().and_then(|f| fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let now = now_ms();
        // Whatever rang before is over; a timer that ended meanwhile is gone,
        // a reminder rings late rather than never.
        state.timers.retain(|t| {
            let late = t.ends_at.map_or(0, |e| now - e);
            !t.is_ringing() && (late <= 0 || (t.kind == Kind::Reminder && late < LATE_REMINDER_MS))
        });
        if !state.timers.iter().any(|t| t.kind.is_pomodoro()) {
            state.pomodoro = None;
        }
        let timers = Self { state: Mutex::new(state), file, dismissed: Mutex::default() };
        timers.save(&timers.state.lock().unwrap());
        timers
    }

    fn save(&self, state: &State) {
        if let Some(file) = &self.file {
            if let Some(dir) = file.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let _ = serde_json::to_vec_pretty(state).map(|b| fs::write(file, b));
        }
    }

    fn change<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        let mut state = self.state.lock().unwrap();
        let out = f(&mut state);
        self.save(&state);
        out
    }

    pub fn show_on(&self) -> ShowOn {
        self.state.lock().unwrap().show_on
    }

    pub fn set_show_on(&self, show_on: ShowOn) {
        self.change(|s| s.show_on = show_on);
    }

    /// A countdown of `secs`, called `label`.
    pub fn start(&self, secs: u64, label: Option<&str>, language: &str) -> Result<Timer, String> {
        self.start_at(now_ms(), secs, label, language)
    }

    fn start_at(&self, now: i64, secs: u64, label: Option<&str>, language: &str) -> Result<Timer, String> {
        if secs == 0 || secs > MAX_SECS {
            return Err("a timer lasts from 1 second to 24 hours".into());
        }
        self.change(|s| {
            if s.timers.len() >= MAX_TIMERS {
                return Err(format!("there are {MAX_TIMERS} timers already"));
            }
            Ok(s.add(Kind::Timer, clean_label(label), secs, now + secs as i64 * 1000, language))
        })
    }

    pub fn remind(&self, text: &str, when: When, language: &str) -> Result<Timer, String> {
        self.remind_at(now_ms(), text, when, language)
    }

    fn remind_at(&self, now: i64, text: &str, when: When, language: &str) -> Result<Timer, String> {
        let label = clean_label(Some(text)).ok_or("a reminder needs its text")?;
        let secs = match when {
            When::In(secs) => secs,
            When::At(time) => secs_until(now, time),
        };
        if secs == 0 || secs > MAX_SECS {
            return Err("a reminder is due within 24 hours".into());
        }
        self.change(|s| {
            if s.timers.len() >= MAX_TIMERS {
                return Err(format!("there are {MAX_TIMERS} timers already"));
            }
            Ok(s.add(Kind::Reminder, Some(label), secs, now + secs as i64 * 1000, language))
        })
    }

    /// Work and break timers in turn, `rounds` times; replaces a pomodoro running.
    pub fn start_pomodoro(&self, work_min: u64, break_min: u64, rounds: u32, language: &str) -> Result<Timer, String> {
        self.start_pomodoro_at(now_ms(), work_min, break_min, rounds, language)
    }

    fn start_pomodoro_at(&self, now: i64, work_min: u64, break_min: u64, rounds: u32, language: &str) -> Result<Timer, String> {
        if !(1..=120).contains(&work_min) || !(1..=60).contains(&break_min) || !(1..=12).contains(&rounds) {
            return Err("work 1-120 minutes, break 1-60 minutes, 1-12 rounds".into());
        }
        self.change(|s| {
            s.timers.retain(|t| !t.kind.is_pomodoro());
            s.pomodoro = Some(Pomodoro { work_s: work_min * 60, break_s: break_min * 60, rounds, round: 1 });
            Ok(s.add(Kind::Work, None, work_min * 60, now + work_min as i64 * 60_000, language))
        })
    }

    /// Stop the pomodoro; false when none ran.
    pub fn stop_pomodoro(&self) -> bool {
        self.change(|s| {
            let had = s.pomodoro.take().is_some() || s.timers.iter().any(|t| t.kind.is_pomodoro());
            s.timers.retain(|t| !t.kind.is_pomodoro());
            had
        })
    }

    /// Cancel, pause or resume the timers `which` names: `all`, a label or
    /// kind (`pomodoro`, `reminder`), or `None` for the one that matters most
    /// (ringing, else ending first; for resume, paused). Returns them.
    pub fn control(&self, action: Action, which: Option<&str>) -> Result<Vec<Timer>, String> {
        self.control_at(now_ms(), action, which)
    }

    fn control_at(&self, now: i64, action: Action, which: Option<&str>) -> Result<Vec<Timer>, String> {
        self.change(|s| {
            let candidates: Vec<usize> = (0..s.timers.len())
                .filter(|&i| {
                    let t = &s.timers[i];
                    match action {
                        Action::Cancel => true,
                        Action::Pause => !t.is_paused() && !t.is_ringing(),
                        Action::Resume => t.is_paused(),
                    }
                })
                .collect();
            let which = which.map(str::trim).filter(|w| !w.is_empty() && !matches!(*w, "next" | "first" | "current"));
            let picked: Vec<usize> = match which {
                Some("all" | "tutti" | "tutto" | "everything") => candidates,
                Some(w) => candidates.into_iter().filter(|&i| s.timers[i].matches(w)).collect(),
                None => {
                    let ringing: Vec<usize> = candidates.iter().copied().filter(|&i| s.timers[i].is_ringing()).collect();
                    if !ringing.is_empty() {
                        ringing
                    } else {
                        candidates.into_iter().min_by_key(|&i| s.timers[i].left_ms(now)).into_iter().collect()
                    }
                }
            };
            if picked.is_empty() {
                return Err(match action {
                    Action::Resume => "no paused timer".into(),
                    _ if s.timers.is_empty() => "no timer is running".into(),
                    _ => "no timer like that".into(),
                });
            }
            let mut out = Vec::new();
            for &i in &picked {
                let t = &mut s.timers[i];
                match action {
                    Action::Cancel => {}
                    Action::Pause => {
                        t.paused_left_ms = t.left_ms(now);
                        t.ends_at = None;
                    }
                    Action::Resume => t.ends_at = Some(now + t.paused_left_ms),
                }
                out.push(t.clone());
            }
            if action == Action::Cancel {
                let mut i = 0;
                s.timers.retain(|_| {
                    i += 1;
                    !picked.contains(&(i - 1))
                });
                if out.iter().any(|t| t.kind.is_pomodoro() && !t.is_ringing()) {
                    s.pomodoro = None;
                    s.timers.retain(|t| !t.kind.is_pomodoro());
                }
            }
            Ok(out)
        })
    }

    /// Stop whatever rings; how many did.
    pub fn dismiss(&self) -> usize {
        let mut state = self.state.lock().unwrap();
        let before = state.timers.len();
        state.timers.retain(|t| !t.is_ringing());
        let n = before - state.timers.len();
        if n > 0 {
            self.save(&state);
            *self.dismissed.lock().unwrap() = Some(Instant::now());
        }
        n
    }

    /// Something ringing was silenced less than `ago` ago.
    pub fn dismissed_within(&self, ago: Duration) -> bool {
        self.dismissed.lock().unwrap().is_some_and(|at| at.elapsed() < ago)
    }

    pub fn is_ringing(&self) -> bool {
        self.state.lock().unwrap().timers.iter().any(Timer::is_ringing)
    }

    /// Timer `id` is still ringing: nobody silenced it.
    pub fn rings(&self, id: u32) -> bool {
        self.state.lock().unwrap().timers.iter().any(|t| t.id == id && t.is_ringing())
    }

    pub fn list(&self) -> Vec<TimerInfo> {
        self.list_at(now_ms())
    }

    fn list_at(&self, now: i64) -> Vec<TimerInfo> {
        let state = self.state.lock().unwrap();
        let mut timers: Vec<&Timer> = state.timers.iter().collect();
        timers.sort_by_key(|t| (!t.is_ringing(), t.is_paused(), t.left_ms(now)));
        timers
            .into_iter()
            .map(|t| TimerInfo {
                id: t.id,
                kind: t.kind,
                label: t.label.clone(),
                total_s: t.total_s,
                left_s: ((t.left_ms(now) + 999) / 1000) as u64,
                state: t.state().into(),
                ends_at: t.ends_at.filter(|_| !t.is_ringing()).and_then(|e| Local.timestamp_millis_opt(e).single()).map(|d| d.format("%H:%M").to_string()),
            })
            .collect()
    }

    pub fn pomodoro(&self) -> Option<Pomodoro> {
        self.state.lock().unwrap().pomodoro
    }

    /// Ring the timers that are up, move a pomodoro on, stop what has rung
    /// long enough. What just ended is returned, to be said.
    pub fn tick(&self) -> Vec<Fired> {
        self.tick_at(now_ms())
    }

    fn tick_at(&self, now: i64) -> Vec<Fired> {
        let mut state = self.state.lock().unwrap();
        let mut fired = Vec::new();
        let mut changed = false;
        let due: Vec<usize> = (0..state.timers.len()).filter(|&i| !state.timers[i].is_ringing() && state.timers[i].ends_at.is_some_and(|e| e <= now)).collect();
        for i in due {
            state.timers[i].ringing_since = Some(now);
            changed = true;
            let timer = state.timers[i].clone();
            let mut pomodoro = None;
            if timer.kind.is_pomodoro()
                && let Some(mut p) = state.pomodoro
            {
                let language = timer.language.clone();
                match timer.kind {
                    Kind::Work if p.round < p.rounds => {
                        state.add(Kind::Break, None, p.break_s, now + p.break_s as i64 * 1000, &language);
                        pomodoro = Some(p);
                    }
                    Kind::Work => state.pomodoro = None,
                    _ => {
                        p.round += 1;
                        state.add(Kind::Work, None, p.work_s, now + p.work_s as i64 * 1000, &language);
                        pomodoro = Some(p);
                    }
                }
                if pomodoro.is_some() {
                    state.pomodoro = pomodoro;
                }
            }
            fired.push(Fired { timer, pomodoro });
        }
        let before = state.timers.len();
        state.timers.retain(|t| {
            let rung = t.ringing_since.map_or(0, |since| now - since);
            rung < if t.kind.is_pomodoro() { PHASE_RING_MS } else { RING_MS }
        });
        changed |= state.timers.len() != before;
        if changed {
            self.save(&state);
        }
        fired
    }

    /// What the board shows: the timer ringing, else the one ending first
    /// (running before paused).
    pub fn board_view(&self) -> Option<BoardTimer> {
        self.board_view_at(now_ms())
    }

    fn board_view_at(&self, now: i64) -> Option<BoardTimer> {
        let state = self.state.lock().unwrap();
        let shown = state.timers.iter().min_by_key(|t| (!t.is_ringing(), t.is_paused(), t.left_ms(now)))?;
        let more = state.timers.iter().filter(|t| t.id != shown.id && !t.is_ringing()).count() as u32;
        let pomodoro = state.pomodoro.filter(|_| shown.kind.is_pomodoro());
        let round = pomodoro.map(|p| match shown.kind {
            // Ringing at the end of a break, the next round has begun.
            Kind::Break if shown.is_ringing() => p.round.saturating_sub(1).max(1),
            _ => p.round,
        });
        Some(BoardTimer {
            kind: shown.kind,
            state: shown.state().into(),
            left_s: (shown.left_ms(now) as f32 / 100.0).round() / 10.0,
            total_s: shown.total_s,
            label: shown.label.as_deref().map(board_label).filter(|l| !l.is_empty()),
            more,
            round,
            rounds: pomodoro.map(|p| p.rounds),
            screen: match state.show_on {
                ShowOn::Left => Some("left".into()),
                ShowOn::Right => Some("right".into()),
                ShowOn::None => None,
            },
        })
    }

    /// The tools the voice agent and MCP clients get.
    pub fn tools() -> Vec<Tool> {
        let tool = |name: &str, description: &str, schema: Value| Tool { name: name.into(), description: description.into(), input_schema: schema };
        vec![
            tool(
                "set_timer",
                "Start a countdown timer; the board shows it and rings when it's up. Give its length in hours, minutes and/or seconds.",
                json!({"type": "object", "properties": {
                    "hours": {"type": "number"},
                    "minutes": {"type": "number"},
                    "seconds": {"type": "number"},
                    "label": {"type": "string", "description": "What it's for, one or two words (\"pasta\"); optional"},
                }}),
            ),
            tool(
                "set_reminder",
                "Remind the user of something later: the board rings and says the text. Give either in_minutes or at.",
                json!({"type": "object", "properties": {
                    "text": {"type": "string", "description": "What to remind, short, in the user's words (\"call Marco\")"},
                    "in_minutes": {"type": "number", "description": "Minutes from now"},
                    "at": {"type": "string", "description": "Time of day, 24-hour HH:MM (today, or tomorrow if it has passed)"},
                }, "required": ["text"]}),
            ),
            tool(
                "pomodoro",
                "Start or stop a pomodoro: work and break timers in turn (25 and 5 minutes, 4 rounds unless told otherwise).",
                json!({"type": "object", "properties": {
                    "action": {"type": "string", "enum": ["start", "stop"]},
                    "work_minutes": {"type": "number"},
                    "break_minutes": {"type": "number"},
                    "rounds": {"type": "number"},
                }, "required": ["action"]}),
            ),
            tool(
                "control_timer",
                "Cancel, pause or resume timers and reminders. Cancel also silences one that is ringing.",
                json!({"type": "object", "properties": {
                    "action": {"type": "string", "enum": ["cancel", "pause", "resume"]},
                    "which": {"type": "string", "description": "A timer's label, \"pomodoro\", \"reminder\" or \"all\"; leave out for the next one"},
                }, "required": ["action"]}),
            ),
            tool("get_timers", "The timers, pomodoro and reminders running, with the time left on each.", json!({"type": "object", "properties": {}})),
        ]
    }

    /// Run timer tool `name`; `None` when it's not one of [`Timers::tools`].
    pub fn call_tool(&self, name: &str, args: &Value, language: &str) -> Option<Result<String, String>> {
        let num = |k: &str| args.get(k).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))).filter(|n| *n >= 0.0);
        let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
        let result = match name {
            "set_timer" => {
                let secs = num("hours").unwrap_or(0.0) * 3600.0 + num("minutes").unwrap_or(0.0) * 60.0 + num("seconds").unwrap_or(0.0);
                self.start(secs.round() as u64, text("label"), language).map(|t| format!("timer {} set for {}", t.id, span_words(t.total_s, false)))
            }
            "set_reminder" => {
                let when = match (num("in_minutes"), text("at").and_then(parse_clock)) {
                    (_, Some(at)) => Some(When::At(at)),
                    (Some(m), None) => Some(When::In((m * 60.0).round() as u64)),
                    (None, None) => None,
                };
                match (text("text"), when) {
                    (Some(t), Some(when)) => self.remind(t, when, language).map(|r| {
                        let at = Local.timestamp_millis_opt(r.ends_at.unwrap_or_default()).single().map(|d| d.format("%H:%M").to_string());
                        format!("reminder set for {}", at.unwrap_or_default())
                    }),
                    (None, _) => Err("text is required".into()),
                    (_, None) => Err("give in_minutes or at (HH:MM)".into()),
                }
            }
            "pomodoro" => match text("action").unwrap_or("start") {
                "stop" => Ok(if self.stop_pomodoro() { "pomodoro stopped" } else { "no pomodoro was running" }.into()),
                _ => {
                    let n = |k: &str, default: f64| num(k).filter(|n| *n > 0.0).unwrap_or(default).round() as u64;
                    self.start_pomodoro(n("work_minutes", 25.0), n("break_minutes", 5.0), n("rounds", 4.0) as u32, language)
                        .map(|t| format!("pomodoro started: {} of work first", span_words(t.total_s, false)))
                }
            },
            "control_timer" => {
                let action = match text("action").unwrap_or("cancel") {
                    "pause" => Action::Pause,
                    "resume" => Action::Resume,
                    _ => Action::Cancel,
                };
                self.control(action, text("which")).map(|ts| {
                    let names: Vec<String> = ts.iter().map(|t| t.label.clone().unwrap_or_else(|| t.kind.name().into())).collect();
                    let done = match action {
                        Action::Cancel => "cancelled",
                        Action::Pause => "paused",
                        Action::Resume => "resumed",
                    };
                    format!("{done}: {}", names.join(", "))
                })
            }
            "get_timers" => Ok(serde_json::to_string(&json!({"timers": self.list(), "pomodoro": self.pomodoro(), "now": Local::now().format("%H:%M").to_string()})).unwrap_or_default()),
            _ => return None,
        };
        Some(result)
    }
}

/// Seconds from `now` until the clock next says `time`.
fn secs_until(now: i64, time: NaiveTime) -> u64 {
    let Some(now) = Local.timestamp_millis_opt(now).single() else { return 0 };
    let today = now.date_naive().and_time(time);
    let mut at = Local.from_local_datetime(&today).earliest().unwrap_or(now);
    if at <= now {
        at = Local.from_local_datetime(&(today + chrono::Duration::days(1))).earliest().unwrap_or(now);
    }
    (at - now).num_seconds().max(0) as u64
}

/// "17:30", "17.30", "5:30", "17".
pub fn parse_clock(text: &str) -> Option<NaiveTime> {
    let t = text.trim();
    let (h, m) = match t.split_once([':', '.']) {
        Some((h, m)) => (h.trim().parse().ok()?, m.trim().parse().ok()?),
        None => (t.parse().ok()?, 0),
    };
    NaiveTime::from_hms_opt(h, m, 0)
}

/// The hour and minute of `t`, for the replies.
pub fn clock_text(t: NaiveTime) -> String {
    format!("{}:{:02}", t.hour(), t.minute())
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_800_000_000_000;

    #[test]
    fn a_timer_rings_then_stops() {
        let timers = Timers::default();
        let t = timers.start_at(T0, 600, Some("pasta"), "it").unwrap();
        assert_eq!(timers.board_view_at(T0 + 1000).unwrap().left_s, 599.0);
        assert!(timers.tick_at(T0 + 599_000).is_empty());
        let fired = timers.tick_at(T0 + 600_200);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].timer.id, t.id);
        assert_eq!(fired[0].spoken(), "Il timer pasta è finito.");
        let view = timers.board_view_at(T0 + 601_000).unwrap();
        assert_eq!((view.state.as_str(), view.left_s, view.label.as_deref()), ("ring", 0.0, Some("PASTA")));
        assert_eq!(view.screen.as_deref(), Some("right"));
        // Not again; gone after ten seconds of ringing.
        assert!(timers.tick_at(T0 + 602_000).is_empty());
        timers.tick_at(T0 + 600_200 + RING_MS);
        assert!(timers.board_view_at(T0 + 700_000).is_none());
    }

    #[test]
    fn dismiss_cancel_pause_resume() {
        let timers = Timers::default();
        timers.start_at(T0, 60, None, "en").unwrap();
        timers.start_at(T0, 300, Some("tea"), "en").unwrap();
        timers.tick_at(T0 + 61_000);
        assert_eq!(timers.board_view_at(T0 + 61_000).unwrap().more, 1);
        assert_eq!(timers.dismiss(), 1);
        assert_eq!(timers.board_view_at(T0 + 61_000).unwrap().label.as_deref(), Some("TEA"));
        let paused = timers.control_at(T0 + 100_000, Action::Pause, None).unwrap();
        assert_eq!(paused[0].paused_left_ms, 200_000);
        assert_eq!(timers.board_view_at(T0 + 500_000).unwrap().state, "pause");
        timers.control_at(T0 + 500_000, Action::Resume, Some("tea")).unwrap();
        assert_eq!(timers.list_at(T0 + 500_000)[0].left_s, 200);
        assert!(timers.control_at(T0, Action::Cancel, Some("coffee")).is_err());
        timers.control_at(T0, Action::Cancel, Some("all")).unwrap();
        assert!(timers.list_at(T0).is_empty());
    }

    #[test]
    fn a_pomodoro_takes_turns() {
        let timers = Timers::default();
        timers.start_pomodoro_at(T0, 25, 5, 2, "en").unwrap();
        let fired = timers.tick_at(T0 + 25 * 60_000);
        assert_eq!(fired[0].spoken(), "Pomodoro 1 done. Break for 5 minutes.");
        // The break runs while the end of the work rings.
        let view = timers.board_view_at(T0 + 25 * 60_000 + 1000).unwrap();
        assert_eq!((view.kind, view.state.as_str()), (Kind::Work, "ring"));
        let later = T0 + 25 * 60_000 + PHASE_RING_MS;
        timers.tick_at(later);
        let view = timers.board_view_at(later).unwrap();
        assert_eq!((view.kind, view.round, view.rounds, view.more), (Kind::Break, Some(1), Some(2), 0));
        let fired = timers.tick_at(T0 + 30 * 60_000);
        assert_eq!(fired[0].spoken(), "Break's over: pomodoro 2 of 2.");
        let fired = timers.tick_at(T0 + 55 * 60_000);
        assert_eq!(fired[0].spoken(), "That's your pomodoro done, nice work!");
        timers.tick_at(T0 + 56 * 60_000);
        assert!(timers.board_view_at(T0 + 56 * 60_000).is_none());
        assert!(timers.pomodoro().is_none());
    }

    #[test]
    fn reminders_and_tools() {
        let timers = Timers::default();
        let r = timers.remind_at(T0, "chiamare Marco.", When::In(1200), "it").unwrap();
        assert_eq!(r.label.as_deref(), Some("chiamare Marco"));
        let fired = timers.tick_at(T0 + 1_200_000);
        assert_eq!(fired[0].spoken(), "Promemoria: chiamare Marco.");
        assert_eq!(timers.board_view_at(T0 + 1_200_000).unwrap().label.as_deref(), Some("CHIAMARE MARCO"));

        let out = timers.call_tool("set_timer", &json!({"minutes": 1.5}), "en").unwrap().unwrap();
        assert!(out.ends_with("set for 1 minute and 30 seconds"), "{out}");
        assert!(timers.call_tool("set_timer", &json!({}), "en").unwrap().is_err());
        assert!(timers.call_tool("set_reminder", &json!({"text": "stretch", "at": "7:05"}), "en").unwrap().is_ok());
        assert!(timers.call_tool("get_face", &json!({}), "en").is_none());
        assert!(timers.call_tool("control_timer", &json!({"action": "cancel", "which": "stretch"}), "en").unwrap().unwrap().contains("stretch"));
    }

    #[test]
    fn words_and_labels() {
        assert_eq!(span_words(600, true), "10 minuti");
        assert_eq!(span_words(5400, true), "1 ora e 30 minuti");
        assert_eq!(span_words(3661, false), "1 hour, 1 minute and 1 second");
        assert_eq!(board_label("Più tè, per favore!"), "PIU TE, PER FAVORE!");
        assert_eq!(parse_clock("17.30"), NaiveTime::from_hms_opt(17, 30, 0));
        assert_eq!(parse_clock("25:00"), None);
        let now = Local.with_ymd_and_hms(2026, 10, 2, 16, 0, 0).unwrap().timestamp_millis();
        assert_eq!(secs_until(now, NaiveTime::from_hms_opt(17, 0, 0).unwrap()), 3600);
        assert_eq!(secs_until(now, NaiveTime::from_hms_opt(15, 0, 0).unwrap()), 23 * 3600);
    }

    #[test]
    fn kept_in_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("timers.json");
        let timers = Timers::open(Some(file.clone()));
        timers.start(600, Some("bread"), "en").unwrap();
        timers.set_show_on(ShowOn::Left);
        let again = Timers::open(Some(file));
        assert_eq!(again.list()[0].label.as_deref(), Some("bread"));
        assert_eq!(again.show_on(), ShowOn::Left);
    }
}
