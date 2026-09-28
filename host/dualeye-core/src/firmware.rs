//! Which firmware is where: the version built into a flash image, and the one
//! running on the board.
//!
//! ESP-IDF stamps `version.txt` into the app descriptor (`esp_app_desc_t`) of
//! every image. Firmware 0.4 and later reports it in the protocol v2 `hello`
//! handshake (see [`crate::link`]). Firmware 0.2 and 0.3 print it as
//! `{"dualeye":"0.3.0","idf":"v6.1"}` when they boot and whenever they read
//! [`VERSION_QUERY`], which is enough to tell the app it needs an update.

use serde::{Deserialize, Serialize};

/// Sent to a protocol 1 board, which answers with its version line.
pub const VERSION_QUERY: &str = "?version\n";

const PARTITION_TABLE: usize = 0x8000;
const PARTITION_MAGIC: [u8; 2] = [0xAA, 0x50];
const APP_DESC_MAGIC: u32 = 0xABCD_5432;
/// `esp_image_header_t` + the first `esp_image_segment_header_t`.
const APP_DESC_OFFSET: usize = 24 + 8;

/// What the app descriptor of a flash image says about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageInfo {
    pub version: String,
    pub project: String,
    pub idf: String,
    /// Build date and time, as `__DATE__ __TIME__` put it.
    pub built: String,
}

/// What the board said about its firmware.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BoardFirmware {
    /// DualEye firmware that reported its version. `protocol` is 2 from
    /// firmware 0.4.0; a board on 1 needs a firmware update to work with this host.
    Version { version: String, idf: Option<String>, protocol: u32 },
    /// DualEye firmware from before versioning: it shows snapshots but can't say which it is.
    Legacy,
    /// The ROM finds no bootable app in flash.
    Missing,
}

/// Read the app descriptor of a merged image (bootloader + partition table +
/// app): the first app partition in the table, at its offset in the image.
pub fn image_info(image: &[u8]) -> Option<ImageInfo> {
    let table = image.get(PARTITION_TABLE..PARTITION_TABLE + 0xC00)?;
    let app = table
        .chunks_exact(32)
        .take_while(|e| e[..2] == PARTITION_MAGIC)
        .find(|e| e[2] == 0)
        .map(|e| u32::from_le_bytes(e[4..8].try_into().unwrap()) as usize)?;
    let desc = image.get(app + APP_DESC_OFFSET..app + APP_DESC_OFFSET + 256)?;
    if u32::from_le_bytes(desc[..4].try_into().unwrap()) != APP_DESC_MAGIC {
        return None;
    }
    let text = |range: std::ops::Range<usize>| {
        let raw = &desc[range];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        String::from_utf8_lossy(&raw[..end]).into_owned()
    };
    Some(ImageInfo {
        version: text(16..48),
        project: text(48..80),
        built: format!("{} {}", text(96..112), text(80..96)),
        idf: text(112..144),
    })
}

/// A protocol 1 firmware's answer to [`VERSION_QUERY`], wherever it sits in a console line.
pub fn parse_version_line(line: &str) -> Option<BoardFirmware> {
    #[derive(Deserialize)]
    struct Reply {
        dualeye: String,
        idf: Option<String>,
    }
    let start = line.find(r#"{"dualeye":"#)?;
    let end = start + line[start..].find('}')? + 1;
    let reply: Reply = serde_json::from_str(&line[start..end]).ok()?;
    Some(BoardFirmware::Version { version: reply.dualeye, idf: reply.idf, protocol: 1 })
}

/// `invalid header: 0xffffffff`: the ROM, looping over an erased flash.
pub(crate) fn is_missing_app_log(line: &str) -> bool {
    line.contains("invalid header")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(version: &str) -> Vec<u8> {
        let mut img = vec![0xFF; 0x10000 + 0x200];
        let mut entry = |i: usize, kind: u8, offset: u32| {
            let e = &mut img[PARTITION_TABLE + i * 32..PARTITION_TABLE + i * 32 + 32];
            e[..2].copy_from_slice(&PARTITION_MAGIC);
            e[2] = kind;
            e[4..8].copy_from_slice(&offset.to_le_bytes());
        };
        entry(0, 1, 0x9000);
        entry(1, 0, 0x10000);
        let desc = &mut img[0x10000 + APP_DESC_OFFSET..0x10000 + APP_DESC_OFFSET + 256];
        desc.fill(0);
        desc[..4].copy_from_slice(&APP_DESC_MAGIC.to_le_bytes());
        desc[16..16 + version.len()].copy_from_slice(version.as_bytes());
        desc[48..51].copy_from_slice(b"eye");
        desc[80..88].copy_from_slice(b"16:05:04");
        desc[96..107].copy_from_slice(b"Sep 26 2026");
        desc[112..116].copy_from_slice(b"v6.1");
        img
    }

    #[test]
    fn reads_the_app_descriptor() {
        let info = image_info(&image("0.2.0")).unwrap();
        assert_eq!(info.version, "0.2.0");
        assert_eq!(info.project, "eye");
        assert_eq!(info.idf, "v6.1");
        assert_eq!(info.built, "Sep 26 2026 16:05:04");
        assert_eq!(image_info(&[0; 100]), None);
    }

    #[test]
    fn finds_the_version_reply_in_a_console_line() {
        let want = BoardFirmware::Version { version: "0.2.0".into(), idf: Some("v6.1".into()), protocol: 1 };
        assert_eq!(parse_version_line(r#"{"dualeye":"0.2.0","idf":"v6.1"}"#), Some(want.clone()));
        assert_eq!(parse_version_line(r#"I (9) x: {"dualeye":"0.2.0","idf":"v6.1"}"#), Some(want));
        assert_eq!(parse_version_line(r#"{"v":1,"cpu":{}}"#), None);
    }
}
