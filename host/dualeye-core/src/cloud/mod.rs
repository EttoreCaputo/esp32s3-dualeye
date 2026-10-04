//! Online services that run the voice's models instead of this computer:
//! speech-to-text ([`crate::Stt`]), the language model ([`crate::Llm`]) and
//! voices ([`crate::Tts`]).
//!
//! A [`Provider`] is a service: where it is, how to talk to it ([`Api`]),
//! where its API key comes from and the models of it DualEye knows
//! ([`CloudModel`]). Wherever a local model id goes (`--stt`, `--llm`,
//! `--tts-voice`, the app's settings) a cloud one does too, as
//! `provider:model`: `groq:whisper-large-v3-turbo`, `groq:hannah`. Local ids
//! have no `:`.
//!
//! Adding a service that speaks OpenAI's API is one more [`Provider`] in
//! [`PROVIDERS`]; one that doesn't is another [`Api`], and a module like
//! [`openai`] that [`Client`] sends its requests to.
//!
//! API keys come from the provider's environment variable (`GROQ_API_KEY`)
//! or `keys.json` in DualEye's data folder ([`set_api_key`]), readable only
//! by its owner.
//!
//! A model that hits its provider's limits (429: too many requests a
//! minute, or the day's are used up) isn't asked again until it said to
//! retry, a minute when it didn't: its requests fail at once with
//! [`CloudError::RateLimited`], which [`crate::Stt`], [`crate::Llm`] and
//! [`crate::Tts`] answer with a local model when there's one.

mod openai;

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::models::Kind;

/// How a [`Provider`] is talked to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Api {
    /// OpenAI's: `/chat/completions`, `/audio/transcriptions`, `/audio/speech`.
    OpenAi,
}

/// An online service.
#[derive(Debug, PartialEq, Eq)]
pub struct Provider {
    /// The `provider` in `provider:model`, and its key in `keys.json`.
    pub id: &'static str,
    pub name: &'static str,
    pub api: Api,
    /// Requests go to `{base_url}/chat/completions` and so on.
    pub base_url: &'static str,
    /// Where the API key is read from first.
    pub key_env: &'static str,
    /// Where to get a key.
    pub keys_url: &'static str,
    /// One line about it for the person picking it.
    pub note: &'static str,
    pub models: &'static [CloudModel],
}

/// A model of a [`Provider`].
#[derive(Debug, PartialEq, Eq)]
pub struct CloudModel {
    /// The `model` in `provider:model`.
    pub id: &'static str,
    pub kind: Kind,
    /// The provider's name for it.
    pub model: &'static str,
    /// A voice's name, for [`Kind::Voice`].
    pub voice: Option<&'static str>,
    /// A voice's language: `it` or `en`.
    pub language: Option<&'static str>,
    pub note: &'static str,
    /// JSON merged into every chat request for it (thinking turned down, a
    /// higher token limit for a model that thinks anyway).
    pub params: &'static str,
    /// The longest text a voice says in one request, in characters (0: no limit).
    pub max_chars: usize,
}

const fn chat(id: &'static str, note: &'static str, params: &'static str) -> CloudModel {
    CloudModel { id, kind: Kind::Llm, model: id, voice: None, language: None, note, params, max_chars: 0 }
}

const fn whisper(id: &'static str, note: &'static str) -> CloudModel {
    CloudModel { id, kind: Kind::Whisper, model: id, voice: None, language: None, note, params: "", max_chars: 0 }
}

const ORPHEUS_EN: &str = "canopylabs/orpheus-v1-english";

const fn orpheus(voice: &'static str, note: &'static str) -> CloudModel {
    CloudModel { id: voice, kind: Kind::Voice, model: ORPHEUS_EN, voice: Some(voice), language: Some("en"), note, params: "", max_chars: 200 }
}

/// [GroqCloud](https://console.groq.com): free with daily limits. The
/// limits that matter for a voice command (free plan, October 2026): 8K
/// tokens a minute for the GPT-OSS and Qwen models, about five commands a
/// minute with the board's tools; 100 sentences a day spoken by Orpheus,
/// 2,000 transcriptions a day by Whisper.
pub const GROQ: Provider = Provider {
    id: "groq",
    name: "Groq",
    api: Api::OpenAi,
    base_url: "https://api.groq.com/openai/v1",
    key_env: "GROQ_API_KEY",
    keys_url: "https://console.groq.com/keys",
    note: "Free with daily limits; what you say is sent to Groq",
    models: &[
        whisper("whisper-large-v3-turbo", "Whisper large v3 turbo on Groq: fast and accurate, 2,000 a day free"),
        whisper("whisper-large-v3", "Whisper large v3 on Groq: a little more accurate, slower"),
        chat(
            "openai/gpt-oss-20b",
            "GPT-OSS 20B on Groq: fast, good at tools; about 5 commands a minute free",
            r#"{"reasoning_effort": "low", "include_reasoning": false, "max_tokens": 1024}"#,
        ),
        chat(
            "openai/gpt-oss-120b",
            "GPT-OSS 120B on Groq: smarter, a little slower; same free limits",
            r#"{"reasoning_effort": "low", "include_reasoning": false, "max_tokens": 1024}"#,
        ),
        chat("qwen/qwen3.8-27b", "Qwen3.8 27B on Groq (preview): good Italian", r#"{"reasoning_effort": "none"}"#),
        chat("llama-3.3-70b-versatile", "Llama 3.3 70B on Groq: good Italian, no thinking", ""),
        orpheus("hannah", "Orpheus on Groq, English, woman's voice; 100 sentences a day free"),
        orpheus("diana", "Orpheus on Groq, English, woman's voice"),
        orpheus("autumn", "Orpheus on Groq, English, woman's voice"),
        orpheus("troy", "Orpheus on Groq, English, man's voice"),
        orpheus("austin", "Orpheus on Groq, English, man's voice"),
        orpheus("daniel", "Orpheus on Groq, English, man's voice"),
    ],
};

/// Every service DualEye can use.
pub const PROVIDERS: &[Provider] = &[GROQ];

impl Provider {
    pub fn by_id(id: &str) -> Option<&'static Provider> {
        PROVIDERS.iter().find(|p| p.id == id)
    }

    /// Its API key: from [`Provider::key_env`], else saved with [`set_api_key`].
    pub fn api_key(&self) -> Option<String> {
        self.key_source().map(|(key, _)| key)
    }

    /// Its API key and where it came from.
    pub fn key_source(&self) -> Option<(String, KeySource)> {
        if let Some(key) = std::env::var(self.key_env).ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
            return Some((key, KeySource::Env));
        }
        saved_keys().remove(self.id).map(|k| (k, KeySource::Saved))
    }
}

/// Where a [`Provider`]'s key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    /// Its environment variable, which wins over a saved one.
    Env,
    /// `keys.json`.
    Saved,
}

/// A cloud model and its provider: what a `provider:model` id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloudRef {
    pub provider: &'static Provider,
    pub model: &'static CloudModel,
}

impl CloudRef {
    /// `groq:hannah` → Groq's Hannah. `None` for a local id (no `:`) or one
    /// that isn't known.
    pub fn by_id(id: &str) -> Option<Self> {
        let (provider, model) = id.split_once(':')?;
        let provider = Provider::by_id(provider)?;
        let model = provider.models.iter().find(|m| m.id == model)?;
        Some(Self { provider, model })
    }

    /// Every cloud model of `kind`.
    pub fn of_kind(kind: Kind) -> impl Iterator<Item = CloudRef> {
        PROVIDERS.iter().flat_map(|provider| provider.models.iter().map(move |model| CloudRef { provider, model })).filter(move |r| r.model.kind == kind)
    }

    /// Its `provider:model` id.
    pub fn id(&self) -> String {
        format!("{}:{}", self.provider.id, self.model.id)
    }
}

/// Whether `id` names a cloud model (or tries to: it has a `:`).
pub fn is_cloud_id(id: &str) -> bool {
    id.contains(':')
}

fn keys_file() -> Option<PathBuf> {
    crate::claude::data_dir().map(|d| d.join("keys.json"))
}

fn saved_keys() -> BTreeMap<String, String> {
    keys_file().and_then(|f| fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Save `provider`'s API key in `keys.json` (`None` forgets it), readable
/// only by this user.
pub fn set_api_key(provider: &Provider, key: Option<&str>) -> io::Result<()> {
    let file = keys_file().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))?;
    let mut keys = saved_keys();
    match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => keys.insert(provider.id.to_string(), k.to_string()),
        None => keys.remove(provider.id),
    };
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(&keys).map_err(io::Error::other)?;
    let tmp = file.with_extension("json.tmp");
    write_private(&tmp, &json)?;
    fs::rename(tmp, file)
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    fs::write(path, bytes)
}

/// What went wrong talking to a provider. Its message starts with the
/// provider's name.
#[derive(Debug, Clone, PartialEq)]
pub enum CloudError {
    /// No key for the provider.
    NoKey { provider: &'static str, env: &'static str },
    /// The key was refused (401, 403).
    BadKey { provider: &'static str },
    /// Too many requests (429): how long to wait, when it said.
    RateLimited { provider: &'static str, retry_after: Option<Duration> },
    /// Any other error status, with the provider's message.
    Status { provider: &'static str, status: u16, message: String },
    /// It couldn't be reached.
    Network { provider: &'static str, why: String },
    /// It answered something unexpected.
    Invalid { provider: &'static str, why: String },
}

impl fmt::Display for CloudError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CloudError::NoKey { provider, env } => write!(f, "{provider}: no API key (set {env}, or add one in the app)"),
            CloudError::BadKey { provider } => write!(f, "{provider}: the API key was refused"),
            CloudError::RateLimited { provider, retry_after: Some(d) } => write!(f, "{provider}: rate limit reached, try again in {} s", d.as_secs().max(1)),
            CloudError::RateLimited { provider, retry_after: None } => write!(f, "{provider}: rate limit reached"),
            CloudError::Status { provider, status, message } => write!(f, "{provider}: {status} {message}"),
            CloudError::Network { provider, why } => write!(f, "{provider}: {why}"),
            CloudError::Invalid { provider, why } => write!(f, "{provider}: {why}"),
        }
    }
}

impl std::error::Error for CloudError {}

impl CloudError {
    /// The provider's limits were hit: a local model should answer instead.
    pub fn is_limit(&self) -> bool {
        matches!(self, CloudError::RateLimited { .. })
    }
}

/// How long a model that hit its limits without saying when to retry is left alone.
const COOLDOWN: Duration = Duration::from_secs(60);

/// The models that hit their limits, by `provider:model` id, and until when.
static LIMITED: Mutex<BTreeMap<String, Instant>> = Mutex::new(BTreeMap::new());

/// A transcription: the text, its language as the provider names it
/// (`italian`, `english`) and `segments` with `avg_logprob`, like
/// whisper-server's `verbose_json`.
pub type TranscriptionReply = Value;

/// One cloud model and the key to use it with. Its [`fmt::Debug`] leaves
/// the key out.
#[derive(Clone, PartialEq, Eq)]
pub struct Client {
    pub model: CloudRef,
    key: String,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client").field("model", &self.model.id()).finish_non_exhaustive()
    }
}

impl Client {
    /// The cloud model `id` (`groq:hannah`) of `kind`, with its provider's key.
    pub fn new(id: &str, kind: Kind) -> Result<Self, String> {
        let model = CloudRef::by_id(id).filter(|r| r.model.kind == kind).ok_or_else(|| format!("unknown cloud model {id}"))?;
        let key = model.provider.api_key().ok_or_else(|| CloudError::NoKey { provider: model.provider.name, env: model.provider.key_env }.to_string())?;
        Ok(Self { model, key })
    }

    /// Check the key and that the service answers (lists its models: free).
    pub fn check(&self) -> Result<(), CloudError> {
        match self.model.provider.api {
            Api::OpenAi => openai::check(self),
        }
    }

    /// A chat completion: OpenAI's request (`messages`, `tools`, sampling);
    /// the first choice's message. Settings only llama.cpp knows are left
    /// out, and the model's [`CloudModel::params`] added.
    pub fn chat(&self, request: &Value) -> Result<Value, CloudError> {
        self.limited(|| match self.model.provider.api {
            Api::OpenAi => openai::chat(self, request),
        })
    }

    /// Transcribe a WAV, in `language` (`it`, `en`) or whichever it hears (`None`).
    pub fn transcribe(&self, wav: &[u8], language: Option<&str>, prompt: &str) -> Result<TranscriptionReply, CloudError> {
        self.limited(|| match self.model.provider.api {
            Api::OpenAi => openai::transcribe(self, wav, language, prompt),
        })
    }

    /// Speak `text` with the voice: a WAV.
    pub fn speak(&self, text: &str) -> Result<Vec<u8>, CloudError> {
        self.limited(|| match self.model.provider.api {
            Api::OpenAi => openai::speak(self, text),
        })
    }

    /// Whether the model is left alone after hitting its limits.
    pub fn is_limited(&self) -> bool {
        self.limited_for().is_some()
    }

    fn limited_for(&self) -> Option<Duration> {
        let mut limited = LIMITED.lock().unwrap();
        let id = self.model.id();
        let left = limited.get(&id).and_then(|until| until.checked_duration_since(Instant::now()));
        if left.is_none() {
            limited.remove(&id);
        }
        left
    }

    /// `request`, unless the model hit its limits and it's not yet time to
    /// retry; a limit it hits now is remembered.
    fn limited<T>(&self, request: impl FnOnce() -> Result<T, CloudError>) -> Result<T, CloudError> {
        if let Some(left) = self.limited_for() {
            return Err(CloudError::RateLimited { provider: self.model.provider.name, retry_after: Some(left) });
        }
        let reply = request();
        if let Err(CloudError::RateLimited { retry_after, .. }) = &reply {
            LIMITED.lock().unwrap().insert(self.model.id(), Instant::now() + retry_after.unwrap_or(COOLDOWN));
        }
        reply
    }
}

/// `text` in pieces of at most `max` characters (0: one piece), split
/// between words: give it a sentence at a time.
pub fn chunks(text: &str, max: usize) -> Vec<String> {
    let text = text.trim();
    if max == 0 || text.chars().count() <= max {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let len = current.chars().count();
        if len > 0 && len + 1 + word.chars().count() > max {
            out.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        // A word longer than the limit on its own: cut it.
        let mut word: Vec<char> = word.chars().collect();
        while word.len() > max {
            out.push(word.drain(..max).collect());
        }
        current.extend(word);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        let r = CloudRef::by_id("groq:hannah").unwrap();
        assert_eq!((r.provider.id, r.model.kind, r.model.voice), ("groq", Kind::Voice, Some("hannah")));
        assert_eq!(r.id(), "groq:hannah");
        assert_eq!(CloudRef::by_id("groq:openai/gpt-oss-20b").map(|r| r.model.kind), Some(Kind::Llm));
        assert!(CloudRef::by_id("small").is_none());
        assert!(CloudRef::by_id("nobody:small").is_none());
    }

    #[test]
    fn catalog_is_consistent() {
        for p in PROVIDERS {
            assert!(!p.id.contains(':') && p.base_url.starts_with("https://"));
            for m in p.models {
                assert_eq!(p.models.iter().filter(|o| o.id == m.id).count(), 1, "{} twice", m.id);
                assert_eq!(m.kind == Kind::Voice, m.voice.is_some() && m.language.is_some(), "{}", m.id);
                assert!(m.params.is_empty() || serde_json::from_str::<serde_json::Map<String, Value>>(m.params).is_ok(), "{}", m.id);
            }
        }
        // Local ids never look like cloud ones.
        assert!(crate::models::MODELS.iter().all(|m| !is_cloud_id(m.id)));
    }

    #[test]
    fn a_limit_is_remembered() {
        let client = Client { model: CloudRef::by_id("groq:whisper-large-v3").unwrap(), key: "k".into() };
        let limit = || CloudError::RateLimited { provider: "Groq", retry_after: Some(Duration::from_secs(30)) };
        assert!(!client.is_limited());
        assert_eq!(client.limited(|| Err::<(), _>(limit())), Err(limit()));
        assert!(client.is_limited());
        // Not asked again while it's limited.
        let reply = client.limited(|| -> Result<(), CloudError> { panic!("asked") });
        assert!(reply.is_err_and(|e| e.is_limit()));
        LIMITED.lock().unwrap().clear();
        assert_eq!(client.limited(|| Ok(1)), Ok(1));
    }

    #[test]
    fn chunks_fit() {
        assert_eq!(chunks("Short.", 200), ["Short."]);
        let long = "word ".repeat(100);
        let parts = chunks(&long, 42);
        assert!(parts.iter().all(|p| p.chars().count() <= 42));
        assert_eq!(parts.join(" "), long.trim());
        assert_eq!(chunks(&"x".repeat(10), 4), ["xxxx", "xxxx", "xx"]);
    }
}
