//! Host side of the DualEye PC monitor.
//!
//! [`Collector`] reads CPU, GPU, memory and fan sensors straight from the OS (no
//! CoolerControl or other daemon), [`Snapshot`] is the JSON line the firmware
//! parses, and [`Bridge`] ties them to the board's USB serial port on a
//! background thread, and [`Esptool`] identifies and flashes the board.
//! [`firmware`] reads the version of a flash image and of the board's firmware.
//! [`ClaudeUsage`] adds Claude Code's usage for the Claude faces. The CLI
//! and a Tauri app are both thin shells over this.

pub mod bridge;
pub mod claude;
pub mod firmware;
pub mod flasher;
pub mod sensors;
pub mod serial;
pub mod snapshot;

pub use bridge::{Bridge, BridgeConfig, BridgeEvent};
pub use claude::{ClaudeMetrics, ClaudeState, ClaudeUsage};
pub use firmware::{BoardFirmware, ImageInfo};
pub use flasher::{ChipInfo, Esptool, FlashEvent};
pub use sensors::{Collector, Reading};
pub use serial::PortInfo;
pub use snapshot::{DeviceMetrics, Face, Faces, Fan, Memory, Rotation, Rotations, Snapshot};
