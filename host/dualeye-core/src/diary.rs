//! The pet's memory of your days, for [`crate::quips`]: when you started and
//! stopped using the computer, how long you used it and for how long without
//! a break, how long Claude worked and music played, the hottest it got, and
//! the timers and pomodoros you finished.
//!
//! A day runs from 5:00 to 5:00, so a late night belongs to the day before:
//! its minutes go past 24 × 60. You count as there while the computer had
//! input in the last minute ([`crate::presence`]); where the OS won't say
//! (Linux), nothing is noted but Claude, music, heat and timers. The last
//! week is kept in `diary.json` in DualEye's data folder and never leaves the
//! computer.

use std::fs;
use std::path::PathBuf;

use chrono::{Datelike, Duration as Days, NaiveDate, NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};

use crate::timers::{Fired, Kind};

/// A day starts at this hour; before it, it's still the night before.
pub const DAY_STARTS_H: u32 = 5;
/// Input this recent: you're at the computer.
const ACTIVE_IDLE_S: u32 = 60;
/// This long away ends a stretch of work.
pub const BREAK_S: i64 = 600;
/// Days kept.
const KEEP_DAYS: usize = 8;
/// Written at most this often, and when a day ends.
const SAVE_EVERY_S: i64 = 300;

/// One day, as far as the pet noticed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Day {
    /// `2026-10-09`: the day it started on.
    pub date: String,
    /// Minutes from that day's midnight of the first and last input: past
    /// 1440 after midnight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_min: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_min: Option<u32>,
    /// Seconds at the computer, and the longest stretch without a break.
    #[serde(default)]
    pub active_s: u32,
    #[serde(default)]
    pub longest_s: u32,
    #[serde(default)]
    pub claude_s: u32,
    #[serde(default)]
    pub music_s: u32,
    /// The hottest the CPU or GPU got.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hottest_c: Option<f32>,
    /// Timers and reminders that rang, and pomodoro work phases finished.
    #[serde(default)]
    pub timers: u32,
    #[serde(default)]
    pub focus: u32,
}

impl Day {
    pub fn date(&self) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(&self.date, "%Y-%m-%d").ok()
    }
}

/// What a snapshot says, for [`Diary::observe`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Seen {
    /// Seconds without input; `None` where the OS won't tell.
    pub idle_s: Option<u32>,
    pub claude: bool,
    pub music: bool,
    pub hottest_c: Option<f32>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    days: Vec<Day>,
}

#[derive(Debug, Default)]
pub struct Diary {
    /// Oldest first; the last is today's.
    days: Vec<Day>,
    file: Option<PathBuf>,
    /// When you last had input, and since when without a break (seconds of
    /// the local clock).
    active_at: Option<i64>,
    stretch_from: Option<i64>,
    observed_at: Option<i64>,
    idle_s: Option<u32>,
    saved_at: i64,
    dirty: bool,
}

/// Where the diary is kept by default.
pub fn default_file() -> Option<PathBuf> {
    crate::claude::data_dir().map(|d| d.join("diary.json"))
}

/// The day `at` belongs to, and its minutes from that day's midnight.
pub fn day_of(at: NaiveDateTime) -> (NaiveDate, u32) {
    let date = (at - Days::hours(DAY_STARTS_H as i64)).date();
    let days = (at.date() - date).num_days() as u32;
    (date, days * 24 * 60 + at.hour() * 60 + at.minute())
}

impl Diary {
    /// The diary kept in `file`, if any; it's written back there.
    pub fn open(file: Option<PathBuf>) -> Self {
        let saved: Saved = file.as_ref().and_then(|f| fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { days: saved.days, file, ..Default::default() }
    }

    /// Note one look at the computer, `dt_s` after the last.
    pub fn observe(&mut self, at: NaiveDateTime, seen: &Seen, dt_s: u32) {
        let (date, minute) = day_of(at);
        let now = at.and_utc().timestamp();
        let date_s = date.format("%Y-%m-%d").to_string();
        let new_day = self.days.last().is_none_or(|d| d.date != date_s);
        if new_day {
            if !self.days.is_empty() {
                self.dirty = true;
                self.save_now(now);
            }
            self.days.push(Day { date: date_s, ..Default::default() });
            if self.days.len() > KEEP_DAYS {
                self.days.remove(0);
            }
        }
        self.observed_at = Some(now);
        self.idle_s = seen.idle_s;
        let day = self.days.last_mut().expect("today");
        if seen.idle_s.is_some_and(|i| i < ACTIVE_IDLE_S) {
            day.first_min.get_or_insert(minute);
            day.last_min = Some(minute);
            day.active_s += dt_s;
            if self.active_at.is_none_or(|a| now - a > BREAK_S) {
                self.stretch_from = Some(now);
            }
            self.active_at = Some(now);
            let stretch = (now - self.stretch_from.unwrap_or(now)) as u32;
            day.longest_s = day.longest_s.max(stretch);
        }
        if seen.claude {
            day.claude_s += dt_s;
        }
        if seen.music {
            day.music_s += dt_s;
        }
        if let Some(t) = seen.hottest_c {
            day.hottest_c = Some(day.hottest_c.map_or(t, |h| h.max(t)));
        }
        self.dirty = true;
        if now - self.saved_at >= SAVE_EVERY_S {
            self.save_now(now);
        }
    }

    /// A timer, reminder or pomodoro phase ended.
    pub fn fired(&mut self, fired: &Fired) {
        let Some(day) = self.days.last_mut() else { return };
        match fired.timer.kind {
            Kind::Work => day.focus += 1,
            Kind::Break => {}
            Kind::Timer | Kind::Reminder => day.timers += 1,
        }
        self.dirty = true;
    }

    pub fn today(&self) -> Option<&Day> {
        self.days.last()
    }

    /// The day before today, if the pet saw it.
    pub fn yesterday(&self) -> Option<&Day> {
        let today = self.today()?.date()?;
        let n = self.days.len();
        self.days[..n.saturating_sub(1)].last().filter(|d| d.date() == today.pred_opt())
    }

    /// The days before today, newest first, back to the first one missing.
    pub fn run_before_today(&self) -> impl Iterator<Item = &Day> {
        let mut want = self.today().and_then(Day::date).and_then(|d| d.pred_opt());
        self.days.iter().rev().skip(1).take_while(move |d| {
            let ok = want.is_some() && d.date() == want;
            want = want.and_then(|w| w.pred_opt());
            ok
        })
    }

    /// Seconds at the computer without a break, now.
    pub fn stretch_s(&self) -> u32 {
        match (self.stretch_from, self.active_at, self.observed_at) {
            (Some(from), Some(active), Some(now)) if now - active <= BREAK_S => (active - from) as u32,
            _ => 0,
        }
    }

    /// Seconds without input at the last look; `None` where the OS won't tell.
    pub fn idle_s(&self) -> Option<u32> {
        self.idle_s
    }

    pub fn weekday(&self) -> Option<chrono::Weekday> {
        self.today()?.date().map(|d| d.weekday())
    }

    /// Write it out if anything changed.
    pub fn save(&mut self) {
        let now = self.observed_at.unwrap_or_default();
        self.save_now(now);
    }

    fn save_now(&mut self, now: i64) {
        self.saved_at = now;
        if !std::mem::take(&mut self.dirty) {
            return;
        }
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let saved = Saved { days: self.days.clone() };
        let _ = serde_json::to_vec_pretty(&saved).map(|b| fs::write(file, b));
    }
}

impl Drop for Diary {
    fn drop(&mut self) {
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    fn active() -> Seen {
        Seen { idle_s: Some(3), ..Default::default() }
    }

    #[test]
    fn a_late_night_belongs_to_the_day_before() {
        assert_eq!(day_of(at(9, 2, 14)), (NaiveDate::from_ymd_opt(2026, 10, 8).unwrap(), 26 * 60 + 14));
        assert_eq!(day_of(at(9, 9, 0)), (NaiveDate::from_ymd_opt(2026, 10, 9).unwrap(), 9 * 60));
    }

    #[test]
    fn notes_first_last_and_stretches() {
        let mut diary = Diary::open(None);
        diary.observe(at(8, 9, 0), &active(), 1);
        diary.observe(at(8, 11, 0), &active(), 1);
        // A long break ends the stretch.
        diary.observe(at(8, 14, 0), &active(), 1);
        diary.observe(at(9, 2, 10), &active(), 1);
        let day = diary.today().unwrap();
        assert_eq!(day.date, "2026-10-08");
        assert_eq!(day.first_min, Some(9 * 60));
        assert_eq!(day.last_min, Some(26 * 60 + 10));
        assert_eq!(day.active_s, 4);
        assert_eq!(diary.stretch_s(), 0);

        diary.observe(at(9, 8, 0), &active(), 1);
        diary.observe(at(9, 8, 5), &active(), 1);
        assert_eq!(diary.today().unwrap().date, "2026-10-09");
        assert_eq!(diary.yesterday().unwrap().last_min, Some(26 * 60 + 10));
        assert_eq!(diary.stretch_s(), 300);
    }

    #[test]
    fn idle_or_unknown_isnt_there() {
        let mut diary = Diary::open(None);
        diary.observe(at(8, 9, 0), &Seen { idle_s: Some(600), music: true, ..Default::default() }, 1);
        diary.observe(at(8, 9, 1), &Seen { idle_s: None, hottest_c: Some(91.0), ..Default::default() }, 1);
        let day = diary.today().unwrap();
        assert_eq!(day.first_min, None);
        assert_eq!(day.music_s, 1);
        assert_eq!(day.hottest_c, Some(91.0));
    }

    #[test]
    fn keeps_a_week_and_saves() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("diary.json");
        {
            let mut diary = Diary::open(Some(file.clone()));
            for d in 1..=12 {
                diary.observe(at(d, 10, 0), &active(), 1);
            }
            assert_eq!(diary.days.len(), KEEP_DAYS);
            assert_eq!(diary.run_before_today().count(), KEEP_DAYS - 1);
        }
        let diary = Diary::open(Some(file));
        assert_eq!(diary.today().unwrap().date, "2026-10-12");
    }
}
