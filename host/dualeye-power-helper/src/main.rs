//! Root LaunchDaemon that the DualEye app registers with `SMAppService`
//! (`Contents/Library/LaunchDaemons/com.dualeye.monitor.power.plist`).
//!
//! macOS 27 lets only Apple's `powermetrics`, run as root, read the CPU's
//! energy counters. This runs it while at least one client is connected and
//! sends each sample as a JSON line on a Unix socket:
//!
//! ```text
//! {"cpu_w":3.1,"gpu_w":0.2,"ane_w":0.0,"package_w":3.3}
//! ```
//!
//! Only code signed by the helper's own team may connect (any code when the
//! helper has no team, as in development builds). Nothing is read from clients.

#[cfg(target_os = "macos")]
mod macos;

use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(target_os = "macos")]
    return macos::run();
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("dualeye-power-helper only runs on macOS");
        ExitCode::FAILURE
    }
}
