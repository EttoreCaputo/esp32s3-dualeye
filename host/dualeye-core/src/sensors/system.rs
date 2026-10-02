//! Network, disk and battery, for the `net`, `disk` and `battery` faces:
//! sysinfo for the first two on every platform, `starship-battery` for the
//! battery (IOKit on macOS, `/sys/class/power_supply` on Linux, the power
//! API on Windows).

use std::path::Path;
use std::time::{Duration, Instant};

use starship_battery::units::{ratio::percent, time::minute};
use starship_battery::{Manager, State};
use sysinfo::{DiskRefreshKind, Disks, Networks};

use crate::snapshot::{Battery, Disk, Net, round1};

/// The battery changes slowly; asking every few seconds is plenty.
const BATTERY_EVERY: Duration = Duration::from_secs(5);
const GB: f64 = 1e9;

pub(super) struct SystemSensors {
    networks: Networks,
    disks: Disks,
    /// When `networks` and `disks` were last refreshed: their counters are since then.
    last: Instant,
    battery: Option<Manager>,
    battery_cache: Option<(Instant, Option<Battery>)>,
}

impl SystemSensors {
    pub fn new() -> Self {
        Self {
            networks: Networks::new_with_refreshed_list(),
            disks: Disks::new_with_refreshed_list_specifics(disk_refresh()),
            last: Instant::now(),
            battery: Manager::new().ok(),
            battery_cache: None,
        }
    }

    /// Throughput since the previous call, disk space and the battery.
    pub fn sample(&mut self) -> (Option<Net>, Option<Disk>, Option<Battery>) {
        let secs = self.last.elapsed().as_secs_f64().max(0.05);
        self.last = Instant::now();
        self.networks.refresh(true);
        self.disks.refresh_specifics(true, disk_refresh());

        let (rx, tx) = self
            .networks
            .iter()
            .filter(|(name, _)| !is_loopback(name))
            .fold((0u64, 0u64), |(rx, tx), (_, n)| (rx + n.received(), tx + n.transmitted()));
        let net = Some(Net { rx_bps: (rx as f64 / secs) as u64, tx_bps: (tx as f64 / secs) as u64 });

        let disk = system_disk(&self.disks).map(|d| {
            let usage = d.usage();
            // Platforms without I/O counters report zeros all along.
            let io = usage.total_read_bytes > 0 || usage.total_written_bytes > 0;
            let total = d.total_space();
            Disk {
                used_gb: round1(total.saturating_sub(d.available_space()) as f64 / GB),
                total_gb: round1(total as f64 / GB),
                read_bps: io.then(|| (usage.read_bytes as f64 / secs) as u64),
                write_bps: io.then(|| (usage.written_bytes as f64 / secs) as u64),
            }
        });

        (net, disk, self.battery())
    }

    fn battery(&mut self) -> Option<Battery> {
        if let Some((at, cached)) = self.battery_cache
            && at.elapsed() < BATTERY_EVERY
        {
            return cached;
        }
        let read = self.battery.as_ref().and_then(read_battery);
        self.battery_cache = Some((Instant::now(), read));
        read
    }
}

fn disk_refresh() -> DiskRefreshKind {
    DiskRefreshKind::nothing().with_storage().with_io_usage()
}

fn is_loopback(name: &str) -> bool {
    name == "lo" || name.starts_with("lo0") || name.to_ascii_lowercase().contains("loopback")
}

/// The disk the system runs from: `/` (or the Mac's data volume, which holds
/// what the person stores), `C:\` on Windows; otherwise the largest fixed one.
fn system_disk(disks: &Disks) -> Option<&sysinfo::Disk> {
    let wanted: &[&str] = if cfg!(windows) {
        &["C:\\"]
    } else if cfg!(target_os = "macos") {
        &["/System/Volumes/Data", "/"]
    } else {
        &["/"]
    };
    let list = disks.list();
    wanted
        .iter()
        .find_map(|w| list.iter().find(|d| d.mount_point() == Path::new(w)))
        .or_else(|| list.iter().filter(|d| !d.is_removable()).max_by_key(|d| d.total_space()))
        .filter(|d| d.total_space() > 0)
}

/// The first battery; `None` on a desktop.
fn read_battery(manager: &Manager) -> Option<Battery> {
    let bat = manager.batteries().ok()?.flatten().next()?;
    let state = bat.state();
    let charging = state == State::Charging;
    let plugged = matches!(state, State::Charging | State::Full | State::Paused);
    let time = if charging { bat.time_to_full() } else if state == State::Discharging { bat.time_to_empty() } else { None };
    Some(Battery {
        pct: round1(f64::from(bat.state_of_charge().get::<percent>())),
        charging,
        plugged,
        mins: time.map(|t| t.get::<minute>().round() as u32).filter(|&m| m > 0),
    })
}
