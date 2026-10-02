//! Apple Silicon clocks and power from IOReport, the private library behind
//! `powermetrics` (no root needed; the same source Stats and macmon read).
//!
//! - `Energy Model`: energy counters per block (`CPU Energy`, `GPU Energy`, in
//!   mJ/µJ/nJ); the delta over the sampling interval is the power.
//! - `CPU Stats` / `CPU Core Performance States` and `GPU Stats` /
//!   `GPU Performance States` (`GPUPH`): time spent in each DVFS state. The
//!   states' frequencies come from the `pmgr` node in the IORegistry
//!   (`voltage-states*`), so the residency-weighted average is the clock the
//!   core ran at.

use std::ffi::{CStr, c_char, c_void};
use std::ptr::null;
use std::time::Instant;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFRelease, CFType, CFTypeRef, TCFType, kCFAllocatorDefault};
use core_foundation::data::CFData;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef, CFMutableDictionary, CFMutableDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use io_kit_sys::{
    IOIteratorNext, IOObjectRelease, IORegistryEntryCreateCFProperties, IORegistryEntryGetName,
    IOServiceGetMatchingServices, IOServiceMatching, kIOMasterPortDefault,
};
use mach2::kern_return::KERN_SUCCESS;

#[link(name = "IOReport", kind = "dylib")]
unsafe extern "C" {
    fn IOReportCopyChannelsInGroup(
        group: CFStringRef,
        subgroup: CFStringRef,
        a: u64,
        b: u64,
        c: u64,
    ) -> CFMutableDictionaryRef;
    fn IOReportMergeChannels(a: CFDictionaryRef, b: CFDictionaryRef, nil: CFTypeRef);
    fn IOReportCreateSubscription(
        a: *const c_void,
        channels: CFMutableDictionaryRef,
        subscribed: *mut CFMutableDictionaryRef,
        id: u64,
        nil: CFTypeRef,
    ) -> CFTypeRef;
    fn IOReportCreateSamples(sub: CFTypeRef, channels: CFMutableDictionaryRef, nil: CFTypeRef) -> CFDictionaryRef;
    fn IOReportCreateSamplesDelta(a: CFDictionaryRef, b: CFDictionaryRef, nil: CFTypeRef) -> CFDictionaryRef;
    fn IOReportChannelGetGroup(ch: CFDictionaryRef) -> CFStringRef;
    fn IOReportChannelGetChannelName(ch: CFDictionaryRef) -> CFStringRef;
    fn IOReportChannelGetUnitLabel(ch: CFDictionaryRef) -> CFStringRef;
    fn IOReportSimpleGetIntegerValue(ch: CFDictionaryRef, idx: i32) -> i64;
    fn IOReportStateGetCount(ch: CFDictionaryRef) -> i32;
    fn IOReportStateGetNameForIndex(ch: CFDictionaryRef, idx: i32) -> CFStringRef;
    fn IOReportStateGetResidency(ch: CFDictionaryRef, idx: i32) -> i64;
}

const CHANNELS: &[(&str, Option<&str>)] = &[
    ("Energy Model", None),
    ("CPU Stats", Some("CPU Core Performance States")),
    ("GPU Stats", Some("GPU Performance States")),
];

/// What one interval between two samples measured.
#[derive(Debug, Default, Clone, Copy)]
pub struct Sample {
    pub cpu_mhz: Option<u32>,
    pub gpu_mhz: Option<u32>,
    pub cpu_w: Option<f64>,
    pub gpu_w: Option<f64>,
}

/// DVFS state frequencies in MHz, lowest first.
#[derive(Debug, Default)]
struct Dvfs {
    /// Efficiency cores (`ECPU`; the middle tier, `MCPU`, on M5 Pro/Max).
    low: Vec<u32>,
    /// Performance cores (`PCPU`).
    high: Vec<u32>,
    gpu: Vec<u32>,
}

pub struct IoReport {
    sub: CFTypeRef,
    channels: CFMutableDictionaryRef,
    dvfs: Dvfs,
    prev: Option<(CFDictionaryRef, Instant)>,
}

// The subscription and the dictionaries are CF objects, usable from any thread
// as long as one thread at a time does (`Collector` is not shared).
unsafe impl Send for IoReport {}

impl IoReport {
    pub fn new() -> Option<Self> {
        let channels = channels()?;
        let mut subscribed: CFMutableDictionaryRef = std::ptr::null_mut();
        let sub = unsafe { IOReportCreateSubscription(null(), channels, &mut subscribed, 0, null()) };
        if !subscribed.is_null() {
            unsafe { CFRelease(subscribed.cast()) };
        }
        if sub.is_null() {
            unsafe { CFRelease(channels.cast()) };
            return None;
        }
        Some(Self { sub, channels, dvfs: dvfs(), prev: None })
    }

    /// Everything since the previous call; `None` on the first.
    pub fn sample(&mut self) -> Option<Sample> {
        let now = Instant::now();
        let next = unsafe { IOReportCreateSamples(self.sub, self.channels, null()) };
        if next.is_null() {
            return None;
        }
        let (prev, then) = self.prev.replace((next, now))?;
        let delta = unsafe { IOReportCreateSamplesDelta(prev, next, null()) };
        unsafe { CFRelease(prev.cast()) };
        if delta.is_null() {
            return None;
        }
        let delta = unsafe { CFDictionary::<CFType, CFType>::wrap_under_create_rule(delta) };
        let secs = now.duration_since(then).as_secs_f64();
        (secs > 0.0).then(|| self.parse(&delta, secs))
    }

    fn parse(&self, delta: &CFDictionary<CFType, CFType>, secs: f64) -> Sample {
        let mut out = Sample::default();
        let mut cpu_w = 0.0;
        let mut gpu_w = 0.0;
        let mut cores = Vec::new();
        for ch in items(delta) {
            let group = string(unsafe { IOReportChannelGetGroup(ch) });
            let name = string(unsafe { IOReportChannelGetChannelName(ch) });
            match group.as_str() {
                "Energy Model" => {
                    let Some(joules) = joules(ch) else { continue };
                    // `CPU Energy`, or `DIE_n_CPU Energy` on an Ultra.
                    if name.ends_with("CPU Energy") {
                        cpu_w += joules / secs;
                    } else if name == "GPU Energy" {
                        gpu_w += joules / secs;
                    }
                }
                "CPU Stats" => {
                    let freqs = if name.contains("PCPU") {
                        &self.dvfs.high
                    } else if name.contains("ECPU") || name.contains("MCPU") {
                        &self.dvfs.low
                    } else {
                        continue;
                    };
                    // An idle core is at its lowest state, not at 0 MHz.
                    if let Some(&min) = freqs.first() {
                        cores.push(average_mhz(&residencies(ch), freqs).unwrap_or(min).max(min));
                    }
                }
                // The table's first state is the GPU switched off.
                "GPU Stats" if name == "GPUPH" && self.dvfs.gpu.len() > 1 => {
                    out.gpu_mhz = average_mhz(&residencies(ch), &self.dvfs.gpu[1..]);
                }
                _ => {}
            }
        }
        if !cores.is_empty() {
            out.cpu_mhz = Some((cores.iter().map(|&f| u64::from(f)).sum::<u64>() / cores.len() as u64) as u32);
        }
        out.cpu_w = (cpu_w > 0.0).then_some(cpu_w);
        out.gpu_w = (gpu_w > 0.0).then_some(gpu_w);
        out
    }
}

impl Drop for IoReport {
    fn drop(&mut self) {
        unsafe {
            if let Some((prev, _)) = self.prev.take() {
                CFRelease(prev.cast());
            }
            CFRelease(self.channels.cast());
            CFRelease(self.sub);
        }
    }
}

/// The channels in [`CHANNELS`], merged into one mutable dictionary.
fn channels() -> Option<CFMutableDictionaryRef> {
    let mut merged: Option<CFMutableDictionaryRef> = None;
    for (group, subgroup) in CHANNELS {
        let group = CFString::new(group);
        let subgroup = subgroup.map(CFString::new);
        let found = unsafe {
            IOReportCopyChannelsInGroup(
                group.as_concrete_TypeRef(),
                subgroup.as_ref().map_or(null(), |s| s.as_concrete_TypeRef()),
                0,
                0,
                0,
            )
        };
        if found.is_null() {
            continue;
        }
        match merged {
            None => merged = Some(found),
            Some(into) => unsafe {
                IOReportMergeChannels(into, found, null());
                CFRelease(found.cast());
            },
        }
    }
    let merged = unsafe { CFDictionary::<CFType, CFType>::wrap_under_create_rule(merged?) };
    if items(&merged).is_empty() {
        return None;
    }
    let copy = CFMutableDictionary::from(&merged);
    let raw = copy.as_concrete_TypeRef();
    std::mem::forget(copy);
    Some(raw)
}

/// The `IOReportChannels` array of a channel list or a sample.
fn items(dict: &CFDictionary<CFType, CFType>) -> Vec<CFDictionaryRef> {
    let key = CFString::from_static_string("IOReportChannels");
    let Some(array) = dict.find(key.as_CFType()) else {
        return Vec::new();
    };
    let array = unsafe { CFArray::<CFType>::wrap_under_get_rule(array.as_CFTypeRef() as CFArrayRef) };
    array.get_all_values().into_iter().map(|v| v as CFDictionaryRef).collect()
}

/// A string the channel owns (get rule).
fn string(s: CFStringRef) -> String {
    if s.is_null() { String::new() } else { unsafe { CFString::wrap_under_get_rule(s) }.to_string() }
}

fn joules(ch: CFDictionaryRef) -> Option<f64> {
    let value = unsafe { IOReportSimpleGetIntegerValue(ch, 0) } as f64;
    let scale = match string(unsafe { IOReportChannelGetUnitLabel(ch) }).trim() {
        "mJ" => 1e-3,
        "uJ" => 1e-6,
        "nJ" => 1e-9,
        _ => return None,
    };
    Some(value * scale)
}

fn residencies(ch: CFDictionaryRef) -> Vec<(String, i64)> {
    let count = unsafe { IOReportStateGetCount(ch) };
    (0..count)
        .map(|i| unsafe { (string(IOReportStateGetNameForIndex(ch, i)), IOReportStateGetResidency(ch, i)) })
        .collect()
}

/// The clock averaged over the time spent in active states. They follow the
/// inactive ones (`IDLE`, `DOWN` on CPUs, `OFF` on the GPU) in the order of
/// `freqs`. `None` if the block never left them.
fn average_mhz(states: &[(String, i64)], freqs: &[u32]) -> Option<u32> {
    let offset = states.iter().position(|(n, _)| !matches!(n.as_str(), "IDLE" | "DOWN" | "OFF"))?;
    let active = &states[offset..];
    let busy: f64 = active.iter().take(freqs.len()).map(|(_, r)| *r as f64).sum();
    if busy <= 0.0 {
        return None;
    }
    let mhz: f64 = active.iter().zip(freqs).map(|((_, r), &f)| *r as f64 / busy * f64::from(f)).sum();
    Some(mhz.round() as u32)
}

/// The DVFS tables from `pmgr`. CPU tables are `voltage-states1-sram` (E) and
/// `voltage-states5-sram` (P) up to M4; M5 lists its clusters in `acc-clusters`.
/// The GPU's is `voltage-states9`.
fn dvfs() -> Dvfs {
    let mut out = Dvfs::default();
    let Some(pmgr) = pmgr_properties() else {
        return out;
    };
    let table = |key: &str| -> Vec<u32> {
        let key = CFString::new(key);
        pmgr.find(&key).and_then(|v| v.downcast::<CFData>()).map(|d| to_mhz(d.bytes())).unwrap_or_default()
    };
    out.low = table("voltage-states1-sram");
    out.high = table("voltage-states5-sram");
    if let Some((low, high)) = acc_clusters(&pmgr) {
        // M5: the two tiers in use are the two highest cluster types.
        out.low = table(&low);
        out.high = table(&high);
    }
    out.gpu = table("voltage-states9");
    out
}

fn pmgr_properties() -> Option<CFDictionary<CFString, CFType>> {
    unsafe {
        let mut iter = 0;
        if IOServiceGetMatchingServices(
            kIOMasterPortDefault,
            IOServiceMatching(c"AppleARMIODevice".as_ptr()),
            &mut iter,
        ) != KERN_SUCCESS
        {
            return None;
        }
        let mut found = None;
        loop {
            let entry = IOIteratorNext(iter);
            if entry == 0 {
                break;
            }
            let mut name: [c_char; 128] = [0; 128];
            let is_pmgr = IORegistryEntryGetName(entry, name.as_mut_ptr()) == KERN_SUCCESS
                && CStr::from_ptr(name.as_ptr()).to_bytes() == b"pmgr";
            if is_pmgr {
                let mut props: CFMutableDictionaryRef = std::ptr::null_mut();
                if IORegistryEntryCreateCFProperties(entry, &mut props, kCFAllocatorDefault, 0) == KERN_SUCCESS
                    && !props.is_null()
                {
                    found = Some(CFDictionary::wrap_under_create_rule(props as CFDictionaryRef));
                }
            }
            IOObjectRelease(entry);
            if found.is_some() {
                break;
            }
        }
        IOObjectRelease(iter);
        found
    }
}

/// `acc-clusters`: 8 bytes per cluster, byte 0 the `voltage-states` index and
/// byte 1 the tier. Only present (with three tiers) from M5 on.
fn acc_clusters(pmgr: &CFDictionary<CFString, CFType>) -> Option<(String, String)> {
    let key = CFString::from_static_string("acc-clusters");
    let data = pmgr.find(&key)?.downcast::<CFData>()?;
    let mut clusters: Vec<(u8, u8)> = data.bytes().chunks_exact(8).map(|c| (c[1], c[0])).collect();
    clusters.sort();
    clusters.dedup_by_key(|c| c.0);
    if clusters.len() < 3 {
        return None;
    }
    let [.., low, high] = clusters[..] else {
        return None;
    };
    Some((format!("voltage-states{}-sram", low.1), format!("voltage-states{}-sram", high.1)))
}

/// (frequency, voltage) pairs of little-endian u32. Up to M3 the frequency is
/// in Hz, from M4 in kHz; real clocks are 100 MHz – 5 GHz either way.
fn to_mhz(bytes: &[u8]) -> Vec<u32> {
    let raw: Vec<u32> = bytes.chunks_exact(8).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let scale = if raw.iter().any(|&f| f > 10_000_000) { 1_000_000 } else { 1_000 };
    raw.into_iter().map(|f| f / scale).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn states(s: &[(&str, i64)]) -> Vec<(String, i64)> {
        s.iter().map(|(n, r)| (n.to_string(), *r)).collect()
    }

    #[test]
    fn averages_over_active_residency() {
        let s = states(&[("DOWN", 0), ("IDLE", 500), ("V0P1", 100), ("V1P2", 400)]);
        assert_eq!(average_mhz(&s, &[1000, 2000]), Some(1800));
    }

    #[test]
    fn idle_block_has_no_clock() {
        let s = states(&[("OFF", 900), ("P1", 0), ("P2", 0)]);
        assert_eq!(average_mhz(&s, &[400, 800]), None);
    }

    #[test]
    fn scales_hz_and_khz() {
        let pair = |f: u32| [f.to_le_bytes(), 0u32.to_le_bytes()].concat();
        assert_eq!(to_mhz(&[pair(600_000_000), pair(3_228_000_000)].concat()), [600, 3228]);
        assert_eq!(to_mhz(&[pair(1_020_000), pair(4_512_000)].concat()), [1020, 4512]);
    }
}
