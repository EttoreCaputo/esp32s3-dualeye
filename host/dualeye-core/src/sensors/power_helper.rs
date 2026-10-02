//! Client of `dualeye-power-helper`, the root LaunchDaemon the desktop app
//! installs on macOS: CPU, GPU and ANE power from `powermetrics`, the only
//! exact source on macOS 27 (see `ioreport.rs`). Without the helper the
//! socket is missing and this stays empty.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;

/// Keep in sync with `SOCKET` in dualeye-power-helper.
pub const SOCKET: &str = "/var/run/com.dualeye.monitor.power.sock";

/// The helper reports once a second; older values are stale.
const FRESH: Duration = Duration::from_secs(3);
const RETRY: Duration = Duration::from_secs(5);

/// One powermetrics report, in W.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct HelperSample {
    pub cpu_w: Option<f64>,
    pub gpu_w: Option<f64>,
    pub ane_w: Option<f64>,
    pub package_w: Option<f64>,
}

type Latest = Mutex<Option<(HelperSample, Instant)>>;

pub struct PowerHelper {
    latest: Arc<Latest>,
}

impl PowerHelper {
    /// Connect in the background, and reconnect whenever the helper goes away
    /// or gets installed, until this is dropped.
    pub fn start() -> Self {
        let latest = Arc::new(Latest::default());
        let weak = Arc::downgrade(&latest);
        thread::Builder::new()
            .name("power-helper".into())
            .spawn(move || follow(&weak))
            .expect("spawn the power helper client");
        Self { latest }
    }

    /// The last report, if the helper sent one in the last few seconds.
    pub fn latest(&self) -> Option<HelperSample> {
        let latest = *self.latest.lock().unwrap();
        latest.filter(|(_, at)| at.elapsed() < FRESH).map(|(s, _)| s)
    }
}

fn follow(latest: &Weak<Latest>) {
    while latest.strong_count() > 0 {
        if let Ok(stream) = UnixStream::connect(SOCKET) {
            // Wake up now and then to notice a dropped `PowerHelper`.
            let _ = stream.set_read_timeout(Some(RETRY));
            for line in BufReader::new(stream).lines() {
                let (Ok(line), Some(latest)) = (line, latest.upgrade()) else { break };
                if let Ok(sample) = serde_json::from_str::<HelperSample>(&line) {
                    *latest.lock().unwrap() = Some((sample, Instant::now()));
                }
            }
        }
        thread::sleep(RETRY);
    }
}
