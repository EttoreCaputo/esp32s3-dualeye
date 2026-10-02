//! The voice agent: what the host makes of a transcript with a local
//! language model ([`Llm`]).
//!
//! [`Agent::respond`] sends the words to the model with the tools a
//! [`Toolbox`] offers (the board's, plus `get_metrics`) and `get_time`, runs
//! the tool calls it asks for and feeds their results back, until it answers
//! in words: that answer is spoken. A question about the time comes with the
//! time already read, as if the model had called `get_time`, and one about
//! the sensors with `get_metrics`. The last few exchanges are kept, so "and
//! on the right too" works, and forgotten after a few minutes of quiet.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::Local;
use serde::Serialize;
use serde_json::{Value, json};

use crate::intents;
use crate::link::Tool;
use crate::llm::{Llm, LlmError};

/// Rounds of tool calls before the model has to answer.
const MAX_ROUNDS: usize = 4;
/// Exchanges kept for follow-ups.
const MEMORY_TURNS: usize = 4;
/// Quiet after which the conversation starts over.
const MEMORY_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_TOKENS: u32 = 256;

const SYSTEM_PROMPT: &str = "\
You are DualEye, a cute desk pet with two round screens. \
The user can speaks Italian or English.

Rules:
1. To do something, call a tool. Never say it is done without calling it.
2. Pick the tool:
- timer, countdown: set_timer
- ricordami, remind me: set_reminder
- pomodoro: pomodoro
- stop, cancel or pause a timer: control_timer
- watch face: set_face
- music: play, pause, next or previous song: media_control
- what song is playing: now_playing
- scrivi, write: show_text
- temperature, load, fans, memory: get_metrics
- time or date: get_time
- louder, quieter, brighter, dimmer: get_state, then set the new value
3. Call each tool once. Never make numbers up.
4. Then reply with one short sentence in the user's language, plain words for speech: no markdown, no emoji. \
If you can't do it, say so.";

/// Where the agent's tools come from, and what runs them.
pub trait Toolbox {
    /// The tools the model may call, besides `get_time`.
    fn tools(&self) -> Vec<Tool>;
    /// Run one: its result as text for the model, or what went wrong.
    fn call(&self, name: &str, arguments: &Value) -> Result<String, String>;
}

/// A tool call the model made.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Action {
    pub tool: String,
    pub arguments: Value,
    /// What the tool said (or why it failed).
    pub result: String,
    pub ok: bool,
}

/// What came of one transcript.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Turn {
    pub reply: String,
    pub actions: Vec<Action>,
    /// From the request to the end of the first tool call that changed
    /// something (not a read like `get_state`), if any.
    pub first_action_ms: Option<u64>,
    /// Requests to the model.
    pub rounds: u32,
    /// Time spent in the model, out of `elapsed_ms`.
    pub llm_ms: u64,
    pub elapsed_ms: u64,
}

#[derive(Default)]
struct Memory {
    /// Each exchange's messages: the user's, the tool calls and results, the reply.
    turns: Vec<Vec<Value>>,
    last: Option<Instant>,
}

pub struct Agent {
    llm: std::sync::Arc<Llm>,
    memory: Mutex<Memory>,
    /// The server has the system prompt and the tools in its cache.
    primed: AtomicBool,
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent").field("llm", &self.llm).finish_non_exhaustive()
    }
}

/// Read-only tools: calling one isn't an action for [`Turn::first_action_ms`].
pub fn is_read_only(tool: &str) -> bool {
    tool.starts_with("get_") || tool == "now_playing"
}

impl Agent {
    pub fn new(llm: std::sync::Arc<Llm>) -> Self {
        Self { llm, memory: Mutex::default(), primed: AtomicBool::new(false) }
    }

    /// Have the server read the system prompt and the tools, which every
    /// request starts with, so the first command doesn't wait for that
    /// (about 1,400 tokens: seconds on a CPU). Once per agent.
    pub fn prime(&self, toolbox: &dyn Toolbox) -> Result<(), LlmError> {
        if self.primed.load(Ordering::Relaxed) {
            return Ok(());
        }
        let tools = toolbox.tools();
        if tools.is_empty() {
            // No board yet: the prompt would be another one.
            return Ok(());
        }
        let request = json!({
            "messages": [{"role": "system", "content": SYSTEM_PROMPT}, {"role": "user", "content": "Ciao"}],
            "tools": tools.iter().map(function).chain([time_tool()]).collect::<Vec<_>>(),
            "max_tokens": 1,
            "chat_template_kwargs": {"enable_thinking": false},
        });
        self.llm.chat(&request)?;
        self.primed.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn llm(&self) -> &Llm {
        &self.llm
    }

    /// Start over: the next transcript has no earlier ones to refer to.
    pub fn forget(&self) {
        *self.memory.lock().unwrap() = Memory::default();
    }

    /// Answer `text` (heard in `language`, `it` or `en`), calling tools on the way.
    pub fn respond(&self, text: &str, language: &str, toolbox: &dyn Toolbox) -> Result<Turn, LlmError> {
        let started = Instant::now();
        let tools: Vec<Value> = toolbox.tools().iter().map(function).chain([time_tool()]).collect();
        let mut messages = vec![json!({"role": "system", "content": SYSTEM_PROMPT})];
        {
            let mut memory = self.memory.lock().unwrap();
            if memory.last.is_some_and(|t| t.elapsed() > MEMORY_TIMEOUT) {
                memory.turns.clear();
            }
            messages.extend(memory.turns.iter().flatten().cloned());
        }
        // Models answer "what time is it" from thin air rather than call
        // get_time, but read a call made for them. Only for such a question:
        // a tool result already there makes them call fewer tools. After the
        // history, so the cached prompt before it stays valid.
        if intents::asks_time(text) {
            messages.push(json!({"role": "assistant", "content": "", "tool_calls": [{"id": "clock", "type": "function", "function": {"name": "get_time", "arguments": "{}"}}]}));
            messages.push(json!({"role": "tool", "tool_call_id": "clock", "content": now()}));
        }
        // Likewise "is the CPU hot?": with more tools to pick from they'd
        // rather ask back than call get_metrics.
        if intents::asks_metrics(text)
            && let Ok(metrics) = toolbox.call("get_metrics", &json!({}))
        {
            messages.push(json!({"role": "assistant", "content": "", "tool_calls": [{"id": "sensors", "type": "function", "function": {"name": "get_metrics", "arguments": "{}"}}]}));
            messages.push(json!({"role": "tool", "tool_call_id": "sensors", "content": metrics}));
        }
        let first = messages.len();
        messages.push(json!({"role": "user", "content": text}));

        let mut actions: Vec<Action> = Vec::new();
        let (mut first_action_ms, mut llm_ms, mut rounds) = (None, 0u64, 0u32);
        let mut reply = String::new();
        for round in 0..MAX_ROUNDS {
            let last = round + 1 == MAX_ROUNDS;
            let request = json!({
                "messages": messages,
                "tools": tools,
                // The last round has to answer in words.
                "tool_choice": if last { "none" } else { "auto" },
                "temperature": 0.2,
                "top_p": 0.8,
                "top_k": 20,
                "max_tokens": MAX_TOKENS,
                "seed": 7,
                "chat_template_kwargs": {"enable_thinking": false},
            });
            let asked = Instant::now();
            let message = self.llm.chat(&request)?;
            llm_ms += asked.elapsed().as_millis() as u64;
            rounds += 1;
            let mut content = message["content"].as_str().unwrap_or_default();
            let mut calls: Vec<Value> = message["tool_calls"].as_array().cloned().unwrap_or_default();
            if calls.is_empty() {
                calls = text_tool_calls(content);
                if !calls.is_empty() {
                    // Not in the history as text, or it writes them so again.
                    content = "";
                }
            }
            if calls.is_empty() {
                reply = spoken(content, language);
                break;
            }
            messages.push(json!({"role": "assistant", "content": content, "tool_calls": calls}));
            for (i, call) in calls.iter().enumerate() {
                let name = call.pointer("/function/name").and_then(Value::as_str).unwrap_or_default().to_string();
                let arguments = match call.pointer("/function/arguments") {
                    Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
                    Some(v @ Value::Object(_)) => v.clone(),
                    _ => json!({}),
                };
                // Small models like to do the same thing twice, or to read
                // the time over and over instead of doing what was asked.
                let before = actions.iter().find(|a| a.ok && a.tool == name && a.arguments == arguments);
                let repeat = before.is_some();
                let result = if let Some(before) = before {
                    Ok(if is_read_only(&name) { format!("already read: {}. Now do what the user asked.", before.result) } else { "already done".to_string() })
                } else if name == "get_time" {
                    Ok(now())
                } else {
                    toolbox.call(&name, &arguments)
                };
                let (ok, result) = match result {
                    Ok(r) => (true, r),
                    Err(e) => (false, format!("error: {e}")),
                };
                if ok && !repeat && !is_read_only(&name) && first_action_ms.is_none() {
                    first_action_ms = Some(started.elapsed().as_millis() as u64);
                }
                let id = call["id"].as_str().map_or_else(|| format!("call_{round}_{i}"), str::to_string);
                messages.push(json!({"role": "tool", "tool_call_id": id, "content": result}));
                if !repeat {
                    actions.push(Action { tool: name, arguments, result, ok });
                }
            }
        }
        if reply.is_empty() {
            // Only tool calls and no words: say whether they worked.
            let it = language == "it";
            let done = actions.iter().all(|a| a.ok) && actions.iter().any(|a| !is_read_only(&a.tool));
            reply = match (done, it) {
                (true, true) => "Fatto.",
                (true, false) => "Done.",
                (false, true) => "Non ci sono riuscito.",
                (false, false) => "That didn't work.",
            }
            .into();
        }
        messages.push(json!({"role": "assistant", "content": reply}));

        let mut memory = self.memory.lock().unwrap();
        memory.turns.push(messages.split_off(first));
        let excess = memory.turns.len().saturating_sub(MEMORY_TURNS);
        memory.turns.drain(..excess);
        memory.last = Some(Instant::now());
        Ok(Turn { reply, actions, first_action_ms, rounds, llm_ms, elapsed_ms: started.elapsed().as_millis() as u64 })
    }
}

/// A board or host tool as an OpenAI-style function.
fn function(tool: &Tool) -> Value {
    let mut parameters = tool.input_schema.clone();
    if !parameters.is_object() {
        parameters = json!({"type": "object", "properties": {}});
    }
    json!({"type": "function", "function": {"name": tool.name, "description": tool.description, "parameters": parameters}})
}

fn time_tool() -> Value {
    json!({"type": "function", "function": {
        "name": "get_time",
        "description": "The current local time, date and day of the week. You don't know them without it.",
        "parameters": {"type": "object", "properties": {}},
    }})
}

fn now() -> String {
    Local::now().format("%A %-d %B %Y, %H:%M").to_string()
}

/// Tool calls the model wrote as text instead of in its template's tags,
/// as JSON (Qwen3 1.7B now and then: `{"name": "set_face", "arguments": {…}}`)
/// or as XML (Qwen3.5: `<tool_call><function=set_face><parameter=face>rings</parameter></function></tool_call>`).
fn text_tool_calls(content: &str) -> Vec<Value> {
    let call = |name: &str, arguments: Value| json!({"type": "function", "function": {"name": name, "arguments": arguments.to_string()}});
    let body = content.trim().trim_start_matches("<tool_call>").trim_end_matches("</tool_call>").trim();
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        return match v["name"].as_str() {
            Some(name) => vec![call(name, v.get("arguments").cloned().unwrap_or_else(|| json!({})))],
            None => vec![],
        };
    }
    content
        .split("<function=")
        .skip(1)
        .filter_map(|part| {
            let (name, rest) = part.split_once('>')?;
            let rest = rest.split("</function>").next().unwrap_or(rest);
            let mut arguments = serde_json::Map::new();
            for param in rest.split("<parameter=").skip(1) {
                let Some((key, value)) = param.split_once('>') else { continue };
                let value = value.split("</parameter>").next().unwrap_or(value).trim();
                // Numbers and enums as JSON when they are, else the text.
                let value = serde_json::from_str(value).unwrap_or_else(|_| json!(value));
                arguments.insert(key.trim().to_string(), value);
            }
            let name = name.trim();
            (!name.is_empty()).then(|| call(name, Value::Object(arguments)))
        })
        .collect()
}

/// Text fit for the speaker: no thinking left over, no markdown, degrees
/// in words ("53°C" reads badly).
fn spoken(content: &str, language: &str) -> String {
    let text = match content.rfind("</think>") {
        Some(i) => &content[i + "</think>".len()..],
        None => content,
    };
    let degrees = if language == "it" { " gradi" } else { " degrees" };
    let text = text.replace("°C", degrees).replace(" °", degrees).replace('°', degrees);
    let text: String = text.chars().filter(|c| !matches!(c, '*' | '#' | '`' | '_')).collect();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spoken_text_is_plain() {
        assert_eq!(spoken("<think>\n\n</think>\n\n**Fatto!** La faccia `rings` è a sinistra.", "it"), "Fatto! La faccia rings è a sinistra.");
        assert_eq!(spoken("  Done.\n", "en"), "Done.");
        assert_eq!(spoken("La CPU è a 53°C.", "it"), "La CPU è a 53 gradi.");
        assert_eq!(spoken("The GPU is at 41.2 °C.", "en"), "The GPU is at 41.2 degrees.");
    }

    #[test]
    fn tool_calls_written_as_text() {
        let args = |call: &Value| serde_json::from_str::<Value>(call["function"]["arguments"].as_str().unwrap()).unwrap();
        let calls = text_tool_calls(r#"{"name": "set_face", "arguments": {"face": "classic", "screen": "left"}}"#);
        assert_eq!(calls[0]["function"]["name"], "set_face");
        assert_eq!(args(&calls[0])["screen"], "left");
        assert!(text_tool_calls("Fatto.").is_empty());

        let calls = text_tool_calls("<tool_call>\n<function=get_time>\n</function>\n</tool_call>");
        assert_eq!(calls[0]["function"]["name"], "get_time");
        assert_eq!(args(&calls[0]), json!({}));
        let calls = text_tool_calls(
            "<tool_call>\n<function=set_timer>\n<parameter=minutes>\n10\n</parameter>\n<parameter=label>\npasta\n</parameter>\n</function>\n</tool_call>\n\
             <tool_call>\n<function=pomodoro>\n<parameter=action>\nstart\n</parameter>\n</function>\n</tool_call>",
        );
        assert_eq!(calls.len(), 2);
        assert_eq!(args(&calls[0]), json!({"minutes": 10, "label": "pasta"}));
        assert_eq!(args(&calls[1]), json!({"action": "start"}));
    }

    #[test]
    fn tools_become_functions() {
        let t = Tool { name: "get_state".into(), description: "State".into(), input_schema: Value::Null };
        assert_eq!(function(&t)["function"]["parameters"], json!({"type": "object", "properties": {}}));
    }
}
