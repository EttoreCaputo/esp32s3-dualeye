//! The models the host runs, downloaded on demand into
//! [`crate::stt::models_dir`]: multilingual Whisper models (ggml, from
//! `ggerganov/whisper.cpp` on Hugging Face) for speech-to-text, and Piper
//! voices (ONNX plus its JSON config, from `rhasspy/piper-voices`) for
//! speaking, and small language models (GGUF, quantized by Unsloth) for the
//! voice agent ([`crate::llm`]). Each file is pinned by size and SHA-256.
//!
//! A download goes to `<file>.part` and is renamed once its checksum
//! matches, so a model file that exists is a complete one.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::Serialize;

use crate::stt::models_dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Speech-to-text, for [`crate::Stt`].
    Whisper,
    /// Text-to-speech, for [`crate::Tts`].
    Voice,
    /// A language model for [`crate::Llm`]: what the voice agent thinks with.
    Llm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ModelFile {
    pub name: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Model {
    /// What `--stt`, `--tts-voice` and the app's settings call it.
    pub id: &'static str,
    pub kind: Kind,
    /// A voice's language: `it` or `en`.
    pub language: Option<&'static str>,
    /// The first is the model proper.
    pub files: &'static [ModelFile],
    /// One line for the person picking it.
    pub note: &'static str,
    /// Where its license is stated (the dataset's, for a voice).
    pub license: &'static str,
    /// The Hugging Face repository of a language model.
    pub repo: Option<&'static str>,
}

const WHISPER_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";
const PIPER_URL: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/main/";

const fn file(name: &'static str, bytes: u64, sha256: &'static str) -> ModelFile {
    ModelFile { name, bytes, sha256 }
}

/// The defaults are [`DEFAULT_MODEL`] and [`default_voice`].
pub const MODELS: &[Model] = &[
    Model {
        id: "base",
        kind: Kind::Whisper,
        language: None,
        files: &[file("ggml-base.bin", 147_951_465, "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe")],
        note: "Fastest, for slow CPUs; often wrong in Italian",
        license: "MIT",
        repo: None,
    },
    Model {
        id: "small",
        kind: Kind::Whisper,
        language: None,
        files: &[file("ggml-small.bin", 487_601_967, "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b")],
        note: "Good balance: about 0.7 s a command on an M1 Pro",
        license: "MIT",
        repo: None,
    },
    Model {
        id: "large-v3-turbo-q5_0",
        kind: Kind::Whisper,
        language: None,
        files: &[file("ggml-large-v3-turbo-q5_0.bin", 574_041_195, "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2")],
        note: "Most accurate; wants a GPU (Apple silicon, NVIDIA)",
        license: "MIT",
        repo: None,
    },
    Model {
        id: "it_IT-paola-medium",
        kind: Kind::Voice,
        language: Some("it"),
        files: &[
            file("it_IT-paola-medium.onnx", 63_511_038, "6fc918b5a0ea6137382833dddfa567bffbe6a5060c02043c87192ee59c04210c"),
            file("it_IT-paola-medium.onnx.json", 7_099, "aea19c0a7fce29fbc359b93f10e7902854401e4c95ae2ea328ae516b15d296cf"),
        ],
        note: "Italian, woman's voice, natural",
        license: "Dataset CC0 1.0 (paolapersico1/Voice-Dataset-Italian); fine-tuned from lessac",
        repo: None,
    },
    Model {
        id: "it_IT-riccardo-x_low",
        kind: Kind::Voice,
        language: Some("it"),
        files: &[
            file("it_IT-riccardo-x_low.onnx", 28_130_791, "1368de15f123275a7ef951c9e5e30be0f58a032daa14a0da44037443c1d1d21b"),
            file("it_IT-riccardo-x_low.onnx.json", 4_161, "146ab9c634afe524e9fb7530f2510df7a42fb1db56b52658ca1fb3d98001a62a"),
        ],
        note: "Italian, man's voice, smaller and flatter",
        license: "Dataset M-AILABS (BSD-style); trained from scratch",
        repo: None,
    },
    Model {
        id: "en_GB-alba-medium",
        kind: Kind::Voice,
        language: Some("en"),
        files: &[
            file("en_GB-alba-medium.onnx", 63_201_294, "401369c4a81d09fdd86c32c5c864440811dbdcc66466cde2d64f7133a66ad03b"),
            file("en_GB-alba-medium.onnx.json", 4_888, "aa965a2f02ecced632c2694e1fc72bbff6d65f265fab567ca945918c73dd89f4"),
        ],
        note: "British English, woman's voice",
        license: "Dataset CC BY 4.0 (Edinburgh DataShare 10283/3270); fine-tuned from lessac",
        repo: None,
    },
    Model {
        id: "en_US-ljspeech-medium",
        kind: Kind::Voice,
        language: Some("en"),
        files: &[
            file("en_US-ljspeech-medium.onnx", 63_531_379, "6f52a751e2349abe7a76735eb09dc1875298c77ea2342ffd2fef79ff81b87f22"),
            file("en_US-ljspeech-medium.onnx.json", 4_972, "141d612cc0a95ed7efc1ca936b845c2364967f2e9217c5dbfcf69fc4d6c65860"),
        ],
        note: "American English, woman's voice",
        license: "Dataset public domain (LJ Speech)",
        repo: None,
    },
    Model {
        id: "qwen3.5-2b",
        kind: Kind::Llm,
        language: None,
        files: &[file("Qwen3.5-2B-Q4_K_M.gguf", 1_280_835_840, "aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223")],
        note: "Lighter and faster (0.65 s); more mistakes, often answers in English",
        license: "Apache-2.0",
        repo: Some("unsloth/Qwen3.5-2B-GGUF"),
    },
    Model {
        id: "qwen3-4b-2507",
        kind: Kind::Llm,
        language: None,
        files: &[file("Qwen3-4B-Instruct-2507-Q4_K_M.gguf", 2_497_281_120, "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597")],
        note: "Recommended: all 53 test commands right, about 0.9 s each on an M1 Pro",
        license: "Apache-2.0",
        repo: Some("unsloth/Qwen3-4B-Instruct-2507-GGUF"),
    },
    Model {
        id: "qwen3.5-4b",
        kind: Kind::Llm,
        language: None,
        files: &[file("Qwen3.5-4B-Q4_K_M.gguf", 2_740_937_888, "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4")],
        note: "About as accurate, slower (1.4 s); sometimes answers in the wrong language",
        license: "Apache-2.0",
        repo: Some("unsloth/Qwen3.5-4B-GGUF"),
    },
];

pub const DEFAULT_MODEL: &str = "small";
/// The language model the voice agent uses unless another is picked.
pub const DEFAULT_LLM: &str = "qwen3-4b-2507";

/// The voice a language speaks with unless another is picked.
pub fn default_voice(language: &str) -> Option<&'static str> {
    match language {
        "it" => Some("it_IT-paola-medium"),
        "en" => Some("en_GB-alba-medium"),
        _ => None,
    }
}

impl Model {
    pub fn by_id(id: &str) -> Option<&'static Model> {
        MODELS.iter().find(|m| m.id == id)
    }

    pub fn of_kind(kind: Kind) -> impl Iterator<Item = &'static Model> {
        MODELS.iter().filter(move |m| m.kind == kind)
    }

    /// All its files.
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }

    pub fn url(&self, file: &ModelFile) -> String {
        let base = match self.kind {
            Kind::Whisper => WHISPER_URL.to_string(),
            // it_IT-paola-medium → it/it_IT/paola/medium/
            Kind::Voice => {
                let mut parts = self.id.splitn(3, '-');
                let (locale, name, quality) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                let family = locale.split('_').next().unwrap_or("");
                format!("{PIPER_URL}{family}/{locale}/{name}/{quality}/")
            }
            Kind::Llm => format!("https://huggingface.co/{}/resolve/main/", self.repo.unwrap_or_default()),
        };
        format!("{base}{}", file.name)
    }

    /// Where the model proper is (or would be) kept.
    pub fn path(&self) -> Option<PathBuf> {
        models_dir().map(|d| d.join(self.files[0].name))
    }

    /// Downloaded and the right size (the checksum was checked on download).
    pub fn is_installed(&self) -> bool {
        let Some(dir) = models_dir() else { return false };
        self.files.iter().all(|f| fs::metadata(dir.join(f.name)).is_ok_and(|m| m.len() == f.bytes))
    }

    pub fn remove(&self) -> io::Result<()> {
        let Some(dir) = models_dir() else { return Ok(()) };
        for f in self.files {
            match fs::remove_file(dir.join(f.name)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(feature = "download")]
mod fetch {
    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    use super::Model;
    use crate::download::download_cancellable;
    use crate::stt::models_dir;

    impl Model {
        /// Download and verify it, reporting whole percents of all its
        /// files. Returns the model proper's path.
        pub fn download(&self, cancel: &AtomicBool, mut progress: impl FnMut(Option<f32>)) -> io::Result<PathBuf> {
            let dir = models_dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder for the models"))?;
            fs::create_dir_all(&dir)?;
            let total = self.bytes() as f64;
            let mut before = 0u64;
            for f in self.files {
                let path = dir.join(f.name);
                if !fs::metadata(&path).is_ok_and(|m| m.len() == f.bytes) {
                    let part = dir.join(format!("{}.part", f.name));
                    let share = f.bytes as f64 / total;
                    let done = before as f64 / total;
                    download_cancellable(&self.url(f), &part, f.sha256, cancel, |p| {
                        // A small file of unknown size counts as not started until it's done.
                        progress(Some(((done + share * p.unwrap_or(0.0) as f64 / 100.0) * 100.0).floor() as f32))
                    })?;
                    fs::rename(&part, &path)?;
                }
                before += f.bytes;
            }
            Ok(dir.join(self.files[0].name))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_is_consistent() {
        assert!(Model::by_id(DEFAULT_MODEL).is_some_and(|m| m.kind == Kind::Whisper));
        assert!(Model::by_id(DEFAULT_LLM).is_some_and(|m| m.kind == Kind::Llm));
        for lang in ["it", "en"] {
            let voice = Model::by_id(default_voice(lang).unwrap()).unwrap();
            assert_eq!((voice.kind, voice.language), (Kind::Voice, Some(lang)));
        }
        for m in MODELS {
            assert!(!m.files.is_empty());
            for f in m.files {
                assert_eq!(f.sha256.len(), 64);
                assert!(m.url(f).ends_with(f.name));
            }
            match m.kind {
                Kind::Whisper => assert!(m.files[0].name.starts_with("ggml-") && m.language.is_none()),
                Kind::Voice => assert_eq!(m.files.iter().map(|f| f.name.to_string()).collect::<Vec<_>>(), [format!("{}.onnx", m.id), format!("{}.onnx.json", m.id)]),
                Kind::Llm => assert!(m.files[0].name.ends_with(".gguf") && m.repo.is_some() && m.language.is_none()),
            }
        }
    }

    #[test]
    fn voice_urls_follow_the_repository_layout() {
        let m = Model::by_id("it_IT-riccardo-x_low").unwrap();
        assert_eq!(m.url(&m.files[1]), "https://huggingface.co/rhasspy/piper-voices/resolve/main/it/it_IT/riccardo/x_low/it_IT-riccardo-x_low.onnx.json");
    }
}
