//! Stream this PC's CPU/GPU/fan sensors to the DualEye board.
//!
//! Close `idf.py monitor` first: it owns the same serial port.
//!
//!   dualeye                 # auto-detect the board and stream
//!   dualeye --once          # print one snapshot, no serial
//!   dualeye --sensors       # list every raw sensor the backends see
//!   dualeye --cpu-face rings --gpu-face claude
//!   dualeye --cpu-rotation 180    # a board mounted upside down
//!   dualeye --claude-statusline   # Claude Code status line helper (reads stdin)

use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use clap::Parser;
use dualeye_core::bridge::{self, BridgeConfig, BridgeEvent};
use dualeye_core::claude::statusline;
use dualeye_core::{BoardFirmware, ClaudeUsage, Collector, Face, Faces, Memory, Rotation, Rotations, Snapshot, serial};

#[derive(Parser)]
#[command(name = "dualeye", version, about = "Stream PC sensors to the ESP32-S3 DualEye board")]
struct Args {
    /// Serial port (default: the only attached Espressif USB device)
    #[arg(long, env = "DUALEYE_PORT")]
    port: Option<String>,
    /// Milliseconds between snapshots
    #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(200..))]
    interval_ms: u64,
    /// Seconds to wait after opening the port before the first write
    #[arg(long, default_value_t = 2.0)]
    boot_wait: f64,
    /// Watch face on the left (CPU) screen: classic, rings, plus, bar, claude or clawd
    #[arg(long, default_value = "classic")]
    cpu_face: Face,
    /// Watch face on the right (GPU) screen: classic, rings, plus, bar, claude or clawd
    #[arg(long, default_value = "classic")]
    gpu_face: Face,
    /// Turn the left (CPU) screen clockwise: 0, 90, 180 or 270 degrees
    #[arg(long, default_value = "0")]
    cpu_rotation: Rotation,
    /// Turn the right (GPU) screen clockwise: 0, 90, 180 or 270 degrees
    #[arg(long, default_value = "0")]
    gpu_rotation: Rotation,
    /// Print one snapshot as JSON and exit, without opening the port
    #[arg(long, conflicts_with_all = ["sensors", "list_ports"])]
    once: bool,
    /// List every raw sensor reading and exit
    #[arg(long, conflicts_with = "list_ports")]
    sensors: bool,
    /// List serial ports and exit
    #[arg(long)]
    list_ports: bool,
    /// Act as Claude Code's status line command: keep its status JSON for the
    /// Claude faces and print a short status line
    #[arg(long = "claude-statusline", hide = true)]
    claude_statusline: bool,
    /// Do not print a line per snapshot
    #[arg(long, short)]
    quiet: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    if args.claude_statusline {
        statusline::run(std::io::stdin().lock(), std::io::stdout().lock());
        return ExitCode::SUCCESS;
    }
    if args.list_ports {
        for p in serial::list_ports() {
            let mark = if p.is_board { "  <- DualEye" } else { "" };
            println!("{:<24} {:04x}:{:04x}  {}{mark}", p.name, p.vid, p.pid, p.product.unwrap_or_default());
        }
        return ExitCode::SUCCESS;
    }
    if args.sensors {
        let mut collector = Collector::new();
        collector.readings();
        thread::sleep(Duration::from_millis(500));
        for r in collector.readings() {
            println!("{:<28} {:<28} {:>9.1} {}", r.source, r.label, r.value, r.unit);
        }
        return ExitCode::SUCCESS;
    }
    if args.once {
        let mut collector = Collector::new();
        collector.sample();
        thread::sleep(Duration::from_millis(500));
        let mut snapshot = collector.sample();
        snapshot.claude = ClaudeUsage::new().sample();
        println!("{}", serde_json::to_string(&snapshot).unwrap());
        return ExitCode::SUCCESS;
    }

    let config = BridgeConfig {
        port: args.port,
        interval: Duration::from_millis(args.interval_ms),
        boot_wait: Duration::from_secs_f64(args.boot_wait.max(0.0)),
        faces: Arc::new(Mutex::new(Faces { cpu: args.cpu_face, gpu: args.gpu_face })),
        rotation: Arc::new(Mutex::new(Rotations { cpu: args.cpu_rotation, gpu: args.gpu_rotation })),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let fatal = Arc::new(AtomicBool::new(false));
    let quiet = args.quiet;
    let sink = {
        let stop = stop.clone();
        let fatal = fatal.clone();
        Arc::new(move |event: BridgeEvent| match event {
            BridgeEvent::Waiting { reason } => eprintln!("{reason}, retrying (or pass --port)"),
            BridgeEvent::Connected { port } => println!("streaming to {port}"),
            BridgeEvent::Snapshot { snapshot, sent } if !quiet => {
                println!("{}{}", summary(&snapshot), if sent { "" } else { "  (no temperature, not sent)" })
            }
            BridgeEvent::Snapshot { .. } => {}
            BridgeEvent::BoardLog { line } => eprintln!("board: {line}"),
            BridgeEvent::Firmware { firmware } => match firmware {
                BoardFirmware::Version { version, .. } => println!("board runs DualEye firmware {version}"),
                BoardFirmware::Legacy => println!("board runs DualEye firmware from before 0.2.0: reflash it from the app"),
                BoardFirmware::Missing => eprintln!("board has no firmware: flash it from the app"),
            },
            BridgeEvent::Disconnected { port, reason, permission_denied } => {
                eprintln!("{port}: {reason}");
                if permission_denied {
                    eprintln!("{}", permission_hint(&port));
                    fatal.store(true, Ordering::Relaxed);
                    stop.store(true, Ordering::Relaxed);
                }
            }
        })
    };
    bridge::run(&config, &stop, sink);
    if fatal.load(Ordering::Relaxed) { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn summary(s: &Snapshot) -> String {
    fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
        v.map_or_else(|| "—".into(), |v| v.to_string())
    }
    let fan = |id| opt(s.fan_rpm(id));
    let mem = |m: Option<Memory>| m.map_or_else(|| "—".into(), |m| format!("{:.1}/{:.0}G", gib(m.used_mb), gib(m.total_mb)));
    format!(
        "cpu {}C {}% {}MHz {}W fan {} ram {} | gpu {}C {}% {}MHz {}W fan {} vram {}",
        opt(s.cpu.temp_c),
        opt(s.cpu.load_pct),
        opt(s.cpu.clock_mhz),
        opt(s.cpu.power_w),
        fan("cpu"),
        mem(s.cpu.mem),
        opt(s.gpu.temp_c),
        opt(s.gpu.load_pct),
        opt(s.gpu.clock_mhz),
        opt(s.gpu.power_w),
        fan("gpu"),
        mem(s.gpu.mem),
    )
}

fn gib(mb: u32) -> f64 {
    f64::from(mb) / 1024.0
}

fn permission_hint(port: &str) -> String {
    if cfg!(target_os = "linux") {
        format!(
            "The serial port is not writable. Add this user to dialout, then log out and back in:\n  \
             sudo usermod -aG dialout \"$USER\"\nUntil then: sudo chmod a+rw {port}"
        )
    } else {
        "The serial port is busy or not accessible; close idf.py monitor or other serial tools.".into()
    }
}
