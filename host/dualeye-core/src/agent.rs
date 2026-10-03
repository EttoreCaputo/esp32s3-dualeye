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

/// The system prompt: who the model plays (`{personality}`, from
/// [`personality_prompt`]), the tools it has (`{tools}`, from the tools
/// themselves) and how to behave. Every request starts with it, so it is
/// cached: keep it the same from one request to the next.
const SYSTEM_PROMPT: &str = "\
You are DualEye, a small companion that lives on the user's desk: a board with two round screens, \
left and right, a microphone and a speaker, connected to the user's computer. The user talks to you \
by voice. Their words reach you through speech recognition, so they may contain small mistakes: \
go by what they mean.

{personality}
Your personality shapes how you talk, never what you do.

Your tools:
{tools}

How to behave:
- When the user asks for something one of your tools does, call it, then confirm briefly. \
Never say something is done unless the tool call worked: a result starting with \"error\" means it failed.
- When the answer depends on a fact (a measurement, the time, a setting, a timer), read it with \
a tool instead of guessing.
- For a change relative to the current value, read the current value first, then set the new one.
- Call each tool once per request, with what the user asked for.
- When the user is just chatting (a greeting, a question about you, a joke, small talk), \
answer in character without calling any tool.
- When they ask for something no tool can do, say so briefly.
- When a request is unclear, ask one short question.

How to reply:
- Work out which language the user is speaking and always reply in that same language.
- Keep it to one or two short sentences, meant to be spoken aloud: no markdown, lists, emoji or symbols.";

/// A personality for [`Agent::set_personality`]: one of the app's presets,
/// or `custom` with the person's own words.
pub fn personality_prompt(id: &str, custom: &str) -> String {
    let preset = match id {
        "playful" => "Personality: playful and funny. You like light jokes and puns, and tease the user gently.",
        "calm" => "Personality: calm and gentle. You speak softly and reassuringly, never in a hurry.",
        "sassy" => "Personality: a sassy cat. Dry humour and a little sarcasm, but you always do what is asked.",
        "butler" => "Personality: a refined butler. Polite and formal, using the formal form of address where the language has one.",
        "minimal" => "Personality: brief and practical. Only say what you did or what was asked, no small talk.",
        "custom" if !custom.trim().is_empty() => {
            let custom: String = custom.trim().chars().take(MAX_PERSONALITY).collect();
            return format!("Personality, as the user described it: {custom}");
        }
        _ => "Personality: a cute, cheerful desk pet. Warm, kind and a little playful.",
    };
    preset.to_string()
}

/// Characters of a custom personality kept for the prompt.
pub const MAX_PERSONALITY: usize = 300;

/// [`SYSTEM_PROMPT`] with a personality and the tools (OpenAI-style functions).
fn system_prompt(personality: &str, tools: &[Value]) -> String {
    let list: Vec<String> = tools
        .iter()
        .map(|t| {
            let f = &t["function"];
            format!("- {}: {}", f["name"].as_str().unwrap_or_default(), f["description"].as_str().unwrap_or_default())
        })
        .collect();
    SYSTEM_PROMPT.replace("{personality}", personality).replace("{tools}", &list.join("\n"))
}

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
    /// The personality's part of the system prompt.
    personality: Mutex<String>,
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
        Self { llm, memory: Mutex::default(), personality: Mutex::new(personality_prompt("", "")), primed: AtomicBool::new(false) }
    }

    /// Talk with this personality ([`personality_prompt`]) from the next transcript on.
    pub fn set_personality(&self, personality: &str) {
        let mut current = self.personality.lock().unwrap();
        if *current != personality {
            *current = personality.to_string();
            // Another prompt: the server's cache has the old one.
            self.primed.store(false, Ordering::Relaxed);
        }
    }

    /// The system prompt every request starts with, for these tools.
    pub fn system_prompt(&self, tools: &[Value]) -> String {
        system_prompt(&self.personality.lock().unwrap(), tools)
    }

    /// Have the server read the system prompt and the tools, which every
    /// request starts with, so the first command doesn't wait for that
    /// (about 1,400 tokens: seconds on a CPU). Once per agent; not for a
    /// cloud model, where it would only spend the free tokens.
    pub fn prime(&self, toolbox: &dyn Toolbox) -> Result<(), LlmError> {
        if self.primed.load(Ordering::Relaxed) || !self.llm.config().is_local() {
            return Ok(());
        }
        let tools = toolbox.tools();
        if tools.is_empty() {
            // No board yet: the prompt would be another one.
            return Ok(());
        }
        let tools: Vec<Value> = tools.iter().map(function).chain([time_tool()]).collect();
        let request = json!({
            "messages": [{"role": "system", "content": self.system_prompt(&tools)}, {"role": "user", "content": "Ciao"}],
            "tools": tools,
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
        let mut messages = vec![json!({"role": "system", "content": self.system_prompt(&tools)})];
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
            // A call cut off by max_tokens has arguments that aren't JSON,
            // which the server refuses to read back in the history.
            let parsed: Vec<Value> = calls
                .iter()
                .map(|call| match call.pointer("/function/arguments") {
                    Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
                    Some(v @ Value::Object(_)) => v.clone(),
                    _ => json!({}),
                })
                .collect();
            for (call, arguments) in calls.iter_mut().zip(&parsed) {
                call["function"]["arguments"] = json!(arguments.to_string());
            }
            messages.push(json!({"role": "assistant", "content": content, "tool_calls": calls}));
            for (i, (call, arguments)) in calls.iter().zip(parsed).enumerate() {
                let name = call.pointer("/function/name").and_then(Value::as_str).unwrap_or_default().to_string();
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
    fn personality_goes_in_the_prompt() {
        let tools = [time_tool()];
        let prompt = system_prompt(&personality_prompt("butler", ""), &tools);
        assert!(prompt.contains("refined butler") && !prompt.contains("{personality}"));
        assert!(prompt.contains("- get_time: The current local time") && !prompt.contains("{tools}"));
        assert!(personality_prompt("unknown", "").contains("cute"));
        assert!(personality_prompt("custom", "  ").contains("cute"));
        let long = "a".repeat(MAX_PERSONALITY + 50);
        assert!(personality_prompt("custom", &long).ends_with(&"a".repeat(MAX_PERSONALITY)));
    }

    #[test]
    fn tools_become_functions() {
        let t = Tool { name: "get_state".into(), description: "State".into(), input_schema: Value::Null };
        assert_eq!(function(&t)["function"]["parameters"], json!({"type": "object", "properties": {}}));
    }
}
