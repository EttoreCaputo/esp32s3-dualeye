//! Host side of the DualEye PC monitor.
//!
//! [`Collector`] reads CPU, GPU, memory and fan sensors straight from the OS (no
//! CoolerControl or other daemon), [`Snapshot`] is the JSON the firmware
//! parses, [`Link`] speaks the board's USB protocol (framing in [`protocol`],
//! JSON-RPC and board tools), [`Bridge`] ties them together on a background
//! thread, [`Hub`] shares the bridge's connection with other processes and
//! [`Board`] uses it (or the port, when no bridge runs), and [`Esptool`] identifies and flashes the board.
//! [`firmware`] reads the version of a flash image and of the board's firmware.
//! [`ClaudeUsage`] adds Claude Code's usage for the Claude faces. The CLI
//! and a Tauri app are both thin shells over this.

pub mod agent;
pub mod bridge;
pub mod claude;
#[cfg(feature = "download")]
pub mod download;
pub mod eval;
pub mod firmware;
pub mod flasher;
pub mod hub;
pub mod intents;
pub mod link;
pub mod llm;
pub mod models;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod protocol;
pub mod sensors;
pub mod serial;
pub(crate) mod sidecar;
pub mod snapshot;
pub mod stt;
pub mod tts;
pub mod voice;

pub use agent::{Agent, Toolbox, Turn};
pub use bridge::{Bridge, BridgeConfig, BridgeEvent};
pub use claude::{ClaudeMetrics, ClaudeState, ClaudeUsage};
pub use firmware::{BoardFirmware, ImageInfo};
pub use flasher::{ChipInfo, Esptool, FlashEvent};
pub use hub::{Board, Hub, HubStatus, Route};
pub use link::{CallError, Hello, Link, LinkEvent, Tool, ToolResult};
pub use llm::{Llm, LlmConfig};
pub use sensors::{Collector, Reading};
pub use serial::PortInfo;
pub use stt::{Stt, SttConfig, SttLanguage, Transcript};
pub use tts::{Tts, TtsConfig};
pub use voice::{Speaker, Spoken, Utterance, VoiceConfig};
pub use snapshot::{DeviceMetrics, Face, Faces, Fan, Memory, Rotation, Rotations, Snapshot};
