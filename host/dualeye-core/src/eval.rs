//! The voice agent's eval set (`eval/commands.json`, about 50 commands in
//! Italian and English) and what runs it: `dualeye eval`.
//!
//! Each case is said to a [`SimBoard`], a board simulated in memory with
//! the real firmware's tool list (`eval/board-tools.json`) and fixed sensor
//! readings. A case passes when the board ends up as the case expects, with
//! everything else unchanged, however the model got there (one `set_face`
//! with `both`, or one per screen), and a question's answer has the number
//! it should. The M5 rules run the same set, for comparison.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Instant;

use chrono::{Local, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agent::{Action, Agent, Toolbox};
use crate::intents::{self, Context};
use crate::link::Tool;
use crate::llm::LlmError;
use crate::snapshot::Snapshot;

const COMMANDS: &str = include_str!("../eval/commands.json");
const HOLDOUT: &str = include_str!("../eval/holdout.json");
const BOARD_TOOLS: &str = include_str!("../eval/board-tools.json");

#[derive(Debug, Clone, Deserialize)]
pub struct Case {
    pub lang: String,
    /// One sentence, or a short conversation: only the end state counts.
    pub say: Vec<String>,
    /// Settings that must have changed, and to what.
    pub expect: BTreeMap<String, Value>,
    /// For a question: the reply must contain one of these.
    #[serde(default)]
    pub reply: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EvalSet {
    pub start: BTreeMap<String, Value>,
    pub metrics: Value,
    pub cases: Vec<Case>,
}

impl EvalSet {
    /// The main set, `eval/commands.json`.
    pub fn builtin() -> Self {
        serde_json::from_str(COMMANDS).expect("eval/commands.json")
    }

    /// `commands` (the main set), `holdout` (other phrasings, not used to
    /// tune the prompt), or a JSON file in the same format.
    pub fn named(name: &str) -> Result<Self, String> {
        match name {
            "commands" => Ok(Self::builtin()),
            "holdout" => Ok(serde_json::from_str(HOLDOUT).expect("eval/holdout.json")),
            path => {
                let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
                serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))
            }
        }
    }
}

/// A board in memory with the settings the tools change.
pub struct SimBoard {
    tools: Vec<Tool>,
    state: Mutex<BTreeMap<String, Value>>,
    metrics: Snapshot,
}

impl SimBoard {
    pub fn new(set: &EvalSet) -> Self {
        let tools: Vec<Tool> = serde_json::from_str(BOARD_TOOLS).expect("eval/board-tools.json");
        let mut metrics: Snapshot = serde_json::from_value(json!({"v": 2, "ts": 0})).unwrap();
        metrics.cpu = serde_json::from_value(set.metrics["cpu"].clone()).unwrap_or_default();
        metrics.gpu = serde_json::from_value(set.metrics["gpu"].clone()).unwrap_or_default();
        metrics.fans = serde_json::from_value(set.metrics["fans"].clone()).unwrap_or_default();
        Self { tools: crate::voice::voice_tools(tools), state: Mutex::new(set.start.clone()), metrics }
    }

    pub fn state(&self) -> BTreeMap<String, Value> {
        self.state.lock().unwrap().clone()
    }

    fn get(&self, key: &str) -> Value {
        self.state.lock().unwrap().get(key).cloned().unwrap_or(Value::Null)
    }

    /// The `get_state` shape the voice agent sees ([`crate::voice::trim_state`]).
    fn board_state(&self) -> Value {
        let screen = |s: &str| json!({"face": self.get(&format!("{s}.face")), "rotation": self.get(&format!("{s}.rotation")), "brightness": self.get(&format!("{s}.brightness"))});
        let wake = match self.get("wake_word").as_str() {
            Some("hiesp") => "Hi ESP",
            _ => "Alexa",
        };
        json!({
            "screens": {"left": screen("left"), "right": screen("right")},
            "voice": {"wake_word": wake, "wake_words": ["alexa", "hiesp"], "muted": self.get("muted")},
            "audio": {"volume": self.get("volume")},
        })
    }
}

fn screens(args: &Value) -> Result<(Vec<&'static str>, &'static str), String> {
    match args.get("screen").map(|s| s.as_str()) {
        None | Some(Some("both")) => Ok((vec!["left", "right"], "both")),
        Some(Some("left")) => Ok((vec!["left"], "left")),
        Some(Some("right")) => Ok((vec!["right"], "right")),
        _ => Err("screen must be left, right or both".into()),
    }
}

fn label(which: &str) -> &str {
    if which == "both" { "both screens" } else { which }
}

fn int(args: &Value, key: &str) -> Result<i64, String> {
    args[key].as_i64().or_else(|| args[key].as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)).ok_or_else(|| format!("{key} must be an integer"))
}

impl Toolbox for SimBoard {
    fn tools(&self) -> Vec<Tool> {
        self.tools.clone()
    }

    /// Like the firmware (`main/board_tools.c`): the same checks and answers.
    fn call(&self, name: &str, args: &Value) -> Result<String, String> {
        let mut state = self.state.lock().unwrap();
        let mut set = |k: String, v: Value| {
            state.insert(k, v);
        };
        match name {
            "set_face" => {
                let face = args["face"].as_str().filter(|f| ["classic", "rings", "plus", "bar", "claude", "clawd"].contains(f));
                let face = face.ok_or("face must be one of classic, rings, plus, bar, claude, clawd")?;
                let (on, which) = screens(args)?;
                on.iter().for_each(|s| set(format!("{s}.face"), json!(face)));
                Ok(format!("{}: {face}", label(which)))
            }
            "set_rotation" => {
                let deg = int(args, "degrees").ok().filter(|d| [0, 90, 180, 270].contains(d)).ok_or("degrees must be 0, 90, 180 or 270")?;
                let (on, which) = screens(args)?;
                on.iter().for_each(|s| set(format!("{s}.rotation"), json!(deg)));
                Ok(format!("{}: turned {deg} degrees", label(which)))
            }
            "set_brightness" => {
                let pct = int(args, "percent").ok().filter(|p| (0..=100).contains(p)).ok_or("percent must be 0 to 100")?;
                let (on, which) = screens(args)?;
                on.iter().for_each(|s| set(format!("{s}.brightness"), json!(pct)));
                Ok(format!("{}: brightness {pct}%", label(which)))
            }
            "show_text" => {
                let text = args["text"].as_str().filter(|t| !t.is_empty()).ok_or("text must be a non-empty string")?;
                let seconds = if args.get("seconds").is_some() { int(args, "seconds")? } else { 4 };
                if !(1..=30).contains(&seconds) {
                    return Err("seconds must be 1 to 30".into());
                }
                let (_, which) = screens(args)?;
                set("text".into(), json!(text));
                set("text.screen".into(), json!(which));
                Ok(format!("{}: showing it for {seconds} s", label(which)))
            }
            "set_wake_word" => {
                let word = args["word"].as_str().ok_or("word must be a string")?;
                let shown = match word {
                    "alexa" => "Alexa",
                    "hiesp" => "Hi ESP",
                    other => return Err(format!("no model for \"{other}\" on this board")),
                };
                set("wake_word".into(), json!(word));
                Ok(format!("wake word is now \"{shown}\""))
            }
            "set_mic" => {
                let muted = args["muted"].as_bool().ok_or("muted must be true or false")?;
                set("muted".into(), json!(muted));
                Ok(if muted { "microphone muted".into() } else { "listening".into() })
            }
            "set_volume" => {
                let pct = int(args, "percent").ok().filter(|p| (0..=100).contains(p)).ok_or("percent must be 0 to 100")?;
                set("volume".into(), json!(pct));
                Ok(format!("speaker volume {pct}%"))
            }
            "get_state" => {
                drop(state);
                Ok(self.board_state().to_string())
            }
            "get_metrics" => Ok(self.metrics.metrics_json().to_string()),
            other => Err(format!("unknown tool {other}")),
        }
    }
}

/// Does `got` satisfy `want` (see `about` in `eval/commands.json`)?
fn matches(want: &Value, got: &Value) -> bool {
    let num = |v: &Value| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()));
    match want.as_str() {
        Some(w) if w.starts_with('>') => num(got).zip(w[1..].parse::<f64>().ok()).is_some_and(|(g, w)| g > w),
        Some(w) if w.starts_with('<') => num(got).zip(w[1..].parse::<f64>().ok()).is_some_and(|(g, w)| g < w),
        Some(w) if w.starts_with('~') => got.as_str().is_some_and(|g| g.to_lowercase().contains(&w[1..].to_lowercase())),
        _ => match (num(want), num(got)) {
            (Some(a), Some(b)) => a == b,
            _ => want == got,
        },
    }
}

/// What's wrong with the end state, if anything.
fn check_state(start: &BTreeMap<String, Value>, end: &BTreeMap<String, Value>, expect: &BTreeMap<String, Value>) -> Option<String> {
    let mut wrong = Vec::new();
    for (key, want) in expect {
        let got = end.get(key).unwrap_or(&Value::Null);
        if !matches(want, got) {
            wrong.push(format!("{key} = {got} (want {want})"));
        }
    }
    for (key, before) in start {
        // Text shown and gone again isn't a change that sticks.
        if !expect.contains_key(key) && !key.starts_with("text") && end.get(key) != Some(before) {
            wrong.push(format!("{key} changed to {}", end.get(key).unwrap_or(&Value::Null)));
        }
    }
    (!wrong.is_empty()).then(|| wrong.join(", "))
}

fn check_reply(reply: &str, wants: &[String]) -> Option<String> {
    if wants.is_empty() {
        return None;
    }
    let hour = Local::now().hour();
    let lower = reply.to_lowercase();
    let found = wants.iter().any(|w| match w.as_str() {
        "{hour}" => {
            let digits: Vec<u32> = lower.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse().ok()).collect();
            // Also the next hour, for a case run on the stroke of it.
            digits.iter().any(|d| [hour, (hour + 1) % 24].iter().any(|h| *d == *h || (*h % 12 == *d % 12 && *d <= 12)))
        }
        w => lower.contains(&w.to_lowercase()),
    });
    (!found).then(|| format!("reply lacks {}", wants.join(" / ")))
}

#[derive(Debug, Clone, Serialize)]
pub struct CaseResult {
    pub lang: String,
    pub say: Vec<String>,
    pub pass: bool,
    /// What was wrong.
    pub why: Option<String>,
    /// The reply to the last sentence.
    pub reply: String,
    /// Every tool call, all sentences.
    pub actions: Vec<Action>,
    /// Last sentence: until the first change on the board, or the reply for a question.
    pub first_ms: u64,
    /// Last sentence: until the reply.
    pub reply_ms: u64,
    /// Requests to the model for the last sentence.
    pub rounds: u32,
    /// The reply looks like another language than the question's.
    pub wrong_language: bool,
}

/// Who answers: the language model, or the M5 rules.
pub enum Responder<'a> {
    Llm(&'a Agent),
    Rules,
}

/// Have the model read its prompt before the first case, as the voice
/// pipeline does while the first command is being said.
pub fn prime(set: &EvalSet, responder: &Responder) -> Result<(), LlmError> {
    match responder {
        Responder::Llm(agent) => agent.prime(&SimBoard::new(set)),
        Responder::Rules => Ok(()),
    }
}

pub fn run_case(set: &EvalSet, case: &Case, responder: &Responder) -> Result<CaseResult, LlmError> {
    let board = SimBoard::new(set);
    let mut actions = Vec::new();
    let mut last = (String::new(), 0, 0, 0);
    if let Responder::Llm(agent) = responder {
        agent.forget();
    }
    for text in &case.say {
        match responder {
            Responder::Llm(agent) => {
                let turn = agent.respond(text, &case.lang, &board)?;
                actions.extend(turn.actions);
                last = (turn.reply, turn.first_action_ms.unwrap_or(turn.elapsed_ms), turn.elapsed_ms, turn.rounds);
            }
            Responder::Rules => {
                let started = Instant::now();
                let volume = board.get("volume").as_u64().map(|v| v as u8);
                let plan = intents::understand(text, &case.lang, &Context { snapshot: Some(board.metrics.clone()), volume });
                let mut ok = true;
                for (tool, args) in &plan.calls {
                    let result = board.call(tool, args);
                    ok &= result.is_ok();
                    let (ok, result) = match result {
                        Ok(r) => (true, r),
                        Err(e) => (false, e),
                    };
                    actions.push(Action { tool: tool.clone(), arguments: args.clone(), result, ok });
                }
                let ms = started.elapsed().as_millis() as u64;
                last = (if ok { plan.reply } else { plan.failure }, ms, ms, 0);
            }
        }
    }
    let (reply, first_ms, reply_ms, rounds) = last;
    let why = check_state(&set.start, &board.state(), &case.expect).or_else(|| check_reply(&reply, &case.reply));
    let wrong_language = reply.split_whitespace().count() >= 4 && intents::guess_language(&reply) != case.lang;
    Ok(CaseResult { lang: case.lang.clone(), say: case.say.clone(), pass: why.is_none(), why, reply, actions, first_ms, reply_ms, rounds, wrong_language })
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub cases: usize,
    pub passed: usize,
    pub passed_it: (usize, usize),
    pub passed_en: (usize, usize),
    pub wrong_language: usize,
    /// Medians and 90th percentiles, in ms.
    pub first_ms_median: u64,
    pub first_ms_p90: u64,
    pub reply_ms_median: u64,
    pub reply_ms_p90: u64,
}

fn percentile(values: &[u64], p: f64) -> u64 {
    let mut v = values.to_vec();
    v.sort_unstable();
    v.get(((v.len() as f64 - 1.0) * p).round() as usize).copied().unwrap_or(0)
}

pub fn summarize(results: &[CaseResult]) -> Summary {
    let lang = |l: &str| (results.iter().filter(|r| r.lang == l && r.pass).count(), results.iter().filter(|r| r.lang == l).count());
    let first: Vec<u64> = results.iter().map(|r| r.first_ms).collect();
    let reply: Vec<u64> = results.iter().map(|r| r.reply_ms).collect();
    Summary {
        cases: results.len(),
        passed: results.iter().filter(|r| r.pass).count(),
        passed_it: lang("it"),
        passed_en: lang("en"),
        wrong_language: results.iter().filter(|r| r.wrong_language).count(),
        first_ms_median: percentile(&first, 0.5),
        first_ms_p90: percentile(&first, 0.9),
        reply_ms_median: percentile(&reply, 0.5),
        reply_ms_p90: percentile(&reply, 0.9),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_set_parses_and_the_sim_board_follows_the_firmware() {
        let set = EvalSet::builtin();
        assert!(set.cases.len() >= 50);
        assert!(EvalSet::named("holdout").unwrap().cases.len() >= 20);
        let board = SimBoard::new(&set);
        assert!(board.tools().iter().any(|t| t.name == "set_face") && !board.tools().iter().any(|t| t.name == "set_mic"));
        assert_eq!(board.call("set_face", &json!({"face": "rings", "screen": "left"})).unwrap(), "left: rings");
        assert!(board.call("set_face", &json!({"face": "ring"})).is_err());
        assert!(board.call("set_brightness", &json!({"percent": 30, "screen": "sinistra"})).is_err());
        assert_eq!(board.call("set_brightness", &json!({"percent": 30.0})).unwrap(), "both screens: brightness 30%");
        let end = board.state();
        assert_eq!(check_state(&set.start, &end, &BTreeMap::from([("left.face".into(), json!("rings"))])), Some("left.brightness changed to 30, right.brightness changed to 30".into()));
    }

    #[test]
    fn expectations() {
        assert!(matches(&json!(">60"), &json!(80)) && !matches(&json!(">60"), &json!(60)));
        assert!(matches(&json!("~pausa caff"), &json!("Pausa caffe")));
        assert!(matches(&json!(30), &json!(30.0)));
        assert_eq!(check_reply("La CPU è a 52 gradi.", &["52".into()]), None);
        assert!(check_reply("Non lo so.", &["52".into()]).is_some());
    }

    /// The rules of M5 as the baseline: they run in the test suite, the
    /// model only with `dualeye eval`.
    #[test]
    fn rules_baseline_runs() {
        let set = EvalSet::builtin();
        let results: Vec<CaseResult> = set.cases.iter().map(|c| run_case(&set, c, &Responder::Rules).unwrap()).collect();
        let s = summarize(&results);
        assert!(s.passed * 2 > s.cases, "rules pass {} of {}", s.passed, s.cases);
    }
}
