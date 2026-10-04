//! [`Api::ElevenLabs`](super::Api::ElevenLabs): [ElevenLabs](https://elevenlabs.io)'
//! own API, for voices only.
//!
//! A voice of it is one the person's account has, by its id
//! (`elevenlabs:voice/<voice_id>`): one they designed or added on
//! ElevenLabs' site, or one of ElevenLabs' own ([`voices`] lists them). It
//! speaks with Flash v2.5, the fastest model and half a credit a character,
//! as 16-bit PCM at 16 kHz, the board's rate. The key needs the Text to
//! Speech permission, and Voices with read access.

use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{Client, CloudError, ELEVENLABS};

const TIMEOUT: Duration = Duration::from_secs(30);
/// How long a voice is left alone once the month's credits are used up:
/// Piper speaks meanwhile.
const QUOTA_WAIT: Duration = Duration::from_secs(3600);
/// What every voice speaks with: any voice works with any model.
const SPEECH_MODEL: &str = "eleven_flash_v2_5";

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    // Error statuses are read like any answer, for ElevenLabs' message.
    AGENT.get_or_init(|| ureq::Agent::new_with_config(ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(TIMEOUT)).build()))
}

/// `path` (with its query) on ElevenLabs, with `key`: the body of a
/// successful answer. With `body`, a POST of it.
fn send(key: &str, path: &str, body: Option<&Value>) -> Result<Vec<u8>, CloudError> {
    let provider = ELEVENLABS.name;
    let network = |e: ureq::Error| CloudError::Network { provider, why: e.to_string() };
    let url = format!("{}{path}", ELEVENLABS.base_url);
    let sent = match body {
        None => agent().get(&url).header("xi-api-key", key).call(),
        Some(body) => agent().post(&url).header("xi-api-key", key).header("Content-Type", "application/json").send(body.to_string().as_bytes()),
    };
    let mut response = sent.map_err(network)?;
    let status = response.status().as_u16();
    let retry_after = response.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<f64>().ok()).map(Duration::from_secs_f64);
    let body = response.body_mut().with_config().limit(32 * 1024 * 1024).read_to_vec().map_err(network)?;
    if (200..300).contains(&status) {
        return Ok(body);
    }
    let (code, message) = error_detail(&body);
    Err(match (status, code.as_deref()) {
        (_, Some("quota_exceeded")) => CloudError::RateLimited { provider, retry_after: Some(QUOTA_WAIT) },
        (429, _) => CloudError::RateLimited { provider, retry_after },
        (401 | 403, None | Some("invalid_api_key")) => CloudError::BadKey { provider },
        _ => CloudError::Status { provider, status, message },
    })
}

/// ElevenLabs' code and message from an error's body: `{"detail": {"status",
/// "message"}}`, a validation error's list, or the start of whatever it is.
fn error_detail(body: &[u8]) -> (Option<String>, String) {
    let v: Value = serde_json::from_slice(body).unwrap_or_default();
    let detail = &v["detail"];
    let code = detail["status"].as_str().or(detail["code"].as_str()).map(str::to_string);
    let message = detail["message"]
        .as_str()
        .or(detail.as_str())
        .or(detail[0]["msg"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| String::from_utf8_lossy(body).chars().take(300).collect());
    (code, message)
}

/// A voice of the person's account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountVoice {
    pub voice_id: String,
    pub name: String,
    /// `generated` (designed), `cloned`, `professional`, `premade`
    /// (ElevenLabs' own)...
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// The voices the account can speak with: its own first, newest first, then
/// ElevenLabs'.
pub fn voices() -> Result<Vec<AccountVoice>, CloudError> {
    let key = ELEVENLABS.api_key().ok_or(CloudError::NoKey { provider: ELEVENLABS.name, env: ELEVENLABS.key_env })?;
    account_voices(&key)
}

fn account_voices(key: &str) -> Result<Vec<AccountVoice>, CloudError> {
    #[derive(Deserialize)]
    struct Page {
        voices: Vec<AccountVoice>,
        #[serde(default)]
        has_more: bool,
        #[serde(default)]
        next_page_token: Option<String>,
    }
    let mut out = Vec::new();
    let mut token: Option<String> = None;
    // An account has a few dozen at most; a bound all the same.
    for _ in 0..10 {
        let mut path = "/v2/voices?page_size=100&sort=created_at_unix&sort_direction=desc".to_string();
        if let Some(t) = &token {
            path.push_str(&format!("&next_page_token={}", urlencoding(t)));
        }
        let page: Page = serde_json::from_slice(&send(key, &path, None)?).map_err(|e| CloudError::Invalid { provider: ELEVENLABS.name, why: e.to_string() })?;
        out.extend(page.voices);
        match page.next_page_token.filter(|_| page.has_more) {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    // Stable: newest first within each.
    out.sort_by_key(|v| v.category == "premade");
    Ok(out)
}

/// `s` safe in a URL's path or query.
fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub(super) fn check(client: &Client) -> Result<(), CloudError> {
    check_key(&client.key)
}

/// Whether `key` works: lists a voice (free).
pub(super) fn check_key(key: &str) -> Result<(), CloudError> {
    send(key, "/v2/voices?page_size=1", None).map(drop)
}

/// `text` spoken in `language` (`it`, `en`): a 16 kHz WAV.
pub(super) fn speak(client: &Client, text: &str, language: Option<&str>) -> Result<Vec<u8>, CloudError> {
    let voice = client.voice_id.as_deref().ok_or_else(|| CloudError::Invalid { provider: ELEVENLABS.name, why: "no voice chosen".into() })?;
    let mut body = json!({"text": text, "model_id": SPEECH_MODEL});
    if let Some(l) = language {
        body["language_code"] = json!(l);
    }
    let pcm = send(&client.key, &format!("/v1/text-to-speech/{}?output_format=pcm_16000", urlencoding(voice)), Some(&body))?;
    let samples: Vec<i16> = pcm.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
    Ok(crate::voice::wav(&samples, 16_000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_errors() {
        let quota = br#"{"detail": {"status": "quota_exceeded", "message": "This request exceeds your quota."}}"#;
        assert_eq!(error_detail(quota), (Some("quota_exceeded".into()), "This request exceeds your quota.".into()));
        let invalid = br#"{"detail": [{"loc": ["body", "text"], "msg": "Field required"}]}"#;
        assert_eq!(error_detail(invalid), (None, "Field required".into()));
        assert_eq!(error_detail(b"Bad Gateway"), (None, "Bad Gateway".into()));
    }

    #[test]
    fn encodes_query_values() {
        assert_eq!(urlencoding("ab/c d+e"), "ab%2Fc%20d%2Be");
    }
}
