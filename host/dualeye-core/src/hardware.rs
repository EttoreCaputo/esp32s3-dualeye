//! What this computer can run: its processor, memory and GPU, and the voice
//! models to recommend for it ([`recommend`]).
//!
//! Whisper and the language model are what take time. On Apple silicon both
//! run on the GPU (Metal), sharing the computer's memory; on an NVIDIA card
//! they run in its own memory (with llama.cpp and whisper.cpp built for
//! CUDA); anywhere else on the CPU, where a 4B model takes several seconds to
//! answer. Whether the GPU is of use is llama-server's to say: the one the
//! app ships for Windows and Linux runs on the CPU, so an NVIDIA card needs
//! a CUDA build (`DUALEYE_LLAMA_SERVER`). The figures behind the thresholds
//! are in the README's voice section.

use serde::Serialize;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

use crate::models;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Gpu {
    /// Apple silicon: Metal, with the computer's memory.
    Apple,
    /// The first NVIDIA card NVML finds.
    Nvidia { name: String, memory_mb: u64 },
    /// None the models can use (or none found): they run on the CPU.
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hardware {
    /// The processor's name, e.g. "Apple M1 Pro".
    pub cpu: String,
    /// Physical cores.
    pub cores: usize,
    pub memory_mb: u64,
    pub gpu: Gpu,
    /// What llama-server can run models on besides the processor
    /// ([`crate::llm::devices`]); `None` without one to ask.
    pub devices: Option<Vec<String>>,
}

/// How quick the answers will be with the recommended models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Speed {
    /// About 1.5 s from the end of speech to the answer, 2–3 s with a tool call.
    Fast,
    /// A few seconds.
    Slow,
    /// Too little memory for a language model: only the fixed phrases.
    Limited,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Recommendation {
    /// A Whisper model id from [`models::MODELS`].
    pub whisper: &'static str,
    /// A language model id; `None`: better without one (the fixed phrases).
    pub llm: Option<&'static str>,
    pub speed: Speed,
    /// One sentence for the person, about this computer.
    pub why: String,
}

const GB: u64 = 1024;

impl Hardware {
    /// Read it now (NVML is asked once; a machine without it has no NVIDIA
    /// card), and ask llama-server for its devices ([`Self::with_devices`]).
    pub fn detect() -> Self {
        Self::detect_quick().with_devices()
    }

    /// Without asking llama-server: at once.
    pub fn detect_quick() -> Self {
        let sys = System::new_with_specifics(RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing()).with_memory(MemoryRefreshKind::nothing().with_ram()));
        let cpu = sys.cpus().first().map(|c| c.brand().trim().to_string()).filter(|b| !b.is_empty()).unwrap_or_else(|| std::env::consts::ARCH.to_string());
        let cores = System::physical_core_count().unwrap_or_else(|| sys.cpus().len()).max(1);
        Self { cpu, cores, memory_mb: sys.total_memory() / (1024 * 1024), gpu: detect_gpu(), devices: None }
    }

    /// Ask llama-server what it can run on: seconds the first time on a Mac
    /// (Metal compiles its shaders), a fraction of one after.
    pub fn with_devices(self) -> Self {
        Self { devices: crate::llm::find_server().and_then(|s| crate::llm::devices(&s)), ..self }
    }

    /// "Apple M1 Pro, 10 cores, 32 GB, GPU: Apple silicon"
    pub fn summary(&self) -> String {
        let gpu = match &self.gpu {
            Gpu::Apple => "GPU: Apple silicon (shared memory)".to_string(),
            Gpu::Nvidia { name, memory_mb } => format!("GPU: {name}, {} GB", gb(*memory_mb)),
            Gpu::None => "no GPU for the models".to_string(),
        };
        format!("{}, {} cores, {} GB, {gpu}", self.cpu, self.cores, gb(self.memory_mb))
    }
}

fn gb(mb: u64) -> u64 {
    (mb + GB / 2) / GB
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn detect_gpu() -> Gpu {
    Gpu::Apple
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn detect_gpu() -> Gpu {
    let nvidia = || {
        let nvml = nvml_wrapper::Nvml::init().ok()?;
        let dev = nvml.device_by_index(0).ok()?;
        Some(Gpu::Nvidia { name: dev.name().unwrap_or_else(|_| "NVIDIA".into()), memory_mb: dev.memory_info().ok()?.total / (1024 * 1024) })
    };
    nvidia().unwrap_or(Gpu::None)
}

#[cfg(not(any(all(target_os = "macos", target_arch = "aarch64"), target_os = "linux", target_os = "windows")))]
fn detect_gpu() -> Gpu {
    Gpu::None
}

/// The models to use on `hw`.
pub fn recommend(hw: &Hardware) -> Recommendation {
    let (best, light) = (models::DEFAULT_LLM, "qwen3.5-2b");
    let ram = hw.memory_mb;
    // A GPU llama-server doesn't list is no use to it.
    let gpu = if hw.devices.as_ref().is_some_and(Vec::is_empty) { &Gpu::None } else { &hw.gpu };
    let unused = match &hw.gpu {
        Gpu::Nvidia { name, .. } if gpu == &Gpu::None => format!("This llama-server can't use the {name} (it wasn't built for CUDA). "),
        _ => String::new(),
    };
    let (whisper, llm, speed, why) = match gpu {
        Gpu::Apple if ram >= 15 * GB => ("small", Some(best), Speed::Fast, "Apple silicon with enough memory runs the recommended models on its GPU.".to_string()),
        Gpu::Apple if ram >= 7 * GB => (
            "small",
            Some(light),
            Speed::Fast,
            format!("{} GB are shared with the GPU: the smaller language model leaves room for the rest.", gb(ram)),
        ),
        // The 4B model with its context (about 3.7 GB) and Whisper small (about 0.8 GB).
        Gpu::Nvidia { memory_mb, .. } if *memory_mb >= 5 * GB && ram >= 7 * GB => ("small", Some(best), Speed::Fast, "The NVIDIA card has room for both models.".to_string()),
        Gpu::Nvidia { memory_mb, .. } if *memory_mb >= 3 * GB && ram >= 7 * GB => {
            ("small", Some(light), Speed::Fast, format!("The NVIDIA card's {} GB fit the smaller language model.", gb(*memory_mb)))
        }
        _ if ram >= 7 * GB && hw.cores >= 4 => (
            "base",
            Some(light),
            Speed::Slow,
            format!("{unused}Without a GPU the models run on the processor: the smaller ones, and answers take a few seconds."),
        ),
        _ => (
            "base",
            None,
            Speed::Limited,
            format!("With {} GB and {} cores a language model would be too slow: the fixed phrases still work.", gb(ram), hw.cores),
        ),
    };
    Recommendation { whisper, llm, speed, why }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(gpu: Gpu, ram_gb: u64, cores: usize) -> Hardware {
        Hardware { cpu: "test".into(), cores, memory_mb: ram_gb * GB, gpu, devices: None }
    }

    #[test]
    fn recommendations() {
        let r = recommend(&hw(Gpu::Apple, 32, 10));
        assert_eq!((r.whisper, r.llm, r.speed), ("small", Some(models::DEFAULT_LLM), Speed::Fast));
        assert_eq!(recommend(&hw(Gpu::Apple, 8, 8)).llm, Some("qwen3.5-2b"));
        let nvidia = |gb| Gpu::Nvidia { name: "RTX".into(), memory_mb: gb * GB };
        assert_eq!(recommend(&hw(nvidia(8), 16, 8)).llm, Some(models::DEFAULT_LLM));
        assert_eq!(recommend(&hw(nvidia(4), 16, 8)).llm, Some("qwen3.5-2b"));
        let cpu = recommend(&hw(Gpu::None, 16, 8));
        assert_eq!((cpu.whisper, cpu.llm, cpu.speed), ("base", Some("qwen3.5-2b"), Speed::Slow));
        assert_eq!(recommend(&hw(Gpu::None, 4, 2)).llm, None);
        // A llama-server without CUDA.
        let cpu_only = Hardware { devices: Some(vec![]), ..hw(nvidia(8), 16, 8) };
        let r = recommend(&cpu_only);
        assert_eq!((r.llm, r.speed), (Some("qwen3.5-2b"), Speed::Slow));
        assert!(r.why.contains("CUDA"));
    }

    #[test]
    fn recommended_models_exist() {
        for gpu in [Gpu::Apple, Gpu::None, Gpu::Nvidia { name: String::new(), memory_mb: 4 * GB }] {
            for ram in [4, 8, 32] {
                let r = recommend(&hw(gpu.clone(), ram, 8));
                assert!(models::Model::by_id(r.whisper).is_some_and(|m| m.kind == models::Kind::Whisper));
                assert!(r.llm.is_none_or(|id| models::Model::by_id(id).is_some_and(|m| m.kind == models::Kind::Llm)));
            }
        }
    }

    #[test]
    fn detects_something() {
        let hw = Hardware::detect();
        assert!(hw.memory_mb > 0 && hw.cores > 0);
        assert!(!hw.summary().is_empty());
    }
}
