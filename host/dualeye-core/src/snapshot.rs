//! Sensor snapshot, the payload of a protocol v2 `metrics` frame, parsed by
//! `main/metrics_parser.c`.

use serde::{Deserialize, Serialize};

use crate::claude::ClaudeMetrics;

/// The snapshot format's version, the `v` field.
pub const SNAPSHOT_VERSION: u32 = 2;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeviceMetrics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temp_c: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_mhz: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_w: Option<f32>,
    /// System RAM under `cpu`, VRAM under `gpu`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem: Option<Memory>,
}

impl DeviceMetrics {
    pub fn is_empty(&self) -> bool {
        self.temp_c.is_none()
            && self.load_pct.is_none()
            && self.clock_mhz.is_none()
            && self.power_w.is_none()
            && self.mem.is_none()
    }
}

/// In MiB.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    pub used_mb: u32,
    pub total_mb: u32,
}

impl Memory {
    pub fn from_bytes(used: u64, total: u64) -> Option<Self> {
        const MIB: u64 = 1024 * 1024;
        (total > 0).then(|| Self { used_mb: (used / MIB) as u32, total_mb: (total / MIB) as u32 })
    }
}

/// Watch face one round screen shows. Mirrors `metrics_face_t` in `main/metrics_model.h`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Face {
    /// Temperature, clock, power, load ring and fan.
    #[default]
    Classic,
    /// Three concentric rings: load, temperature, memory.
    Rings,
    /// Classic plus a RAM (left screen) or VRAM (right screen) bar.
    Plus,
    /// Classic plus a smaller memory bar, without the numbers.
    Bar,
    /// Claude Code: 5-hour and weekly limit rings, tokens, a small Clawd.
    Claude,
    /// Claude Code: a large animated Clawd showing whether Claude is working.
    Clawd,
    /// Download and upload speed (firmware 1.1).
    Net,
    /// The system disk: space used, reads and writes (firmware 1.1).
    Disk,
    /// The laptop's battery (firmware 1.1).
    Battery,
    /// The picture or GIF uploaded for that screen (firmware 1.1).
    Image,
}

impl Face {
    pub const ALL: [Face; 10] = [
        Face::Classic,
        Face::Rings,
        Face::Plus,
        Face::Bar,
        Face::Claude,
        Face::Clawd,
        Face::Net,
        Face::Disk,
        Face::Battery,
        Face::Image,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Face::Classic => "classic",
            Face::Rings => "rings",
            Face::Plus => "plus",
            Face::Bar => "bar",
            Face::Claude => "claude",
            Face::Clawd => "clawd",
            Face::Net => "net",
            Face::Disk => "disk",
            Face::Battery => "battery",
            Face::Image => "image",
        }
    }

    /// Shows a CPU's or a GPU's metrics, as its screen's [`Source`] says.
    pub fn has_source(self) -> bool {
        matches!(self, Face::Classic | Face::Rings | Face::Plus | Face::Bar)
    }
}

/// Whose metrics a screen's classic, rings, plus and bar faces show. Mirrors
/// `metrics_source_t` in `main/metrics_model.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Cpu,
    Gpu,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Cpu => "cpu",
            Source::Gpu => "gpu",
        }
    }
}

impl std::str::FromStr for Source {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "cpu" => Ok(Source::Cpu),
            "gpu" => Ok(Source::Gpu),
            _ => Err(format!("unknown source `{s}`, expected cpu or gpu")),
        }
    }
}

/// The source of each screen: by default the CPU on the left, the GPU on the right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sources {
    #[serde(default = "cpu")]
    pub cpu: Source,
    #[serde(default = "gpu")]
    pub gpu: Source,
}

fn cpu() -> Source {
    Source::Cpu
}

fn gpu() -> Source {
    Source::Gpu
}

impl Default for Sources {
    fn default() -> Self {
        Self { cpu: Source::Cpu, gpu: Source::Gpu }
    }
}

impl std::str::FromStr for Face {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Face::ALL.into_iter().find(|f| f.name() == s).ok_or_else(|| {
            let names: Vec<_> = Face::ALL.iter().map(|f| f.name()).collect();
            format!("unknown face `{s}`, expected one of: {}", names.join(", "))
        })
    }
}

/// Which face each screen shows: left (`cpu`) and right (`gpu`), and whose
/// metrics the faces that have a source show there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Faces {
    #[serde(default, deserialize_with = "face_or_classic")]
    pub cpu: Face,
    #[serde(default, deserialize_with = "face_or_classic")]
    pub gpu: Face,
    #[serde(default)]
    pub src: Sources,
}

impl Faces {
    pub fn new(cpu: Face, gpu: Face) -> Self {
        Self { cpu, gpu, src: Sources::default() }
    }
}

/// Names this build doesn't know (such as the retired `memory` and `gauge`)
/// read as `classic`, so an old settings file still loads.
fn face_or_classic<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Face, D::Error> {
    Ok(String::deserialize(d)?.parse().unwrap_or_default())
}

/// Extra clockwise turn of one screen on top of the DualEye mounting, for a
/// board that sits another way round. On the wire, in degrees.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(into = "u16")]
pub enum Rotation {
    #[default]
    R0,
    R90,
    R180,
    R270,
}

impl Rotation {
    pub const ALL: [Rotation; 4] = [Rotation::R0, Rotation::R90, Rotation::R180, Rotation::R270];

    pub fn degrees(self) -> u16 {
        match self {
            Rotation::R0 => 0,
            Rotation::R90 => 90,
            Rotation::R180 => 180,
            Rotation::R270 => 270,
        }
    }

    pub fn from_degrees(degrees: u16) -> Option<Self> {
        Rotation::ALL.into_iter().find(|r| r.degrees() == degrees)
    }
}

impl From<Rotation> for u16 {
    fn from(r: Rotation) -> u16 {
        r.degrees()
    }
}

/// Anything but a quarter turn reads as upright, like the firmware does.
impl<'de> Deserialize<'de> for Rotation {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(u16::deserialize(d).ok().and_then(Rotation::from_degrees).unwrap_or_default())
    }
}

impl std::str::FromStr for Rotation {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.trim_end_matches('°')
            .parse()
            .ok()
            .and_then(Rotation::from_degrees)
            .ok_or_else(|| format!("unknown rotation `{s}`, expected one of: 0, 90, 180, 270"))
    }
}

/// How each screen is turned: left (`cpu`) and right (`gpu`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rotations {
    #[serde(default)]
    pub cpu: Rotation,
    #[serde(default)]
    pub gpu: Rotation,
}

impl Rotations {
    pub fn is_upright(&self) -> bool {
        *self == Rotations::default()
    }
}

/// `id` is what the UI keys on: `"cpu"` (case/radiator fans) or `"gpu"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fan {
    pub id: String,
    pub rpm: u32,
}

/// Network throughput over every interface but loopback, in bytes per second.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Net {
    pub rx_bps: u64,
    pub tx_bps: u64,
}

/// The system disk, in GB (10^9 bytes, as the OS shows them), and its
/// throughput where the OS tells.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Disk {
    pub used_gb: f32,
    pub total_gb: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_bps: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_bps: Option<u64>,
}

/// The laptop's battery.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Battery {
    pub pct: f32,
    pub charging: bool,
    /// On mains power: charging, full or held at a charge limit.
    pub plugged: bool,
    /// Minutes to empty, or to full while charging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mins: Option<u32>,
}

/// Through a string, so an `f32` reads `34.8` rather than `34.79999923706055`.
pub fn short_floats(value: &impl Serialize) -> serde_json::Value {
    serde_json::to_string(value).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(serde_json::Value::Null)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub v: u32,
    pub ts: u64,
    #[serde(default, skip_serializing_if = "DeviceMetrics::is_empty")]
    pub cpu: DeviceMetrics,
    #[serde(default, skip_serializing_if = "DeviceMetrics::is_empty")]
    pub gpu: DeviceMetrics,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fans: Vec<Fan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<Net>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk: Option<Disk>,
    /// Absent without a battery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bat: Option<Battery>,
    /// What the bridge has set the board's faces to, for frontends that
    /// mirror the screens. Not part of the payload: faces are board state
    /// since protocol v2, set with the `set_face` tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<Faces>,
    /// Likewise for the rotation (`set_rotation`); left out when both screens are upright.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rot: Option<Rotations>,
    /// Claude Code usage, also set by the bridge; absent where Claude Code
    /// hasn't run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude: Option<ClaudeMetrics>,
}

impl Snapshot {
    /// What the `get_metrics` tool returns (over MCP and to the voice
    /// agent): the sensor part, with the units spelled out.
    pub fn metrics_json(&self) -> serde_json::Value {
        serde_json::json!({
            "cpu": short_floats(&self.cpu),
            "gpu": short_floats(&self.gpu),
            "fans": self.fans,
            "net": self.net,
            "disk": short_floats(&self.disk),
            "battery": short_floats(&self.bat),
            "note": "temperatures in °C, clocks in MHz, power in W, memory in MiB (system RAM under cpu, VRAM under gpu), \
                     fan speeds in RPM, network and disk speeds in bytes per second, disk space in GB, \
                     battery in percent with minutes to empty (or to full while charging); battery is null on a desktop",
        })
    }

    /// The firmware drops lines that carry neither temperature.
    pub fn is_sendable(&self) -> bool {
        self.cpu.temp_c.is_some() || self.gpu.temp_c.is_some()
    }

    pub fn fan_rpm(&self, id: &str) -> Option<u32> {
        self.fans.iter().find(|f| f.id == id).map(|f| f.rpm)
    }

    /// The `metrics` frame payload: this snapshot without `face` and `rot`.
    pub fn to_payload(&self) -> Vec<u8> {
        let wire = Snapshot { face: None, rot: None, ..self.clone() };
        serde_json::to_vec(&wire).expect("snapshot is always serializable")
    }
}

pub(crate) fn round1(value: f64) -> f32 {
    ((value * 10.0).round() / 10.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(snap: &Snapshot) -> String {
        String::from_utf8(snap.to_payload()).unwrap()
    }

    #[test]
    fn line_matches_firmware_format() {
        let snap = Snapshot {
            v: SNAPSHOT_VERSION,
            ts: 1_700_000_000,
            cpu: DeviceMetrics {
                temp_c: Some(round1(36.33)),
                load_pct: Some(round1(1.81)),
                clock_mhz: Some(1572),
                power_w: Some(round1(14.596)),
                mem: Memory::from_bytes(12_884_901_888, 33_285_996_544),
            },
            gpu: DeviceMetrics {
                temp_c: Some(31.0),
                mem: Some(Memory { used_mb: 1024, total_mb: 24576 }),
                ..Default::default()
            },
            fans: vec![Fan { id: "cpu".into(), rpm: 3770 }],
            net: Some(Net { rx_bps: 1200, tx_bps: 30 }),
            disk: None,
            bat: Some(Battery { pct: 84.0, charging: true, plugged: true, mins: Some(42) }),
            face: Some(Faces::new(Face::Rings, Face::Plus)),
            rot: None,
            claude: None,
        };
        assert_eq!(
            payload(&snap),
            "{\"v\":2,\"ts\":1700000000,\"cpu\":{\"temp_c\":36.3,\"load_pct\":1.8,\"clock_mhz\":1572,\"power_w\":14.6,\
             \"mem\":{\"used_mb\":12288,\"total_mb\":31744}},\"gpu\":{\"temp_c\":31.0,\"mem\":{\"used_mb\":1024,\"total_mb\":24576}},\
             \"fans\":[{\"id\":\"cpu\",\"rpm\":3770}],\"net\":{\"rx_bps\":1200,\"tx_bps\":30},\
             \"bat\":{\"pct\":84.0,\"charging\":true,\"plugged\":true,\"mins\":42}}"
        );
    }

    #[test]
    fn empty_sections_are_omitted() {
        let snap = Snapshot {
            v: 1,
            ts: 0,
            cpu: DeviceMetrics::default(),
            gpu: DeviceMetrics::default(),
            fans: vec![],
            net: None,
            disk: None,
            bat: None,
            face: None,
            rot: None,
            claude: None,
        };
        assert_eq!(payload(&snap), "{\"v\":1,\"ts\":0}");
        assert!(!snap.is_sendable());
    }

    #[test]
    fn claude_section() {
        let snap = Snapshot {
            v: 1,
            ts: 0,
            cpu: DeviceMetrics { temp_c: Some(40.0), ..Default::default() },
            gpu: DeviceMetrics::default(),
            fans: vec![],
            net: None,
            disk: None,
            bat: None,
            face: Some(Faces::new(Face::Claude, Face::Clawd)),
            rot: None,
            claude: Some(ClaudeMetrics {
                tok: 1_234_567,
                today: 4_500_000,
                left_min: Some(133),
                s_pct: Some(42.0),
                w_pct: None,
                state: crate::ClaudeState::Work,
                model: Some("OPUS 5.5".into()),
            }),
        };
        assert_eq!(
            payload(&snap),
            "{\"v\":1,\"ts\":0,\"cpu\":{\"temp_c\":40.0},\
             \"claude\":{\"tok\":1234567,\"today\":4500000,\"left_min\":133,\"s_pct\":42.0,\"state\":\"work\",\"model\":\"OPUS 5.5\"}}"
        );
    }

    #[test]
    fn faces_parse_by_name() {
        assert_eq!("plus".parse::<Face>(), Ok(Face::Plus));
        assert_eq!("clawd".parse::<Face>(), Ok(Face::Clawd));
        assert!("gauge".parse::<Face>().is_err());
        let faces: Faces = serde_json::from_str("{\"gpu\":\"rings\"}").unwrap();
        assert_eq!(faces, Faces::new(Face::Classic, Face::Rings));
        let old: Faces = serde_json::from_str("{\"cpu\":\"gauge\",\"gpu\":\"memory\"}").unwrap();
        assert_eq!(old, Faces::default());
        assert_eq!(old.src, Sources { cpu: Source::Cpu, gpu: Source::Gpu });
        let swapped: Faces = serde_json::from_str("{\"cpu\":\"net\",\"src\":{\"cpu\":\"gpu\"}}").unwrap();
        assert_eq!(swapped.cpu, Face::Net);
        assert_eq!(swapped.src, Sources { cpu: Source::Gpu, gpu: Source::Gpu });
    }

    #[test]
    fn rotation_is_not_in_the_payload() {
        let snap = Snapshot {
            v: 1,
            ts: 0,
            cpu: DeviceMetrics { temp_c: Some(40.0), ..Default::default() },
            gpu: DeviceMetrics::default(),
            fans: vec![],
            net: None,
            disk: None,
            bat: None,
            face: None,
            rot: Some(Rotations { cpu: Rotation::R180, gpu: Rotation::R0 }),
            claude: None,
        };
        assert_eq!(payload(&snap), "{\"v\":1,\"ts\":0,\"cpu\":{\"temp_c\":40.0}}");
        // Frontends still see it on the snapshot.
        assert!(serde_json::to_string(&snap).unwrap().contains("\"rot\":{\"cpu\":180,\"gpu\":0}"));
        assert_eq!("90".parse::<Rotation>(), Ok(Rotation::R90));
        assert_eq!("270°".parse::<Rotation>(), Ok(Rotation::R270));
        assert!("45".parse::<Rotation>().is_err());
        let odd: Rotations = serde_json::from_str("{\"cpu\":45,\"gpu\":90}").unwrap();
        assert_eq!(odd, Rotations { cpu: Rotation::R0, gpu: Rotation::R90 });
        assert!(Rotations::default().is_upright());
    }
}
