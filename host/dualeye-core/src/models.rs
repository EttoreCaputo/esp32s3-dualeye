//! The Whisper models the host can transcribe with: a short catalogue of
//! multilingual ggml models from `ggerganov/whisper.cpp` on Hugging Face,
//! pinned by size and SHA-256, kept in [`crate::stt::models_dir`].
//!
//! A download goes to `<file>.part` and is renamed once its checksum
//! matches, so a model file that exists is a complete one.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::Serialize;

use crate::stt::models_dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Model {
    /// What `--stt` and the app's settings call it.
    pub id: &'static str,
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    /// One line for the person picking it.
    pub note: &'static str,
}

const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";

/// The default is [`DEFAULT_MODEL`].
pub const MODELS: &[Model] = &[
    Model {
        id: "base",
        file: "ggml-base.bin",
        bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        note: "Fastest, for slow CPUs; often wrong in Italian",
    },
    Model {
        id: "small",
        file: "ggml-small.bin",
        bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        note: "Good balance: about 0.7 s a command on an M1 Pro",
    },
    Model {
        id: "large-v3-turbo-q5_0",
        file: "ggml-large-v3-turbo-q5_0.bin",
        bytes: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        note: "Most accurate; wants a GPU (Apple silicon, NVIDIA)",
    },
];

pub const DEFAULT_MODEL: &str = "small";

impl Model {
    pub fn by_id(id: &str) -> Option<&'static Model> {
        MODELS.iter().find(|m| m.id == id)
    }

    pub fn url(&self) -> String {
        format!("{BASE_URL}{}", self.file)
    }

    /// Where it is (or would be) kept.
    pub fn path(&self) -> Option<PathBuf> {
        models_dir().map(|d| d.join(self.file))
    }

    /// Downloaded and the right size (the checksum was checked on download).
    pub fn is_installed(&self) -> bool {
        self.path().and_then(|p| fs::metadata(p).ok()).is_some_and(|m| m.len() == self.bytes)
    }

    pub fn remove(&self) -> io::Result<()> {
        match self.path().map(fs::remove_file) {
            Some(Err(e)) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
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

    impl Model {
        /// Download and verify it, reporting whole percents. Returns its path.
        pub fn download(&self, cancel: &AtomicBool, progress: impl FnMut(Option<f32>)) -> io::Result<PathBuf> {
            let path = self.path().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder for the models"))?;
            if self.is_installed() {
                return Ok(path);
            }
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            let part = path.with_extension("bin.part");
            download_cancellable(&self.url(), &part, self.sha256, cancel, progress)?;
            fs::rename(&part, &path)?;
            Ok(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_is_consistent() {
        assert!(Model::by_id(DEFAULT_MODEL).is_some());
        for m in MODELS {
            assert_eq!(m.sha256.len(), 64);
            assert!(m.file.starts_with("ggml-") && m.file.ends_with(".bin"));
            assert!(m.url().ends_with(m.file));
        }
    }
}
