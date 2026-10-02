//! Pictures for the image face: [`prepare`] turns a PNG, JPEG, WebP, BMP or
//! (animated) GIF into what the board plays, and [`upload`] sends it.
//!
//! The board does no decoding: each frame is cropped to the middle square,
//! scaled to 240 x 240 and turned into RGB565, raw or run-length coded,
//! whichever is shorter. The format is in `main/media.h`. An animation is
//! thinned until it fits the board's slot and plays at most [`MAX_FPS`].

use std::io::Cursor;
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::{self, FilterType};
use image::{AnimationDecoder, ImageFormat, RgbaImage};
use serde::Serialize;
use serde_json::{Value, json};

use crate::hub::Board;
use crate::link::CallError;

pub const SIZE: u32 = 240;
/// What the board plays smoothly with both screens animating.
pub const MAX_FPS: u32 = 20;
/// The slot of firmware 1.1, for when the board can't be asked.
pub const DEFAULT_SLOT: usize = 0x7F_0000 / 2;
const MAX_FRAMES: usize = 512;
const HEADER: usize = 16;
const ENTRY: usize = 12;
/// Browsers play a GIF frame of 0 or 10 ms for 100 ms.
const SHORTEST_GIF_DELAY_MS: u32 = 20;
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(5);

/// One frame as the board gets it.
struct Frame {
    data: Vec<u8>,
    delay_ms: u16,
    rle: bool,
}

/// An image ready for [`upload`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Prepared {
    #[serde(skip)]
    pub blob: Vec<u8>,
    pub frames: usize,
    /// Frames the source had, before thinning.
    pub source_frames: usize,
    pub bytes: usize,
    /// One loop, in ms; 0 for a still picture.
    pub duration_ms: u32,
}

/// Decode `bytes` and make the board's image of it, no bigger than `slot` bytes.
pub fn prepare(bytes: &[u8], slot: usize) -> Result<Prepared, String> {
    let frames = decode(bytes)?;
    let source_frames = frames.len();
    let mut frames: Vec<(RgbaImage, u32)> = frames.into_iter().map(|(img, delay)| (square(&img), delay)).collect();
    frames = limit_rate(frames, 1000 / MAX_FPS);
    loop {
        let encoded: Vec<Frame> = frames.iter().map(|(img, delay)| encode_frame(img, *delay)).collect();
        let blob = assemble(&encoded);
        if blob.len() <= slot && encoded.len() <= MAX_FRAMES {
            let duration_ms = if encoded.len() > 1 { encoded.iter().map(|f| u32::from(f.delay_ms)).sum() } else { 0 };
            return Ok(Prepared { bytes: blob.len(), frames: encoded.len(), blob, source_frames, duration_ms });
        }
        if frames.len() == 1 {
            return Err(format!("the image takes {} bytes, the board has room for {slot}", blob.len()));
        }
        frames = thin(frames);
    }
}

/// Every frame with its delay in ms: one for a still picture.
fn decode(bytes: &[u8]) -> Result<Vec<(RgbaImage, u32)>, String> {
    let format = image::guess_format(bytes).map_err(|_| "not a picture this app can read (PNG, JPEG, GIF, WebP or BMP)".to_string())?;
    let animated: Option<Vec<image::Frame>> = match format {
        ImageFormat::Gif => {
            let decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
            Some(decoder.into_frames().collect_frames().map_err(|e| e.to_string())?)
        }
        ImageFormat::WebP => {
            let decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
            if decoder.has_animation() { Some(decoder.into_frames().collect_frames().map_err(|e| e.to_string())?) } else { None }
        }
        _ => None,
    };
    match animated {
        Some(frames) if !frames.is_empty() => Ok(frames
            .into_iter()
            .map(|f| {
                let (num, den) = f.delay().numer_denom_ms();
                let ms = num.checked_div(den).unwrap_or(100);
                (f.into_buffer(), if ms < SHORTEST_GIF_DELAY_MS { 100 } else { ms })
            })
            .collect()),
        _ => {
            let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
            Ok(vec![(img.to_rgba8(), 0)])
        }
    }
}

/// The middle square, scaled to the screen.
fn square(img: &RgbaImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    let side = w.min(h);
    let cropped = imageops::crop_imm(img, (w - side) / 2, (h - side) / 2, side, side).to_image();
    imageops::resize(&cropped, SIZE, SIZE, FilterType::Triangle)
}

/// Merge frames shorter than `min_ms` into the next one kept.
fn limit_rate(frames: Vec<(RgbaImage, u32)>, min_ms: u32) -> Vec<(RgbaImage, u32)> {
    let mut out: Vec<(RgbaImage, u32)> = Vec::new();
    for (img, delay) in frames {
        match out.last_mut() {
            Some(last) if last.1 < min_ms => last.1 += delay,
            _ => out.push((img, delay)),
        }
    }
    out
}

/// Every other frame, each kept one lasting for both.
fn thin(frames: Vec<(RgbaImage, u32)>) -> Vec<(RgbaImage, u32)> {
    let mut out: Vec<(RgbaImage, u32)> = Vec::with_capacity(frames.len().div_ceil(2));
    for (i, (img, delay)) in frames.into_iter().enumerate() {
        if i % 2 == 0 {
            out.push((img, delay));
        } else if let Some(last) = out.last_mut() {
            last.1 += delay;
        }
    }
    out
}

/// RGB565, little-endian; transparency over black.
fn rgb565(img: &RgbaImage) -> Vec<u16> {
    img.pixels()
        .map(|p| {
            let [r, g, b, a] = p.0;
            let blend = |c: u8| (u16::from(c) * u16::from(a) / 255) as u8;
            let (r, g, b) = (blend(r), blend(g), blend(b));
            (u16::from(r >> 3) << 11) | (u16::from(g >> 2) << 5) | u16::from(b >> 3)
        })
        .collect()
}

/// Runs of equal pixels and stretches of others, as `media_decode` reads them.
fn rle(pixels: &[u16]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < pixels.len() {
        let mut run = 1;
        while i + run < pixels.len() && pixels[i + run] == pixels[i] && run < 0x8000 {
            run += 1;
        }
        if run >= 3 {
            out.extend_from_slice(&(0x8000 | (run - 1) as u16).to_le_bytes());
            out.extend_from_slice(&pixels[i].to_le_bytes());
            i += run;
            continue;
        }
        // Literals up to the next run of three.
        let start = i;
        while i < pixels.len() && i - start < 0x8000 {
            if i + 2 < pixels.len() && pixels[i] == pixels[i + 1] && pixels[i] == pixels[i + 2] {
                break;
            }
            i += 1;
        }
        out.extend_from_slice(&((i - start - 1) as u16).to_le_bytes());
        for p in &pixels[start..i] {
            out.extend_from_slice(&p.to_le_bytes());
        }
    }
    out
}

fn encode_frame(img: &RgbaImage, delay_ms: u32) -> Frame {
    let pixels = rgb565(img);
    let raw: Vec<u8> = pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
    let coded = rle(&pixels);
    let delay_ms = delay_ms.min(u32::from(u16::MAX)) as u16;
    if coded.len() < raw.len() { Frame { data: coded, delay_ms, rle: true } } else { Frame { data: raw, delay_ms, rle: false } }
}

fn assemble(frames: &[Frame]) -> Vec<u8> {
    let table = HEADER + frames.len() * ENTRY;
    let mut blob = Vec::with_capacity(table + frames.iter().map(|f| f.data.len()).sum::<usize>());
    blob.extend_from_slice(b"DEIM");
    blob.extend_from_slice(&[1, 0]);
    blob.extend_from_slice(&(frames.len() as u16).to_le_bytes());
    blob.extend_from_slice(&(SIZE as u16).to_le_bytes());
    blob.extend_from_slice(&(SIZE as u16).to_le_bytes());
    blob.extend_from_slice(&[0; 4]);
    let mut offset = table;
    for f in frames {
        blob.extend_from_slice(&(offset as u32).to_le_bytes());
        blob.extend_from_slice(&(f.data.len() as u32).to_le_bytes());
        blob.extend_from_slice(&f.delay_ms.to_le_bytes());
        blob.extend_from_slice(&[u8::from(f.rle), 0]);
        offset += f.data.len();
    }
    for f in frames {
        blob.extend_from_slice(&f.data);
    }
    blob
}

/// Send `image` to the board's slot for `screen` (`left`, `right`) with
/// `call` (a JSON-RPC request to the board), reporting the share sent.
pub fn upload(
    call: impl Fn(&str, Value, Duration) -> Result<Value, CallError>,
    screen: &str,
    image: &Prepared,
    mut progress: impl FnMut(f32),
) -> Result<(), CallError> {
    let crc = crc32fast::hash(&image.blob);
    let begin = call("media/begin", json!({"screen": screen, "size": image.blob.len(), "crc32": crc}), UPLOAD_TIMEOUT)?;
    let chunk = begin["chunk"].as_u64().unwrap_or(2048).clamp(256, 2304) as usize;
    let engine = base64::engine::general_purpose::STANDARD;
    for (i, part) in image.blob.chunks(chunk).enumerate() {
        let offset = i * chunk;
        call("media/write", json!({"offset": offset, "data": engine.encode(part)}), UPLOAD_TIMEOUT)?;
        progress((offset + part.len()) as f32 / image.blob.len() as f32);
    }
    call("media/end", json!({}), UPLOAD_TIMEOUT)?;
    Ok(())
}

/// Where a copy of the picture sent to each screen is kept, for the app's mirror.
pub fn copy_path(screen: &str) -> Option<PathBuf> {
    crate::claude::data_dir().map(|d| d.join("images").join(screen))
}

/// The copy kept for `screen` as a `data:` URL, for a webview.
pub fn copy_data_url(screen: &str) -> Option<String> {
    let bytes = std::fs::read(copy_path(screen)?).ok()?;
    let mime = match image::guess_format(&bytes).ok()? {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::WebP => "image/webp",
        ImageFormat::Bmp => "image/bmp",
        _ => return None,
    };
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

fn why(e: CallError) -> String {
    match e {
        CallError::Rpc { code: -32601, .. } => "the board's firmware has no image face: update it to 1.1 or later".into(),
        other => other.to_string(),
    }
}

/// Prepare `bytes` and put it on `screen` (`left`, `right`), keeping a copy
/// of the original for the mirror. `progress` gets the share sent.
pub fn send(board: &Board, screen: &str, bytes: &[u8], progress: impl FnMut(f32)) -> Result<Prepared, String> {
    let prepared = board
        .session(|call| {
            let slot = call("media/info", json!({}), UPLOAD_TIMEOUT)?["slot_size"].as_u64().map_or(DEFAULT_SLOT, |s| s as usize);
            let prepared = prepare(bytes, slot).map_err(CallError::Invalid)?;
            upload(call, screen, &prepared, progress)?;
            Ok(prepared)
        })
        .map_err(|e| match e {
            CallError::Invalid(msg) => msg,
            other => why(other),
        })?;
    if let Some(path) = copy_path(screen) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, bytes);
    }
    Ok(prepared)
}

/// Take the picture off `screen`.
pub fn clear(board: &Board, screen: &str) -> Result<(), String> {
    board.session(|call| call("media/clear", json!({"screen": screen}), UPLOAD_TIMEOUT)).map_err(why)?;
    if let Some(path) = copy_path(screen) {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_rle(data: &[u8]) -> Vec<u16> {
        let words: Vec<u16> = data.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < words.len() {
            let w = words[i];
            let n = usize::from(w & 0x7FFF) + 1;
            if w & 0x8000 != 0 {
                out.extend(std::iter::repeat_n(words[i + 1], n));
                i += 2;
            } else {
                out.extend_from_slice(&words[i + 1..i + 1 + n]);
                i += 1 + n;
            }
        }
        out
    }

    #[test]
    fn rle_round_trips() {
        let mut pixels = vec![7u16; 1000];
        pixels.extend([1, 2, 3, 3, 4, 4, 4, 4, 5]);
        pixels.extend(vec![9u16; 40000]);
        pixels.extend((0..500).map(|i| i as u16));
        assert_eq!(decode_rle(&rle(&pixels)), pixels);
        assert!(rle(&pixels).len() < pixels.len());
    }

    #[test]
    fn still_picture_is_one_frame() {
        let img = RgbaImage::from_pixel(480, 320, image::Rgba([255, 0, 0, 255]));
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png).unwrap();
        let p = prepare(&png, DEFAULT_SLOT).unwrap();
        assert_eq!((p.frames, p.duration_ms), (1, 0));
        assert_eq!(&p.blob[..4], b"DEIM");
        // Solid red codes to a few runs.
        assert!(p.bytes < 2000, "{}", p.bytes);
        assert_eq!(rgb565(&RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255])))[0], 0xF800);
    }

    #[test]
    fn animations_are_thinned_to_fit() {
        let frames: Vec<(RgbaImage, u32)> =
            (0..8).map(|i| (RgbaImage::from_fn(SIZE, SIZE, |x, y| image::Rgba([(x * 7 + i) as u8, (y * 13) as u8, (x ^ y) as u8, 255])), 30)).collect();
        // 30 ms frames merge into 60 ms ones at 20 fps.
        let limited = limit_rate(frames, 1000 / MAX_FPS);
        assert_eq!(limited.len(), 4);
        assert!(limited.iter().all(|(_, d)| *d == 60));
        let thinned = thin(limited);
        assert_eq!(thinned.iter().map(|(_, d)| d).collect::<Vec<_>>(), [&120, &120]);
    }
}
