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
//!   dualeye tools                 # list the board's tools
//!   dualeye call set_face --screen left --face rings
//!   dualeye call show_text '{"text":"Ciao"}'

use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use clap::{Parser, Subcommand};
use dualeye_core::bridge::{self, BridgeConfig, BridgeEvent};
use dualeye_core::claude::statusline;
use dualeye_core::{BoardFirmware, ClaudeUsage, Collector, Face, Faces, Link, LinkEvent, Memory, Rotation, Rotations, Snapshot, serial};
use serde_json::{Map, Value};

#[derive(Parser)]
#[command(name = "dualeye", version, about = "Stream PC sensors to the ESP32-S3 DualEye board", args_conflicts_with_subcommands = true)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
    /// Serial port (default: the only attached Espressif USB device)
    #[arg(long, env = "DUALEYE_PORT")]
    port: Option<String>,
    /// Milliseconds between snapshots
    #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(200..))]
    interval_ms: u64,
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

/// Talk to the board's tools directly. These open the port themselves, so
/// quit the app (or a running `dualeye`) first.
#[derive(Subcommand)]
enum Command {
    /// List the tools the board offers
    Tools {
        #[arg(long, env = "DUALEYE_PORT")]
        port: Option<String>,
        /// Print the full `tools/list` result as JSON
        #[arg(long)]
        json: bool,
    },
    /// Call a board tool: `call set_face --screen left --face rings`, or
    /// `call set_face '{"screen":"left","face":"rings"}'`
    Call {
        #[arg(long, env = "DUALEYE_PORT")]
        port: Option<String>,
        /// Print the full result as JSON
        #[arg(long)]
        json: bool,
        tool: String,
        /// Arguments: one JSON object, or `--name value` pairs (values that
        /// parse as JSON, like numbers, are sent as such)
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
}

fn main() -> ExitCode {
    let args = Args::parse();
    match args.command {
        Some(Command::Tools { port, json }) => return tools(port, json),
        Some(Command::Call { port, json, tool, args }) => return call(port, json, &tool, &args),
        None => {}
    }
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
                BoardFirmware::Version { version, protocol: 1, .. } => {
                    println!("board runs DualEye firmware {version}, which this version can't drive: reflash it from the app")
                }
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

const TOOL_TIMEOUT: Duration = Duration::from_secs(3);

/// Open the board and do the handshake; board log lines go to stderr.
fn connect(port: Option<String>) -> Result<Link, String> {
    let port = port.or_else(serial::detect_board).ok_or("board not found on USB (pass --port)")?;
    let link = Link::open(&port, |event| {
        if let LinkEvent::Log(line) | LinkEvent::Text(line) = event {
            eprintln!("board: {line}");
        }
    })
    .map_err(|e| format!("{port}: {e}"))?;
    link.handshake(Duration::from_secs(5)).map_err(|e| format!("{port}: handshake failed: {e} (firmware 0.4 or later needed)"))?;
    Ok(link)
}

fn tools(port: Option<String>, json: bool) -> ExitCode {
    let result = connect(port).and_then(|link| link.list_tools(TOOL_TIMEOUT).map_err(|e| e.to_string()));
    match result {
        Ok(tools) if json => println!("{}", serde_json::to_string_pretty(&serde_json::json!({"tools": tools})).unwrap()),
        Ok(tools) => {
            for tool in tools {
                println!("{}  {}", tool.name, tool.description);
                let props = tool.input_schema.get("properties").and_then(Value::as_object);
                let required: Vec<&str> = tool.input_schema.get("required").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
                for (name, schema) in props.into_iter().flatten() {
                    println!("    --{name} {}{}", arg_hint(schema), if required.contains(&name.as_str()) { "  (required)" } else { "" });
                }
            }
        }
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

/// `<classic|rings|…>` or `<integer>` from a property's JSON schema.
fn arg_hint(schema: &Value) -> String {
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        let names: Vec<String> = values.iter().map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string)).collect();
        return format!("<{}>", names.join("|"));
    }
    format!("<{}>", schema.get("type").and_then(Value::as_str).unwrap_or("value"))
}

fn call(port: Option<String>, json: bool, tool: &str, raw: &[String]) -> ExitCode {
    let arguments = match parse_tool_args(raw) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let result = connect(port).and_then(|link| link.call_tool(tool, arguments, TOOL_TIMEOUT).map_err(|e| e.to_string()));
    match result {
        Ok(result) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            } else if let Some(structured) = &result.structured_content {
                println!("{}", serde_json::to_string_pretty(structured).unwrap());
            } else {
                println!("{}", result.text());
            }
            if result.is_error { ExitCode::FAILURE } else { ExitCode::SUCCESS }
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// One JSON object, or `--name value` pairs.
fn parse_tool_args(raw: &[String]) -> Result<Value, String> {
    if let [only] = raw
        && only.trim_start().starts_with('{')
    {
        return serde_json::from_str(only).map_err(|e| format!("arguments are not valid JSON: {e}"));
    }
    let mut args = Map::new();
    let mut it = raw.iter();
    while let Some(key) = it.next() {
        let name = key.strip_prefix("--").ok_or_else(|| format!("expected --name before `{key}`"))?;
        let (name, value) = match name.split_once('=') {
            Some((n, v)) => (n, v.to_string()),
            None => (name, it.next().ok_or_else(|| format!("--{name} needs a value"))?.clone()),
        };
        let value = serde_json::from_str(&value).unwrap_or(Value::String(value));
        args.insert(name.replace('-', "_"), value);
    }
    Ok(Value::Object(args))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(list: &[&str]) -> Result<Value, String> {
        parse_tool_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn tool_args_from_flags_or_json() {
        assert_eq!(args(&["--screen", "left", "--face", "rings"]), Ok(json!({"screen": "left", "face": "rings"})));
        assert_eq!(args(&["--percent=40"]), Ok(json!({"percent": 40})));
        assert_eq!(args(&["--text", "Ciao Duo"]), Ok(json!({"text": "Ciao Duo"})));
        assert_eq!(args(&[r#"{"degrees":180}"#]), Ok(json!({"degrees": 180})));
        assert_eq!(args(&[]), Ok(json!({})));
        assert!(args(&["--face"]).is_err());
        assert!(args(&["rings"]).is_err());
    }
}
