//! Direct sensor access, one backend per platform:
//!
//! | Data            | Linux                       | Windows                  | macOS              |
//! |-----------------|-----------------------------|--------------------------|--------------------|
//! | CPU load, clock | sysinfo                     | sysinfo                  | sysinfo            |
//! | CPU temp        | hwmon (coretemp, k10temp…)  | ACPI thermal zone (WMI)  | SMC / IOHID        |
//! | CPU power       | RAPL (powercap)             | —                        | —                  |
//! | NVIDIA GPU      | NVML                        | NVML                     | —                  |
//! | AMD GPU         | hwmon (amdgpu)              | —                        | —                  |
//! | Apple/Mac GPU   | —                           | —                        | SMC + IOAccelerator|
//! | Fans            | hwmon                       | —                        | —                  |
//! | RAM             | sysinfo                     | sysinfo                  | sysinfo            |
//! | VRAM            | NVML, amdgpu `mem_info_*`   | NVML                     | IOAccelerator      |

#[cfg(not(target_os = "linux"))]
mod components;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod nvidia;

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

use crate::snapshot::{DeviceMetrics, Fan, Memory, SNAPSHOT_VERSION, Snapshot, round1};

/// One raw sensor value, for diagnostics (`dualeye --sensors`, an app's sensor page).
#[derive(Debug, Clone, Serialize)]
pub struct Reading {
    pub source: String,
    pub label: String,
    pub value: f64,
    pub unit: &'static str,
}

/// What a platform backend adds on top of the portable CPU load/clock.
#[derive(Debug, Default)]
struct PlatformSample {
    cpu_temp: Option<f64>,
    cpu_power: Option<f64>,
    gpu: DeviceMetrics,
    board_fans: Vec<u32>,
    gpu_fans: Vec<u32>,
}

pub struct Collector {
    sys: System,
    #[cfg(target_os = "linux")]
    platform: linux::LinuxSensors,
    #[cfg(not(target_os = "linux"))]
    platform: components::ComponentSensors,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    nvidia: Option<nvidia::Nvidia>,
    #[cfg(target_os = "macos")]
    mac_gpu: macos::MacGpu,
}

impl Collector {
    pub fn new() -> Self {
        let sys = System::new_with_specifics(RefreshKind::nothing().with_cpu(cpu_refresh()).with_memory(ram_refresh()));
        Self {
            sys,
            #[cfg(target_os = "linux")]
            platform: linux::LinuxSensors::new(),
            #[cfg(not(target_os = "linux"))]
            platform: components::ComponentSensors::new(),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            nvidia: nvidia::Nvidia::init(),
            #[cfg(target_os = "macos")]
            mac_gpu: macos::MacGpu::new(),
        }
    }

    /// Read every sensor once. CPU load and power are deltas since the previous
    /// call, so the first sample after [`Collector::new`] may lack them; call at
    /// a steady interval (≥ 200 ms).
    pub fn sample(&mut self) -> Snapshot {
        self.sys.refresh_cpu_specifics(cpu_refresh());
        self.sys.refresh_memory_specifics(ram_refresh());
        #[allow(unused_mut)]
        let mut platform = self.platform.sample();

        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if let Some((gpu, fans)) = self.nvidia.as_ref().and_then(|n| n.sample()) {
            platform.gpu = gpu;
            platform.gpu_fans = fans;
        }
        #[cfg(target_os = "macos")]
        self.mac_gpu.fill(&mut platform.gpu, self.sys.total_memory());

        let cpus = self.sys.cpus();
        let clock = if cpus.is_empty() {
            0
        } else {
            cpus.iter().map(|c| c.frequency()).sum::<u64>() / cpus.len() as u64
        };
        let cpu = DeviceMetrics {
            temp_c: platform.cpu_temp.map(round1),
            load_pct: Some(round1(f64::from(self.sys.global_cpu_usage()))),
            clock_mhz: (clock > 0).then_some(clock as u32),
            power_w: platform.cpu_power.map(round1),
            mem: self.ram(),
        };

        let mut fans = Vec::new();
        if let Some(&rpm) = platform.board_fans.iter().max() {
            fans.push(Fan { id: "cpu".into(), rpm });
        }
        if let Some(&rpm) = platform.gpu_fans.iter().max() {
            fans.push(Fan { id: "gpu".into(), rpm });
        }

        Snapshot {
            v: SNAPSHOT_VERSION,
            ts: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            cpu,
            gpu: platform.gpu,
            fans,
            face: None,
            rot: None,
            claude: None,
        }
    }

    fn ram(&self) -> Option<Memory> {
        Memory::from_bytes(self.sys.used_memory(), self.sys.total_memory())
    }

    /// Every raw temperature/fan/power value the backends can see.
    pub fn readings(&mut self) -> Vec<Reading> {
        #[allow(unused_mut)]
        let mut out = self.platform.readings();
        self.sys.refresh_memory_specifics(ram_refresh());
        if let Some(ram) = self.ram() {
            for (label, mb) in [("RAM used", ram.used_mb), ("RAM total", ram.total_mb)] {
                out.push(Reading { source: "memory".into(), label: label.into(), value: f64::from(mb), unit: "MB" });
            }
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if let Some(nvidia) = &self.nvidia {
            out.extend(nvidia.readings());
        }
        #[cfg(target_os = "macos")]
        out.extend(self.mac_gpu.readings());
        out
    }
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}

fn cpu_refresh() -> CpuRefreshKind {
    CpuRefreshKind::nothing().with_cpu_usage().with_frequency()
}

fn ram_refresh() -> MemoryRefreshKind {
    MemoryRefreshKind::nothing().with_ram()
}

pub(crate) fn average(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}
