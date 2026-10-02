//! Windows and macOS: temperatures from sysinfo's component list.
//!
//! On Windows that is the ACPI thermal zone over WMI (needs an elevated
//! process, and many boards report a fixed or missing value). On macOS it is
//! the SMC on Intel and the IOHID sensor hub on Apple Silicon; `macos.rs`
//! adds the SMC's own CPU keys, fans, clocks and power on top. Windows exposes
//! no fan RPM or CPU power without a vendor driver.

use sysinfo::Components;

use super::{PlatformSample, Reading, average};
use crate::snapshot::round1;

/// Label fragments (lower-case) that mark a CPU sensor: Intel/AMD naming,
/// Apple Silicon P/E clusters (`pACC`, `eACC`) and SoC die sensors.
const CPU_HINTS: &[&str] = &["cpu", "core", "package", "tdie", "tctl", "pacc", "eacc", "soc"];

pub struct ComponentSensors {
    components: Components,
}

impl ComponentSensors {
    pub fn new() -> Self {
        Self { components: Components::new_with_refreshed_list() }
    }

    pub fn sample(&mut self) -> PlatformSample {
        self.components.refresh(false);
        let mut cpu = Vec::new();
        let mut gpu = Vec::new();
        let mut other = Vec::new();
        for c in self.components.list() {
            let Some(temp) = c.temperature().filter(|t| t.is_finite() && *t > 0.0) else { continue };
            let label = c.label().to_lowercase();
            if label.contains("gpu") {
                gpu.push(f64::from(temp));
            } else if CPU_HINTS.iter().any(|h| label.contains(h)) {
                cpu.push(f64::from(temp));
            } else if label.contains("thermal") || label.contains("acpi") {
                other.push(f64::from(temp));
            }
        }
        let mut out = PlatformSample { cpu_temp: average(&cpu).or_else(|| average(&other)), ..Default::default() };
        out.gpu.temp_c = average(&gpu).map(round1);
        out
    }

    pub fn readings(&mut self) -> Vec<Reading> {
        self.components.refresh(false);
        self.components
            .list()
            .iter()
            .filter_map(|c| {
                Some(Reading {
                    source: "components".into(),
                    label: c.label().to_string(),
                    value: f64::from(c.temperature()?),
                    unit: "°C",
                })
            })
            .collect()
    }
}
