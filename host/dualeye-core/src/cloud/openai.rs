//! [`Api::OpenAi`](super::Api::OpenAi): OpenAI's HTTP API, which Groq and
//! most other services speak.

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{Map, Value};

use super::{Client, CloudError, TranscriptionReply};

const TIMEOUT: Duration = Duration::from_secs(30);
/// What a chat request may carry: llama.cpp's own settings (`top_k`,
/// `chat_template_kwargs`) are refused by most services.
const CHAT_FIELDS: &[&str] = &["messages", "tools", "tool_choice", "temperature", "top_p", "max_tokens", "seed", "stop", "response_format"];

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    // Error statuses are read like any answer, for the provider's message.
    AGENT.get_or_init(|| ureq::Agent::new_with_config(ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(TIMEOUT)).build()))
}

fn url(client: &Client, path: &str) -> String {
    format!("{}{path}", client.model.provider.base_url)
}

fn auth(client: &Client) -> String {
    format!("Bearer {}", client.key)
}

/// The body of a successful answer, or the error it stands for.
fn read(client: &Client, sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Vec<u8>, CloudError> {
    let provider = client.model.provider.name;
    let mut response = sent.map_err(|e| CloudError::Network { provider, why: e.to_string() })?;
    let status = response.status().as_u16();
    let retry_after = response.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<f64>().ok()).map(Duration::from_secs_f64);
    let body = response.body_mut().with_config().limit(32 * 1024 * 1024).read_to_vec().map_err(|e| CloudError::Network { provider, why: e.to_string() })?;
    match status {
        200..=299 => Ok(body),
        401 | 403 => Err(CloudError::BadKey { provider }),
        429 => Err(CloudError::RateLimited { provider, retry_after }),
        _ => Err(CloudError::Status { provider, status, message: error_message(&body) }),
    }
}

/// `{"error": {"message": ...}}`, or the start of whatever it is.
fn error_message(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| String::from_utf8_lossy(body).chars().take(300).collect())
}

fn json(client: &Client, body: &[u8]) -> Result<Value, CloudError> {
    serde_json::from_slice(body).map_err(|e| CloudError::Invalid { provider: client.model.provider.name, why: e.to_string() })
}

pub(super) fn check(client: &Client) -> Result<(), CloudError> {
    read(client, agent().get(url(client, "/models")).header("Authorization", auth(client)).call()).map(|_| ())
}

/// `request` cut down to [`CHAT_FIELDS`], with the model and its params.
fn chat_request(client: &Client, request: &Value) -> Value {
    let mut body: Map<String, Value> = request.as_object().into_iter().flatten().filter(|(k, _)| CHAT_FIELDS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    // A request without tools mustn't choose among them.
    if !body.contains_key("tools") {
        body.remove("tool_choice");
    }
    body.insert("model".into(), client.model.model.model.into());
    if let Ok(Value::Object(params)) = serde_json::from_str::<Value>(client.model.model.params) {
        body.extend(params);
    }
    Value::Object(body)
}

pub(super) fn chat(client: &Client, request: &Value) -> Result<Value, CloudError> {
    let body = serde_json::to_vec(&chat_request(client, request)).map_err(|e| CloudError::Invalid { provider: client.model.provider.name, why: e.to_string() })?;
    let sent = agent().post(url(client, "/chat/completions")).header("Authorization", auth(client)).header("Content-Type", "application/json").send(&body[..]);
    let reply = json(client, &read(client, sent)?)?;
    reply.pointer("/choices/0/message").cloned().ok_or_else(|| CloudError::Invalid {
        provider: client.model.provider.name,
        why: format!("no message in {}", reply.to_string().chars().take(300).collect::<String>()),
    })
}

pub(super) fn transcribe(client: &Client, wav: &[u8], language: Option<&str>, prompt: &str) -> Result<TranscriptionReply, CloudError> {
    let mut fields = vec![("model", client.model.model.model), ("response_format", "verbose_json"), ("temperature", "0"), ("prompt", prompt)];
    if let Some(l) = language {
        fields.push(("language", l));
    }
    let (content_type, body) = crate::stt::multipart(wav, &fields).map_err(|e| CloudError::Invalid { provider: client.model.provider.name, why: e.to_string() })?;
    let sent = agent().post(url(client, "/audio/transcriptions")).header("Authorization", auth(client)).header("Content-Type", content_type).send(&body[..]);
    json(client, &read(client, sent)?)
}

pub(super) fn speak(client: &Client, text: &str) -> Result<Vec<u8>, CloudError> {
    let m = client.model.model;
    let body = serde_json::json!({"model": m.model, "voice": m.voice.unwrap_or_default(), "input": text, "response_format": "wav"}).to_string();
    let sent = agent().post(url(client, "/audio/speech")).header("Authorization", auth(client)).header("Content-Type", "application/json").send(body.as_bytes());
    read(client, sent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::CloudRef;
    use serde_json::json;

    #[test]
    fn chat_request_drops_llama_settings_and_adds_params() {
        let client = Client { model: CloudRef::by_id("groq:openai/gpt-oss-20b").unwrap(), key: "k".into(), voice_id: None };
        let request = json!({"messages": [], "tools": [], "tool_choice": "auto", "top_k": 20, "max_tokens": 256, "chat_template_kwargs": {}});
        let body = chat_request(&client, &request);
        assert_eq!(body["model"], "openai/gpt-oss-20b");
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["reasoning_effort"], "low");
        assert!(body.get("top_k").is_none() && body.get("chat_template_kwargs").is_none());
        let body = chat_request(&client, &json!({"messages": [], "tool_choice": "none"}));
        assert!(body.get("tool_choice").is_none());
    }

    #[test]
    fn reads_the_error_message() {
        assert_eq!(error_message(br#"{"error": {"message": "model not found", "type": "invalid_request_error"}}"#), "model not found");
        assert_eq!(error_message(b"Bad Gateway"), "Bad Gateway");
    }
}
