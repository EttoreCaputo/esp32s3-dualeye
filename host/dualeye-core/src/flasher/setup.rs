//! First-use setup of esptool, with nothing preinstalled on the machine:
//!
//! 1. a Python interpreter: the system one when it is usable (Linux and
//!    Windows, 3.10+ with `venv`), otherwise a portable CPython from
//!    [python-build-standalone](https://github.com/astral-sh/python-build-standalone),
//!    pinned and checked against its SHA-256;
//! 2. a virtualenv in `<dir>/venv`;
//! 3. `pip install esptool` into it.
//!
//! Every step is skipped when already done, and an interrupted setup is
//! redone from the step that failed. The output of the last setup is kept in
//! `<dir>/setup.log`.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::download::download;

use super::{Esptool, FlashEvent, READY_MARKER, command, run_streaming, venv_python};

/// What gets installed into the virtualenv.
pub const ESPTOOL_REQUIREMENT: &str = "esptool>=5.1,<6";

const PYTHON_RELEASE: &str = "20260924";
const PYTHON_VERSION: &str = "3.12.14";

/// `install_only_stripped` builds of [`PYTHON_VERSION`] for each supported host.
const PYTHON_BUILDS: &[(&str, &str, &str, &str)] = &[
    // (os, arch, target triple, sha256)
    ("linux", "x86_64", "x86_64-unknown-linux-gnu", "269b2c99e4db15b242bf01832f4fea1e8f1a664f273cff519393f296e9820b41"),
    ("linux", "aarch64", "aarch64-unknown-linux-gnu", "c8499b61252c433280f134df954464d19811527b31cb920c35fc6967c1222e35"),
    ("macos", "x86_64", "x86_64-apple-darwin", "7ea9761b9069c10b9a20531d568645849d604c59e9c7f11f6659f1e1790c968e"),
    ("macos", "aarch64", "aarch64-apple-darwin", "c2edb321cd32ec2b170df208db0446dccc4398db602ca27cf2079098fb1f7d9d"),
    ("windows", "x86_64", "x86_64-pc-windows-msvc", "c5bf8edfe858c1df9891be498b5bbc8761d383df5b9790658b088fea4870433a"),
    ("windows", "aarch64", "aarch64-pc-windows-msvc", "1d07bd9c97e6e1942b290bd2daa22d5a41e1e5acd2fd3d4c696892ceffe47144"),
];

/// esptool from `dir`, setting it up first if needed. Needs the network the
/// first time (Python and esptool are downloaded, about 30–50 MB).
pub fn ensure(dir: &Path, mut on_event: impl FnMut(FlashEvent)) -> io::Result<Esptool> {
    if let Some(tool) = Esptool::installed(dir) {
        return Ok(tool);
    }
    fs::create_dir_all(dir)?;
    let log_path = dir.join("setup.log");
    let mut log = File::create(&log_path)?;
    let mut step = |message: &str, percent: Option<f32>| {
        if percent.is_none() {
            let _ = writeln!(log, "{message}");
        }
        on_event(FlashEvent::Setup { message: message.into(), percent })
    };
    let failed = |what: &str, e: io::Error| {
        io::Error::new(e.kind(), format!("{what}: {e} (full output in {})", log_path.display()))
    };

    let python = match system_python() {
        Some(python) => python,
        None => portable_python(dir, &mut step).map_err(|e| failed("getting Python", e))?,
    };
    step(&format!("Using {}", python.display()), None);

    step("Creating the virtual environment", None);
    let venv = dir.join("venv");
    remove_dir(&venv)?;
    let mut cmd = command(&python);
    cmd.arg("-m").arg("venv").arg(&venv);
    run_streaming(cmd, |e| {
        if let FlashEvent::Log { line } = e {
            step(&line, None);
        }
    })
    .map_err(|e| failed("creating the virtualenv", e))?;

    step("Installing esptool", None);
    let mut cmd = command(&venv_python(&venv));
    cmd.args(["-m", "pip", "install", "--disable-pip-version-check", "--no-input", "--no-cache-dir", "--retries", "10", ESPTOOL_REQUIREMENT]);
    run_streaming(cmd, |e| {
        if let FlashEvent::Log { line } = e {
            step(&line, None);
        }
    })
    .map_err(|e| failed("installing esptool", e))?;
    File::create(venv.join(READY_MARKER))?;

    Esptool::installed(dir).ok_or_else(|| io::Error::other("esptool was installed but does not run"))
}

/// A usable interpreter already on the machine. Never on macOS: its
/// `/usr/bin/python3` stub pops up the Xcode tools installer.
fn system_python() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        return None;
    }
    // Debian/Ubuntu ship Python without `venv`/`ensurepip` unless python3-venv is installed.
    let check = "import sys, venv, ensurepip; sys.exit(sys.version_info < (3, 10))";
    ["python3", "python"].into_iter().map(PathBuf::from).find(|python| {
        command(python)
            .args(["-c", check])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// The pinned portable CPython in `<dir>/python`, downloaded if missing.
fn portable_python(dir: &Path, step: &mut impl FnMut(&str, Option<f32>)) -> io::Result<PathBuf> {
    let home = dir.join("python");
    let python = if cfg!(windows) { home.join("python.exe") } else { home.join("bin").join("python3") };
    if python.is_file() {
        return Ok(python);
    }

    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let &(_, _, triple, sha256) = PYTHON_BUILDS
        .iter()
        .find(|(o, a, ..)| *o == os && *a == arch)
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, format!("no portable Python for {os}/{arch}")))?;
    let name = format!("cpython-{PYTHON_VERSION}+{PYTHON_RELEASE}-{triple}-install_only_stripped.tar.gz");
    let url = format!(
        "https://github.com/astral-sh/python-build-standalone/releases/download/{PYTHON_RELEASE}/{}",
        name.replace('+', "%2B")
    );

    let archive = dir.join("python.tar.gz.partial");
    let message = format!("Downloading Python {PYTHON_VERSION}");
    download(&url, &archive, sha256, |percent| step(&message, percent)).map_err(|e| with_context("downloading Python", e))?;

    step("Unpacking Python", None);
    // The archive holds a single `python/` folder; unpack beside it, then move it in place.
    let staging = dir.join("python.partial");
    remove_dir(&staging)?;
    tar::Archive::new(flate2::read::GzDecoder::new(File::open(&archive)?)).unpack(&staging)?;
    remove_dir(&home)?;
    fs::rename(staging.join("python"), &home)?;
    remove_dir(&staging)?;
    fs::remove_file(&archive)?;
    Ok(python)
}

/// Stream `url` into `dest`, failing unless its SHA-256 is `sha256`.
fn remove_dir(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn with_context(what: &str, e: io::Error) -> io::Error {
    io::Error::new(e.kind(), format!("{what}: {e}"))
}
