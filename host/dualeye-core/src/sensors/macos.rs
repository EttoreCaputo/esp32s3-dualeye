//! macOS: what sysinfo's component list cannot give.
//!
//! - Temperatures from the SMC. Apple Silicon keys change with every chip
//!   generation, so each has its own list (the ones Stats uses); a chip not
//!   listed falls back to the key prefixes (`Tp`/`Te` CPU, `Tg` GPU). Intel
//!   Macs use `TC0x` for the CPU and `TG..`/`TCGC` for the GPU.
//! - Fan speeds from the SMC (`F<n>Ac`).
//! - CPU and GPU clocks and power from IOReport (`ioreport.rs`), Apple Silicon only.
//! - GPU load and memory from IOAccelerator's `PerformanceStatistics`.
//!
//! None of it needs root.

use std::ffi::{CStr, c_char, c_void};
use std::mem::size_of;

use core_foundation::base::{CFType, TCFType, kCFAllocatorDefault};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use io_kit_sys::types::io_connect_t;
use io_kit_sys::{
    IOConnectCallStructMethod, IOIteratorNext, IOObjectRelease, IORegistryEntryCreateCFProperty, IOServiceClose,
    IOServiceGetMatchingService, IOServiceGetMatchingServices, IOServiceMatching, IOServiceOpen, kIOMasterPortDefault,
};
use mach2::kern_return::KERN_SUCCESS;
use mach2::traps::mach_task_self;

use super::ioreport::IoReport;
use super::{PlatformSample, Reading, average};
use crate::snapshot::{Memory, round1};

/// `SMCKeyData_t` from Apple's SMC user client (80 bytes).
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct KeyData {
    key: u32,
    vers: [u8; 6],
    p_limit: PLimitData,
    info: KeyInfo,
    result: u8,
    status: u8,
    data8: u8,
    data32: u32,
    bytes: [u8; 32],
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct PLimitData {
    version: u16,
    length: u16,
    cpu: u32,
    gpu: u32,
    mem: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct KeyInfo {
    data_size: u32,
    data_type: u32,
    data_attributes: u8,
}

const _: () = assert!(size_of::<KeyData>() == 80);

const SMC_HANDLE_EVENT: u32 = 2;
const SMC_READ_BYTES: u8 = 5;
const SMC_KEY_AT_INDEX: u8 = 8;
const SMC_KEY_INFO: u8 = 9;

const fn fourcc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

struct Smc {
    conn: io_connect_t,
}

// The connection is a mach port, usable from any thread.
unsafe impl Send for Smc {}

impl Smc {
    fn open() -> Option<Self> {
        unsafe {
            let service = IOServiceGetMatchingService(kIOMasterPortDefault, IOServiceMatching(c"AppleSMC".as_ptr()));
            if service == 0 {
                return None;
            }
            let mut conn = 0;
            let kr = IOServiceOpen(service, mach_task_self(), 0, &mut conn);
            IOObjectRelease(service);
            (kr == KERN_SUCCESS).then_some(Self { conn })
        }
    }

    fn call(&self, input: KeyData) -> Option<KeyData> {
        let mut out = KeyData::default();
        let mut size = size_of::<KeyData>();
        let kr = unsafe {
            IOConnectCallStructMethod(
                self.conn,
                SMC_HANDLE_EVENT,
                (&input as *const KeyData).cast::<c_void>(),
                size_of::<KeyData>(),
                (&mut out as *mut KeyData).cast::<c_void>(),
                &mut size,
            )
        };
        (kr == KERN_SUCCESS && out.result == 0).then_some(out)
    }

    fn key_info(&self, key: u32) -> Option<KeyInfo> {
        self.call(KeyData { key, data8: SMC_KEY_INFO, ..Default::default() }).map(|o| o.info)
    }

    fn read(&self, key: u32, info: KeyInfo) -> Option<[u8; 32]> {
        self.call(KeyData { key, info, data8: SMC_READ_BYTES, ..Default::default() }).map(|o| o.bytes)
    }

    fn key_at(&self, index: u32) -> Option<u32> {
        self.call(KeyData { data32: index, data8: SMC_KEY_AT_INDEX, ..Default::default() }).map(|o| o.key)
    }

    fn key_count(&self) -> u32 {
        let key = fourcc(b"#KEY");
        self.key_info(key).and_then(|i| self.read(key, i)).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// `flt ` on Apple Silicon; `sp78` (signed 8.8 fixed point), `fpe2`
    /// (unsigned 14.2, fan speeds) and plain integers on Intel.
    fn value(&self, key: &SmcKey) -> Option<f64> {
        let b = self.read(key.code, key.info)?;
        let v = match &key.info.data_type.to_be_bytes() {
            b"flt " => f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            b"sp78" => f64::from(i16::from_be_bytes([b[0], b[1]])) / 256.0,
            b"fpe2" => f64::from(u16::from_be_bytes([b[0], b[1]])) / 4.0,
            b"ui8 " => f64::from(b[0]),
            b"ui16" => f64::from(u16::from_be_bytes([b[0], b[1]])),
            b"ui32" => f64::from(u32::from_be_bytes([b[0], b[1], b[2], b[3]])),
            _ => return None,
        };
        v.is_finite().then_some(v)
    }

    fn temperature(&self, key: &SmcKey) -> Option<f64> {
        self.value(key).filter(|t| *t > 0.0 && *t < 150.0)
    }

    fn rpm(&self, key: &SmcKey) -> Option<u32> {
        self.value(key).filter(|r| (0.0..100_000.0).contains(r)).map(|r| r.round() as u32)
    }
}

impl Drop for Smc {
    fn drop(&mut self) {
        unsafe { IOServiceClose(self.conn) };
    }
}

#[derive(Clone)]
struct SmcKey {
    code: u32,
    name: String,
    info: KeyInfo,
}

/// CPU and GPU temperature keys per Apple Silicon generation (Stats'
/// `Modules/Sensors/values.swift`). Keys a model lacks just do not read back.
fn generation_keys(brand: &str) -> Option<(&'static [&'static str], &'static [&'static str])> {
    let generation = brand.strip_prefix("Apple M")?.chars().next()?.to_digit(10)?;
    Some(match generation {
        1 => (
            &["Tp09", "Tp0T", "Tp01", "Tp05", "Tp0D", "Tp0H", "Tp0L", "Tp0P", "Tp0X", "Tp0b"],
            &["Tg05", "Tg0D", "Tg0L", "Tg0T"],
        ),
        2 => (
            &["Tp1h", "Tp1t", "Tp1p", "Tp1l", "Tp01", "Tp05", "Tp09", "Tp0D", "Tp0X", "Tp0b", "Tp0f", "Tp0j"],
            &["Tg0f", "Tg0j"],
        ),
        3 => (
            &[
                "Te05", "Te0L", "Te0P", "Te0S", "Tf04", "Tf09", "Tf0A", "Tf0B", "Tf0D", "Tf0E", "Tf44", "Tf49", "Tf4A",
                "Tf4B", "Tf4D", "Tf4E",
            ],
            &["Tf14", "Tf18", "Tf19", "Tf1A", "Tf24", "Tf28", "Tf29", "Tf2A"],
        ),
        4 => (
            &["Te05", "Te0S", "Te09", "Te0H", "Tp01", "Tp05", "Tp09", "Tp0D", "Tp0V", "Tp0Y", "Tp0b", "Tp0e"],
            &["Tg0G", "Tg0H", "Tg1U", "Tg1k", "Tg0K", "Tg0L", "Tg0d", "Tg0e", "Tg0j", "Tg0k"],
        ),
        5 => (
            &[
                "Tp00", "Tp04", "Tp08", "Tp0C", "Tp0G", "Tp0K", "Tp0O", "Tp0R", "Tp0U", "Tp0X", "Tp0a", "Tp0d", "Tp0g",
                "Tp0j", "Tp0m", "Tp0p", "Tp0u", "Tp0y",
            ],
            &["Tg0U", "Tg0X", "Tg0d", "Tg0g", "Tg0j", "Tg1Y", "Tg1c"],
        ),
        _ => return None,
    })
}

/// For chips not in [`generation_keys`]: Apple Silicon `Tp`/`Te`, Intel `TC<n>.`.
fn is_cpu_key(name: &[u8; 4]) -> bool {
    name.starts_with(b"Tp") || name.starts_with(b"Te") || (name.starts_with(b"TC") && name[2].is_ascii_digit())
}

fn is_gpu_key(name: &[u8; 4]) -> bool {
    name.starts_with(b"Tg") || name.starts_with(b"TG") || name == b"TCGC"
}

/// `F<n>Ac`: a fan's actual speed.
fn is_fan_key(name: &[u8; 4]) -> bool {
    name[0] == b'F' && name[1].is_ascii_digit() && &name[2..] == b"Ac"
}

#[derive(Default)]
struct SmcKeys {
    cpu: Vec<SmcKey>,
    gpu: Vec<SmcKey>,
    fans: Vec<SmcKey>,
}

/// Walk the SMC's key table once for the temperature and fan keys that read back.
fn discover(smc: &Smc, brand: &str) -> SmcKeys {
    let mut temps = Vec::new();
    let mut fans = Vec::new();
    for code in (0..smc.key_count()).filter_map(|i| smc.key_at(i)) {
        let name = code.to_be_bytes();
        let fan = is_fan_key(&name);
        if !fan && name[0] != b'T' {
            continue;
        }
        let Some(info) = smc.key_info(code) else {
            continue;
        };
        let key = SmcKey { code, name: String::from_utf8_lossy(&name).into_owned(), info };
        if fan {
            if smc.rpm(&key).is_some() {
                fans.push(key);
            }
        } else if smc.temperature(&key).is_some() {
            temps.push(key);
        }
    }
    let pick = |names: &[&str]| -> Vec<SmcKey> {
        temps.iter().filter(|k| names.contains(&k.name.as_str())).cloned().collect()
    };
    let (mut cpu, mut gpu) = generation_keys(brand).map(|(c, g)| (pick(c), pick(g))).unwrap_or_default();
    let code = |k: &SmcKey| k.code.to_be_bytes();
    if gpu.is_empty() {
        gpu = temps.iter().filter(|k| is_gpu_key(&code(k))).cloned().collect();
    }
    if cpu.is_empty() {
        cpu = temps.iter().filter(|k| is_cpu_key(&code(k)) && !is_gpu_key(&code(k))).cloned().collect();
    }
    fans.sort_by_key(|k| k.code);
    SmcKeys { cpu, gpu, fans }
}

/// What one IOAccelerator reports.
#[derive(Default)]
struct AccelStats {
    utilization: Option<f64>,
    used: Option<f64>,
    /// Only discrete GPUs have their own memory; Apple Silicon shares the RAM.
    total: Option<f64>,
}

fn number(dict: &CFDictionary, key: &str) -> Option<f64> {
    let key = CFString::new(key);
    let value = dict.find(key.as_CFTypeRef().cast())?;
    let value = unsafe { CFType::wrap_under_get_rule(*value) };
    value.downcast::<CFNumber>()?.to_f64()
}

fn accelerators() -> Vec<AccelStats> {
    let mut out = Vec::new();
    unsafe {
        let mut iter = 0;
        if IOServiceGetMatchingServices(kIOMasterPortDefault, IOServiceMatching(c"IOAccelerator".as_ptr()), &mut iter)
            != KERN_SUCCESS
        {
            return out;
        }
        let key = CFString::from_static_string("PerformanceStatistics");
        loop {
            let entry = IOIteratorNext(iter);
            if entry == 0 {
                break;
            }
            let prop = IORegistryEntryCreateCFProperty(entry, key.as_concrete_TypeRef(), kCFAllocatorDefault, 0);
            IOObjectRelease(entry);
            if prop.is_null() {
                continue;
            }
            let Some(stats) = CFType::wrap_under_create_rule(prop).downcast_into::<CFDictionary>() else {
                continue;
            };
            let vram_used = number(&stats, "vramUsedBytes");
            out.push(AccelStats {
                utilization: number(&stats, "Device Utilization %").or_else(|| number(&stats, "GPU Activity(%)")),
                used: vram_used.or_else(|| number(&stats, "In use system memory")),
                total: vram_used.zip(number(&stats, "vramFreeBytes")).map(|(u, f)| u + f),
            });
        }
        IOObjectRelease(iter);
    }
    out
}

/// `machdep.cpu.brand_string`: "Apple M3 Pro", "Intel(R) Core(TM) i9-9880H …".
fn cpu_brand() -> String {
    unsafe extern "C" {
        fn sysctlbyname(
            name: *const c_char,
            old: *mut c_void,
            oldlen: *mut usize,
            new: *mut c_void,
            newlen: usize,
        ) -> i32;
    }
    let mut buf = [0u8; 128];
    let mut len = buf.len();
    let ok = unsafe {
        sysctlbyname(c"machdep.cpu.brand_string".as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0)
            == 0
    };
    if !ok {
        return String::new();
    }
    CStr::from_bytes_until_nul(&buf).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

pub struct MacSensors {
    smc: Option<Smc>,
    keys: SmcKeys,
    report: Option<IoReport>,
}

impl MacSensors {
    pub fn new() -> Self {
        let smc = Smc::open();
        let keys = smc.as_ref().map(|s| discover(s, &cpu_brand())).unwrap_or_default();
        Self { smc, keys, report: IoReport::new() }
    }

    fn temps<'a>(&self, keys: &'a [SmcKey]) -> Vec<(&'a SmcKey, f64)> {
        let Some(smc) = &self.smc else {
            return Vec::new();
        };
        keys.iter().filter_map(|k| Some((k, smc.temperature(k)?))).collect()
    }

    fn fans(&self) -> Vec<(&SmcKey, u32)> {
        let Some(smc) = &self.smc else {
            return Vec::new();
        };
        self.keys.fans.iter().filter_map(|k| Some((k, smc.rpm(k)?))).collect()
    }

    /// Fill in what the component list could not; `ram_total` stands in for
    /// the memory size of a GPU that shares the RAM. Clocks and power are
    /// averages since the previous call (or [`MacSensors::readings`]).
    pub fn fill(&mut self, out: &mut PlatformSample, ram_total: u64) {
        let average_of = |t: Vec<(&SmcKey, f64)>| average(&t.into_iter().map(|(_, t)| t).collect::<Vec<_>>());
        // The SMC has a sensor per core; IOHID's are not named after the CPU on every chip.
        if let Some(t) = average_of(self.temps(&self.keys.cpu)) {
            out.cpu_temp = Some(t);
        }
        if let Some(t) = average_of(self.temps(&self.keys.gpu)) {
            out.gpu.temp_c = Some(round1(t));
        }
        out.board_fans = self.fans().into_iter().map(|(_, rpm)| rpm).collect();

        if let Some(s) = self.report.as_mut().and_then(IoReport::sample) {
            out.cpu_clock = s.cpu_mhz.or(out.cpu_clock);
            out.cpu_power = s.cpu_w.or(out.cpu_power);
            out.gpu.clock_mhz = s.gpu_mhz;
            out.gpu.power_w = s.gpu_w.map(round1);
        }

        // The busiest one is the GPU in use (Intel Macs switch between two).
        let accels = accelerators();
        let Some(busy) =
            accels.iter().max_by(|a, b| a.utilization.unwrap_or(0.0).total_cmp(&b.utilization.unwrap_or(0.0)))
        else {
            return;
        };
        out.gpu.load_pct = busy.utilization.map(|u| round1(u.clamp(0.0, 100.0)));
        if let Some(used) = busy.used {
            out.gpu.mem = Memory::from_bytes(used as u64, busy.total.map_or(ram_total, |t| t as u64));
        }
    }

    pub fn readings(&mut self) -> Vec<Reading> {
        let smc = |label: String, value: f64, unit| Reading { source: "smc".into(), label, value, unit };
        let mut out: Vec<Reading> = Vec::new();
        out.extend(self.temps(&self.keys.cpu).into_iter().map(|(k, t)| smc(format!("CPU {}", k.name), t, "°C")));
        out.extend(self.temps(&self.keys.gpu).into_iter().map(|(k, t)| smc(format!("GPU {}", k.name), t, "°C")));
        out.extend(self.fans().into_iter().map(|(k, rpm)| smc(format!("Fan {}", k.name), f64::from(rpm), "RPM")));
        if let Some(s) = self.report.as_mut().and_then(IoReport::sample) {
            let ior =
                |label: &str, value: f64, unit| Reading { source: "ioreport".into(), label: label.into(), value, unit };
            out.extend(s.cpu_mhz.map(|f| ior("CPU clock", f64::from(f), "MHz")));
            out.extend(s.cpu_w.map(|w| ior("CPU power", w, "W")));
            out.extend(s.gpu_mhz.map(|f| ior("GPU clock", f64::from(f), "MHz")));
            out.extend(s.gpu_w.map(|w| ior("GPU power", w, "W")));
        }
        for (i, a) in accelerators().iter().enumerate() {
            if let Some(u) = a.utilization {
                out.push(Reading {
                    source: "ioaccelerator".into(),
                    label: format!("GPU {i} utilization"),
                    value: u,
                    unit: "%",
                });
            }
            if let Some(used) = a.used {
                let mb = used / (1024.0 * 1024.0);
                out.push(Reading {
                    source: "ioaccelerator".into(),
                    label: format!("GPU {i} memory in use"),
                    value: mb,
                    unit: "MB",
                });
            }
        }
        out
    }
}
