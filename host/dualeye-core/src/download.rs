//! HTTPS downloads checked against a pinned SHA-256: esptool's Python and
//! the Whisper models.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

/// Download `url` into `dest`, reporting whole percents (`None` when the size
/// is unknown). A file that doesn't match `sha256` is removed.
pub fn download(url: &str, dest: &Path, sha256: &str, progress: impl FnMut(Option<f32>)) -> io::Result<()> {
    download_cancellable(url, dest, sha256, &AtomicBool::new(false), progress)
}

/// [`download`] that gives up (`ErrorKind::Interrupted`, `dest` removed) once
/// `cancel` is set.
pub fn download_cancellable(url: &str, dest: &Path, sha256: &str, cancel: &AtomicBool, mut progress: impl FnMut(Option<f32>)) -> io::Result<()> {
    let response = ureq::get(url).call().map_err(io::Error::other)?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let mut body = response.into_body().into_with_config().limit(u64::MAX).reader();
    let mut file = File::create(dest)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let (mut done, mut shown) = (0u64, -1i32);
    progress(total.map(|_| 0.0));
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = fs::remove_file(dest);
            return Err(io::Error::new(io::ErrorKind::Interrupted, "download cancelled"));
        }
        let n = body.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        done += n as u64;
        if let Some(total) = total {
            let percent = (done as f64 / total as f64 * 100.0) as i32;
            if percent != shown {
                shown = percent;
                progress(Some(percent as f32));
            }
        }
    }
    file.sync_all()?;
    let digest: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if digest != sha256 {
        let _ = fs::remove_file(dest);
        return Err(io::Error::new(io::ErrorKind::InvalidData, "checksum mismatch, download corrupted"));
    }
    Ok(())
}
