//! Now and then the pet says something about your day: "ieri hai fatto le
//! 2, eh", "three hours without a break: stretch a little?". Only with
//! something worth saying in the [`Diary`], only at a natural moment (the
//! first minutes of the day, coming back after a while, a long stretch of
//! work) and rarely: once a day at most by default, never at night or while
//! music plays, never the same thing twice in a day.
//!
//! [`Quips::consider`] decides when and what ([`Quip`]): a topic, the fact
//! behind it in a sentence for the language model, and a scene for the eyes.
//! The bridge then has the model put it in the pet's words
//! ([`crate::agent::Agent::quip`]), or takes [`Quip::fallback`] without one,
//! and says it.

use chrono::{NaiveDateTime, Timelike, Weekday};
use serde::{Deserialize, Serialize};

use crate::diary::Diary;

/// How often the pet comments on your day.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuipLevel {
    Off,
    /// Once a day at most.
    #[default]
    Rare,
    /// Up to three times a day.
    Often,
}

impl QuipLevel {
    /// The most in a day, and the least time between two.
    fn budget(self) -> Option<(usize, i64)> {
        match self {
            QuipLevel::Off => None,
            QuipLevel::Rare => Some((1, 4 * 3600)),
            QuipLevel::Often => Some((3, 90 * 60)),
        }
    }
}

/// Nothing said before this hour or from this one on.
const QUIET_UNTIL_H: u32 = 8;
const QUIET_FROM_H: u32 = 23;
/// Away this long, then back: a moment to say something.
const BACK_AFTER_S: u32 = 20 * 60;
/// Back means input this recent.
const BACK_IDLE_S: u32 = 10;
/// The day's first minutes: at the computer this long (settled in), not yet
/// this long.
const FIRST_FROM_S: u32 = 45;
const FIRST_UNTIL_S: u32 = 15 * 60;
/// A stretch of work this long without a break.
const STRETCH_S: u32 = 3 * 3600;
/// After midnight this late (minutes of the day before): a late night.
const LATE_MIN: u32 = 24 * 60 + 30;
/// Started before this: early.
const EARLY_MIN: u32 = 7 * 60;
const LONG_DAY_S: u32 = 9 * 3600;
const LONG_AWAY_S: u32 = 3 * 3600;
const HOT_C: f32 = 90.0;
const CLAUDE_DAY_S: u32 = 3 * 3600;
const FOCUS_DAY: u32 = 4;
const MUSIC_DAY_S: u32 = 3 * 3600;

/// What the pet comments on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    /// Up after midnight three nights running.
    LateStreak,
    /// Up after midnight last night.
    LateNight,
    LongDay,
    EarlyBird,
    Focus,
    HotDay,
    ClaudeDay,
    MusicDay,
    Weekend,
    Monday,
    LongAway,
    Lunch,
    NoBreak,
}

impl Topic {
    pub fn name(self) -> &'static str {
        match self {
            Topic::LateStreak => "late_streak",
            Topic::LateNight => "late_night",
            Topic::LongDay => "long_day",
            Topic::EarlyBird => "early_bird",
            Topic::Focus => "focus",
            Topic::HotDay => "hot_day",
            Topic::ClaudeDay => "claude_day",
            Topic::MusicDay => "music_day",
            Topic::Weekend => "weekend",
            Topic::Monday => "monday",
            Topic::LongAway => "long_away",
            Topic::Lunch => "lunch",
            Topic::NoBreak => "no_break",
        }
    }

    /// The eyes' scene before the words.
    pub fn scene(self) -> &'static str {
        match self {
            Topic::LateStreak => "eye_roll",
            Topic::LateNight => "suspicious",
            Topic::LongDay => "tired",
            Topic::EarlyBird => "surprised",
            Topic::Focus | Topic::ClaudeDay => "proud",
            Topic::HotDay => "hot",
            Topic::MusicDay => "sing",
            Topic::Weekend => "curious",
            Topic::Monday => "yawn",
            Topic::LongAway => "excited",
            Topic::Lunch => "happy",
            Topic::NoBreak => "sigh",
        }
    }
}

/// Something to say.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Quip {
    pub topic: Topic,
    /// The fact, in a sentence for the language model (English).
    pub fact: String,
    /// The numbers in it, for the fallback: an `HH:MM`, or minutes or hours.
    #[serde(skip)]
    detail: Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum Detail {
    #[default]
    None,
    /// Minutes of the day (past 1440 after midnight).
    Clock(u32),
    Hours(u32),
    Count(u32),
}

fn clock(min: u32) -> String {
    format!("{}:{:02}", (min / 60) % 24, min % 60)
}

impl Quip {
    fn new(topic: Topic, fact: String, detail: Detail) -> Self {
        Self { topic, fact, detail }
    }

    /// The words without a language model, in `language` (`it` or `en`).
    pub fn fallback(&self, language: &str) -> String {
        let it = language == "it";
        let n = match self.detail {
            Detail::Clock(m) => clock(m),
            Detail::Hours(h) | Detail::Count(h) => h.to_string(),
            Detail::None => String::new(),
        };
        match (self.topic, it) {
            (Topic::LateStreak, true) => "Tre notti di fila a far tardi. Stasera a nanna presto, promesso?".to_string(),
            (Topic::LateStreak, false) => "Three late nights in a row. Early to bed tonight, deal?".to_string(),
            (Topic::LateNight, true) => format!("Ieri hai fatto le {n}, eh. Io ho dormito benissimo."),
            (Topic::LateNight, false) => format!("Up until {n} last night, huh? I slept beautifully."),
            (Topic::LongDay, true) => format!("Ieri {n} ore al computer. Oggi ce la prendiamo più comoda?"),
            (Topic::LongDay, false) => format!("{n} hours at the computer yesterday. Easier day today?"),
            (Topic::EarlyBird, true) => format!("Già qui alle {n}? Io ho ancora gli occhi chiusi."),
            (Topic::EarlyBird, false) => format!("Here already at {n}? My eyes are still shut."),
            (Topic::Focus, true) => format!("Ieri {n} pomodori! Sono fiero di te."),
            (Topic::Focus, false) => format!("{n} pomodoros yesterday! I'm proud of you."),
            (Topic::HotDay, true) => "Ieri il computer scottava. Oggi andiamoci piano, eh.".to_string(),
            (Topic::HotDay, false) => "The computer was boiling yesterday. Go easy on it today.".to_string(),
            (Topic::ClaudeDay, true) => format!("Ieri Claude ha lavorato {n} ore. Più di me di sicuro."),
            (Topic::ClaudeDay, false) => format!("Claude worked {n} hours yesterday. More than me, for sure."),
            (Topic::MusicDay, true) => format!("Ieri {n} ore di musica. Ho ancora il ritornello in testa."),
            (Topic::MusicDay, false) => format!("{n} hours of music yesterday. It's still stuck in my head."),
            (Topic::Weekend, true) => "Anche nel weekend al computer? Io avrei fatto un pisolino.".to_string(),
            (Topic::Weekend, false) => "At the computer on the weekend too? I'd have napped.".to_string(),
            (Topic::Monday, true) => "Lunedì. Facciamo finta che sia venerdì?".to_string(),
            (Topic::Monday, false) => "Monday. Shall we pretend it's Friday?".to_string(),
            (Topic::LongAway, true) => "Eccoti! Mi stavo annoiando da morire.".to_string(),
            (Topic::LongAway, false) => "There you are! I was bored to bits.".to_string(),
            (Topic::Lunch, true) => "Bentornato! Mangiato bene? Io ho sognato croccantini.".to_string(),
            (Topic::Lunch, false) => "Welcome back! Good lunch? I dreamt of treats.".to_string(),
            (Topic::NoBreak, true) => format!("Sono {n} ore senza una pausa. Sgranchiamoci un attimo?"),
            (Topic::NoBreak, false) => format!("{n} hours without a break. Time for a stretch?"),
        }
    }
}

/// When the pet last spoke up, and what it already said today.
#[derive(Debug, Default)]
pub struct Quips {
    /// When it last did, in seconds of the local clock.
    said_at: Option<i64>,
    /// Today's date (as the diary has it) and what it said about it.
    day: String,
    topics: Vec<Topic>,
    /// The day's first minutes and a long stretch are moments once a day.
    first_done: bool,
    stretch_done: bool,
    /// The idle seconds at the last look.
    idle_s: Option<u32>,
}

impl Quips {
    /// Something to say now, or `None` (by far the most likely). `busy`: music
    /// plays or the board is in a conversation.
    pub fn consider(&mut self, diary: &Diary, at: NaiveDateTime, level: QuipLevel, busy: bool) -> Option<Quip> {
        let today = diary.today()?;
        if today.date != self.day {
            self.day = today.date.clone();
            self.topics.clear();
            self.first_done = false;
            self.stretch_done = false;
        }
        let was_idle = self.idle_s;
        self.idle_s = diary.idle_s();
        let (max, gap) = level.budget()?;
        let now = at.and_utc().timestamp();
        let hour = at.hour();
        if busy || !(QUIET_UNTIL_H..QUIET_FROM_H).contains(&hour) {
            return None;
        }
        if self.topics.len() >= max || self.said_at.is_some_and(|t| now - t < gap) {
            return None;
        }

        let back = matches!((was_idle, self.idle_s), (Some(w), Some(i)) if w >= BACK_AFTER_S && i < BACK_IDLE_S);
        let quip = if !self.first_done && (FIRST_FROM_S..FIRST_UNTIL_S).contains(&today.active_s) {
            self.first_done = true;
            self.first_of_day(diary)
        } else if back {
            self.on_return(was_idle.unwrap_or(0), hour)
        } else if !self.stretch_done && diary.stretch_s() >= STRETCH_S {
            self.stretch_done = true;
            Some(Quip::new(
                Topic::NoBreak,
                format!("The user has been at the computer for {} hours without a break.", diary.stretch_s() / 3600),
                Detail::Hours(diary.stretch_s() / 3600),
            ))
        } else {
            None
        }?;
        self.topics.push(quip.topic);
        self.said_at = Some(now);
        Some(quip)
    }

    fn fresh(&self, topic: Topic) -> bool {
        !self.topics.contains(&topic)
    }

    /// The first thing worth saying about yesterday (or today's start).
    fn first_of_day(&self, diary: &Diary) -> Option<Quip> {
        let today = diary.today()?;
        let yesterday = diary.yesterday();
        let late = |d: &crate::diary::Day| d.last_min.is_some_and(|m| m >= LATE_MIN);
        let streak = diary.run_before_today().take(3).filter(|d| late(d)).count() == 3;
        let mut candidates: Vec<Quip> = Vec::new();
        if streak {
            candidates.push(Quip::new(Topic::LateStreak, "The user stayed up past midnight three nights in a row.".into(), Detail::None));
        }
        if let Some(y) = yesterday {
            if let Some(m) = y.last_min.filter(|_| late(y)) {
                candidates.push(Quip::new(Topic::LateNight, format!("Last night the user was at the computer until {}.", clock(m)), Detail::Clock(m)));
            }
            if y.active_s >= LONG_DAY_S {
                let h = y.active_s / 3600;
                candidates.push(Quip::new(Topic::LongDay, format!("Yesterday the user spent {h} hours at the computer."), Detail::Hours(h)));
            }
            if y.focus >= FOCUS_DAY {
                candidates.push(Quip::new(Topic::Focus, format!("Yesterday the user finished {} pomodoro focus sessions.", y.focus), Detail::Count(y.focus)));
            }
            if y.hottest_c.is_some_and(|t| t >= HOT_C) {
                let t = y.hottest_c.unwrap_or_default().round() as u32;
                candidates.push(Quip::new(Topic::HotDay, format!("Yesterday the computer got as hot as {t} °C."), Detail::Count(t)));
            }
            if y.claude_s >= CLAUDE_DAY_S {
                let h = y.claude_s / 3600;
                candidates.push(Quip::new(Topic::ClaudeDay, format!("Yesterday Claude Code worked for {h} hours on the user's projects."), Detail::Hours(h)));
            }
            if y.music_s >= MUSIC_DAY_S {
                let h = y.music_s / 3600;
                candidates.push(Quip::new(Topic::MusicDay, format!("Yesterday music played on the computer for {h} hours."), Detail::Hours(h)));
            }
        }
        if let Some(m) = today.first_min.filter(|m| *m < EARLY_MIN) {
            candidates.push(Quip::new(Topic::EarlyBird, format!("The user is at the computer early today, since {}.", clock(m)), Detail::Clock(m)));
        }
        match diary.weekday() {
            Some(Weekday::Sat | Weekday::Sun) => {
                candidates.push(Quip::new(Topic::Weekend, "It's the weekend and the user is at the computer anyway.".into(), Detail::None))
            }
            Some(Weekday::Mon) => candidates.push(Quip::new(Topic::Monday, "It's Monday morning.".into(), Detail::None)),
            _ => {}
        }
        candidates.into_iter().find(|q| self.fresh(q.topic))
    }

    fn on_return(&self, away_s: u32, hour: u32) -> Option<Quip> {
        let quip = if away_s >= LONG_AWAY_S {
            Quip::new(Topic::LongAway, format!("The user is back after {} hours away.", away_s / 3600), Detail::Hours(away_s / 3600))
        } else if (12..15).contains(&hour) && away_s >= 30 * 60 {
            Quip::new(Topic::Lunch, "The user is back from what was probably lunch.".into(), Detail::None)
        } else {
            return None;
        };
        self.fresh(quip.topic).then_some(quip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diary::Seen;
    use chrono::NaiveDate;

    fn at(day: u32, h: u32, m: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap().and_hms_opt(h, m, s).unwrap()
    }

    fn seen(idle: u32) -> Seen {
        Seen { idle_s: Some(idle), ..Default::default() }
    }

    /// A Thursday the 8th until 2:10 on the 9th.
    fn late_diary() -> Diary {
        let mut diary = Diary::open(None);
        diary.observe(at(8, 10, 0, 0), &seen(1), 1);
        diary.observe(at(9, 2, 10, 0), &seen(1), 1);
        diary
    }

    #[test]
    fn the_morning_after_a_late_night() {
        let mut diary = late_diary();
        let mut quips = Quips::default();
        let mut said = None;
        for s in 0..120 {
            diary.observe(at(9, 9, 0, 0) + chrono::Duration::seconds(s), &seen(1), 1);
            if let Some(q) = quips.consider(&diary, at(9, 9, 0, 0) + chrono::Duration::seconds(s), QuipLevel::Rare, false) {
                assert!(said.is_none(), "said twice");
                said = Some(q);
            }
        }
        let quip = said.expect("a quip");
        assert_eq!(quip.topic, Topic::LateNight);
        assert!(quip.fact.contains("2:10"));
        assert_eq!(quip.fallback("it"), "Ieri hai fatto le 2:10, eh. Io ho dormito benissimo.");
    }

    #[test]
    fn nothing_to_say_says_nothing() {
        let mut diary = Diary::open(None);
        let mut quips = Quips::default();
        // Thursday, an ordinary day.
        for s in 0..120 {
            let t = at(8, 9, 0, 0) + chrono::Duration::seconds(s);
            diary.observe(t, &seen(1), 1);
            assert_eq!(quips.consider(&diary, t, QuipLevel::Often, false), None);
        }
    }

    #[test]
    fn off_quiet_hours_and_busy() {
        let mut diary = late_diary();
        let mut quips = Quips::default();
        for s in 0..120 {
            let t = at(9, 9, 0, 0) + chrono::Duration::seconds(s);
            diary.observe(t, &seen(1), 1);
            assert_eq!(quips.consider(&diary, t, QuipLevel::Off, false), None);
            assert_eq!(quips.consider(&diary, t, QuipLevel::Rare, true), None);
        }
    }

    #[test]
    fn rare_is_once_a_day() {
        let mut diary = late_diary();
        let mut quips = Quips::default();
        let mut count = 0;
        // Morning, then back from a long lunch, then a long stretch.
        let mut t = at(9, 9, 0, 0);
        let mut step = |diary: &mut Diary, quips: &mut Quips, t: NaiveDateTime, idle: u32| {
            diary.observe(t, &seen(idle), 1);
            if quips.consider(diary, t, QuipLevel::Rare, false).is_some() {
                count += 1;
            }
        };
        for _ in 0..120 {
            step(&mut diary, &mut quips, t, 1);
            t += chrono::Duration::seconds(1);
        }
        t = at(9, 13, 0, 0);
        step(&mut diary, &mut quips, t, 3600);
        step(&mut diary, &mut quips, t + chrono::Duration::seconds(1), 1);
        assert_eq!(count, 1);

        let mut often = Quips::default();
        let mut diary = late_diary();
        let mut n = 0;
        for s in 0..120 {
            let t = at(9, 9, 0, 0) + chrono::Duration::seconds(s);
            diary.observe(t, &seen(1), 1);
            n += often.consider(&diary, t, QuipLevel::Often, false).is_some() as u32;
        }
        diary.observe(at(9, 13, 0, 0), &seen(4 * 3600), 1);
        often.consider(&diary, at(9, 13, 0, 0), QuipLevel::Often, false);
        diary.observe(at(9, 13, 0, 1), &seen(1), 1);
        n += often.consider(&diary, at(9, 13, 0, 1), QuipLevel::Often, false).is_some() as u32;
        assert_eq!(n, 2);
    }
}
