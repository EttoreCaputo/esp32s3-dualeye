//! Stream this PC's CPU/GPU/fan sensors to the DualEye board.
//!
//! Close `idf.py monitor` first: it owns the same serial port.
//!
//!   dualeye                 # auto-detect the board and stream; faces and rotation stay as the board has them
//!   dualeye --once          # print one snapshot, no serial
//!   dualeye --sensors       # list every raw sensor the backends see
//!   dualeye --cpu-face rings --gpu-face claude
//!   dualeye --cpu-rotation 180    # a board mounted upside down
//!   dualeye --voice-dump          # keep what the board hears after its wake word as WAV files
//!   dualeye --claude-statusline   # Claude Code status line helper (reads stdin)
//!   dualeye tools                 # list the board's tools
//!   dualeye call set_face --screen left --face rings
//!   dualeye call show_text '{"text":"Ciao"}'
//!   dualeye mcp                   # MCP server on stdio, for Claude Code / Claude Desktop
//!   dualeye models download small # a Whisper model, for `dualeye --stt`
//!   dualeye piper install         # Piper, for `dualeye --tts` (needs Python 3)
//!   dualeye models download it_IT-paola-medium   # a voice for it
//!   dualeye kokoro install        # Kokoro, for its voices (needs Python 3.10 to 3.13)
//!   dualeye models download kokoro-if_sara       # a Kokoro voice
//!   dualeye --stt --tts           # voice commands with spoken replies
//!   dualeye models download qwen3-4b-2507        # a language model, for `dualeye --llm`
//!   dualeye --stt --tts --llm     # ...understood by a local language model (llama-server)
//!   dualeye ask "metti rings a sinistra"         # type a command to the language model
//!   dualeye eval                  # run the voice commands' eval set against the language model
//!   dualeye say "Ciao!"           # speak through a running bridge's speaker
//!   dualeye timer 10m pasta       # a timer on the board (also: timer, timer cancel, timer remind 17:30 call Marco, timer pomodoro)

use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use clap::{Parser, Subcommand};
use dualeye_core::bridge::{self, BridgeConfig, BridgeEvent};
use dualeye_core::claude::{hooks, statusline};
use dualeye_core::hardware::{self, Hardware};
use dualeye_core::models::{self, Kind, Model};
use dualeye_core::tts::{self, Tts, TtsConfig};
use dualeye_core::stt::{self, Stt, SttConfig, SttLanguage};
use dualeye_core::eval::{self, EvalSet, Responder};
use dualeye_core::llm::{self, Llm, LlmConfig};
use dualeye_core::timers::{self, Timers};
use dualeye_core::voice::{self, VoiceConfig};
use dualeye_core::{
    Agent, Board, BoardFirmware, ClaudeUsage, Collector, Face, Faces, Hub, Memory, Rotation, Rotations, Snapshot, Source, Sources, Tool, Toolbox, intents, mcp, media,
    serial,
};
use serde_json::{Map, Value, json};

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
    /// Watch face on the left screen: classic, rings, plus, bar, claude, clawd,
    /// net, disk, battery, image or timer. Without any face, source or rotation flag
    /// the board keeps its own; with one, the others fall back to classic,
    /// the usual source and 0
    #[arg(long)]
    cpu_face: Option<Face>,
    /// Watch face on the right screen, from the same list
    #[arg(long)]
    gpu_face: Option<Face>,
    /// Whose metrics classic, rings, plus and bar show on the left screen: cpu (default) or gpu
    #[arg(long)]
    cpu_source: Option<Source>,
    /// Likewise on the right screen: gpu (default) or cpu
    #[arg(long)]
    gpu_source: Option<Source>,
    /// Turn the left (CPU) screen clockwise: 0, 90, 180 or 270 degrees
    #[arg(long)]
    cpu_rotation: Option<Rotation>,
    /// Turn the right (GPU) screen clockwise: 0, 90, 180 or 270 degrees
    #[arg(long)]
    gpu_rotation: Option<Rotation>,
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
    /// Act as a Claude Code hook: pass its JSON to the running bridge for
    /// the Claude alerts; prints nothing
    #[arg(long = "claude-hook", hide = true)]
    claude_hook: bool,
    /// Do not print a line per snapshot
    #[arg(long, short)]
    quiet: bool,
    /// Save each utterance the board streams after its wake word as a WAV
    /// file, in DIR or in `voice/` in DualEye's data folder
    #[arg(long, value_name = "DIR", num_args = 0..=1, default_missing_value = "")]
    voice_dump: Option<std::path::PathBuf>,
    /// Transcribe what the board hears with whisper.cpp (`whisper-server` on
    /// the PATH): a model from `dualeye models` (default: small), or a ggml
    /// model file
    #[arg(long, value_name = "MODEL|FILE", num_args = 0..=1, default_missing_value = "small")]
    stt: Option<String>,
    /// Language to transcribe in: auto (Italian or English), it or en
    #[arg(long, default_value = "auto")]
    stt_language: SttLanguage,
    /// Answer voice commands out loud through the board's speaker, with
    /// Piper (`dualeye piper install`) or Kokoro (`dualeye kokoro install`)
    /// and the downloaded voices
    #[arg(long)]
    tts: bool,
    /// A voice from `dualeye models` for its language, instead of the
    /// default one (repeat for Italian and English)
    #[arg(long, value_name = "VOICE", requires = "tts")]
    tts_voice: Vec<String>,
    /// After a spoken answer, don't listen for a few seconds more without
    /// the wake word
    #[arg(long, requires = "tts")]
    no_follow_up: bool,
    #[command(flatten)]
    llm: LlmArgs,
}

#[derive(clap::Args, Clone)]
struct LlmArgs {
    /// Understand voice commands with a local language model (llama.cpp's
    /// `llama-server` on the PATH): a model from `dualeye models`, or a GGUF
    /// file. Without it, a few fixed phrases are understood
    #[arg(long, value_name = "MODEL|FILE", num_args = 0..=1, default_missing_value = models::DEFAULT_LLM)]
    llm: Option<String>,
    /// Layers of the language model on the GPU; 0 runs it on the CPU
    /// (default: as many as fit)
    #[arg(long, value_name = "N")]
    llm_gpu_layers: Option<u32>,
}

/// Talk to the board's tools. While the app or a streaming `dualeye` runs,
/// these go through it; otherwise they open the port for the call.
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
    /// Run an MCP server on stdin/stdout with the board's tools plus
    /// get_metrics and get_claude_usage. Add it to Claude Code with
    /// `claude mcp add dualeye -- dualeye mcp`
    Mcp {
        /// Used only when neither the app nor a streaming `dualeye` is running
        #[arg(long, env = "DUALEYE_PORT")]
        port: Option<String>,
    },
    /// List the Whisper models for `--stt` and the voices for `--tts`, or
    /// download or remove one
    Models {
        #[command(subcommand)]
        action: Option<ModelsAction>,
    },
    /// Show whether Piper (text-to-speech, for `--tts`) is installed, or install it
    Piper {
        #[command(subcommand)]
        action: Option<EngineAction>,
    },
    /// Show whether Kokoro (text-to-speech for the `kokoro-` voices) is
    /// installed, or install it
    Kokoro {
        #[command(subcommand)]
        action: Option<EngineAction>,
    },
    /// Type a command to the language model, as if said to the board: it
    /// runs the board's tools and prints its answer
    Ask {
        text: String,
        /// it or en (default: what the text looks like)
        #[arg(long)]
        language: Option<String>,
        #[arg(long, env = "DUALEYE_PORT")]
        port: Option<String>,
        #[command(flatten)]
        llm: LlmArgs,
    },
    /// Run the voice commands' eval set (about 50, Italian and English) on a
    /// simulated board: accuracy and latency of a language model, or of the
    /// fixed phrases with --rules
    Eval {
        #[command(flatten)]
        llm: LlmArgs,
        /// The fixed phrases (M5) instead of a language model
        #[arg(long, conflicts_with = "llm")]
        rules: bool,
        /// commands (the main set), holdout (other phrasings), or a JSON file
        #[arg(long, default_value = "commands")]
        set: String,
        /// Only the cases in this language: it or en
        #[arg(long)]
        language: Option<String>,
        /// Print every case, not only the failures
        #[arg(long, short)]
        verbose: bool,
        /// Print the results as JSON
        #[arg(long)]
        json: bool,
    },
    /// Put a picture or an animated GIF on a screen, for its image face:
    /// `dualeye image cat.gif --screen left`, then `--cpu-face image`.
    /// `--clear` takes it off
    Image {
        /// PNG, JPEG, GIF, WebP or BMP; cropped to the middle square
        #[arg(required_unless_present = "clear")]
        file: Option<std::path::PathBuf>,
        /// left or right
        #[arg(long, default_value = "left", value_parser = ["left", "right"])]
        screen: String,
        #[arg(long)]
        clear: bool,
        #[arg(long, env = "DUALEYE_PORT")]
        port: Option<String>,
    },
    /// Timers, reminders and a pomodoro, shown and rung by the board while
    /// the app or `dualeye` runs: `timer` lists them, `timer 10m [label]`
    /// (or 90s, 1h30m, 25 for minutes) starts one, `timer cancel|pause|resume
    /// [label|all]`, `timer remind 17:30|20m <text>`, `timer pomodoro [stop |
    /// WORK BREAK ROUNDS]`
    Timer {
        /// it or en: what the board says when it's up (default: the system's)
        #[arg(long)]
        language: Option<String>,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Speak through the board's speaker. Needs the app with spoken replies
    /// on, or `dualeye --tts`, running
    Say {
        text: String,
        /// it or en (default: what the text looks like)
        #[arg(long)]
        language: Option<String>,
    },
}

#[derive(Subcommand)]
enum EngineAction {
    /// Create a virtualenv in DualEye's data folder and pip install the engine into it
    Install {
        /// The Python 3 to create it with (default: python3 on the PATH, or
        /// the newest python3.N the engine supports)
        #[arg(long)]
        python: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum ModelsAction {
    /// Download a model and check its SHA-256
    Download { id: String },
    /// Delete a downloaded model
    Remove { id: String },
}

fn main() -> ExitCode {
    let args = Args::parse();
    match args.command {
        Some(Command::Tools { port, json }) => return tools(port, json),
        Some(Command::Call { port, json, tool, args }) => return call(port, json, &tool, &args),
        Some(Command::Models { action }) => return models_command(action),
        Some(Command::Piper { action }) => return engine_command(tts::Engine::Piper, action),
        Some(Command::Kokoro { action }) => return engine_command(tts::Engine::Kokoro, action),
        Some(Command::Ask { text, language, port, llm }) => return ask(&text, language.as_deref(), port, &llm),
        Some(Command::Eval { llm, rules, set, language, verbose, json }) => return eval(&llm, rules, &set, language.as_deref(), verbose, json),
        Some(Command::Image { file, screen, clear, port }) => return image(port, file.as_deref(), &screen, clear),
        Some(Command::Say { text, language }) => {
            return match board(None).say(&text, language.as_deref()) {
                Ok(r) => {
                    println!("{}", serde_json::to_string(&r).unwrap_or_default());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            };
        }
        Some(Command::Timer { language, args }) => return timer(language.as_deref(), &args),
        Some(Command::Mcp { port }) => {
            return match mcp::serve_stdio(port) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("mcp: {e}");
                    ExitCode::FAILURE
                }
            };
        }
        None => {}
    }
    if args.claude_statusline {
        statusline::run(std::io::stdin().lock(), std::io::stdout().lock());
        return ExitCode::SUCCESS;
    }
    if args.claude_hook {
        hooks::run(std::io::stdin().lock());
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

    let stt = match args.stt.as_deref().map(|m| stt_config(m, args.stt_language)) {
        None => None,
        Some(Ok(config)) => Some(Arc::new(Stt::new(config))),
        Some(Err(why)) => {
            eprintln!("{why}");
            return ExitCode::FAILURE;
        }
    };
    let tts = match args.tts.then(|| tts_config(&args.tts_voice)) {
        None => None,
        Some(Ok(config)) => Some(Arc::new(Tts::new(config))),
        Some(Err(why)) => {
            eprintln!("{why}");
            return ExitCode::FAILURE;
        }
    };
    let agent = match args.llm.llm.is_some().then(|| agent(&args.llm)) {
        None => None,
        Some(Ok(agent)) => Some(agent),
        Some(Err(why)) => {
            eprintln!("{why}");
            return ExitCode::FAILURE;
        }
    };
    // Lets `dualeye mcp` and `dualeye call` use the board while this streams.
    let hub = Hub::start().inspect_err(|e| eprintln!("not sharing the board with other processes: {e}")).ok();
    let config = BridgeConfig {
        port: args.port,
        interval: Duration::from_millis(args.interval_ms),
        faces: Arc::new(Mutex::new(Faces {
            cpu: args.cpu_face.unwrap_or_default(),
            gpu: args.gpu_face.unwrap_or_default(),
            src: Sources { cpu: args.cpu_source.unwrap_or(Source::Cpu), gpu: args.gpu_source.unwrap_or(Source::Gpu) },
        })),
        rotation: Arc::new(Mutex::new(Rotations {
            cpu: args.cpu_rotation.unwrap_or_default(),
            gpu: args.gpu_rotation.unwrap_or_default(),
        })),
        hub,
        adopt_board_settings: args.cpu_face.is_none()
            && args.gpu_face.is_none()
            && args.cpu_source.is_none()
            && args.gpu_source.is_none()
            && args.cpu_rotation.is_none()
            && args.gpu_rotation.is_none(),
        voice: Arc::new(Mutex::new(VoiceConfig {
            dump_dir: args.voice_dump.map(|d| if d.as_os_str().is_empty() { voice::default_dump_dir().unwrap_or(d) } else { d }),
            stt,
            tts,
            agent,
            follow_up: !args.no_follow_up,
        })),
        // The alerts need the hooks in Claude Code's settings (the app's Display tab adds them).
        claude_alerts: Arc::default(),
        timers: Arc::new(Timers::open(timers::default_file())),
    };
    let stop = Arc::new(AtomicBool::new(false));
    // Ctrl-C ends the loop, so the whisper-server sidecar is stopped too.
    {
        let stop = stop.clone();
        let _ = ctrlc::set_handler(move || stop.store(true, Ordering::Relaxed));
    }
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
            BridgeEvent::Wake { word, volume_db } => match volume_db {
                Some(db) => println!("wake word \"{word}\" ({db:.0} dBFS)"),
                None => println!("wake word \"{word}\""),
            },
            BridgeEvent::VoiceState { state } if !quiet => println!("voice: {state}"),
            BridgeEvent::VoiceState { .. } => {}
            BridgeEvent::Transcript { id, transcript: Some(t) } => {
                println!("utterance {id} [{}, {:.1} s]: {}", t.language, t.elapsed_ms as f64 / 1000.0, t.text)
            }
            BridgeEvent::Transcript { id, transcript: None } => println!("utterance {id}: no words"),
            BridgeEvent::VoiceError { message } => eprintln!("{message}"),
            BridgeEvent::Reply { id, text, actions, by, elapsed_ms, .. } => {
                for a in actions {
                    println!("utterance {id}: {a}");
                }
                println!("utterance {id}: reply ({by}, {elapsed_ms} ms): {text}")
            }
            BridgeEvent::Spoken { id, spoken } => println!(
                "utterance {id}: spoken, first audio after {} ms, {:.1} s played ({}{})",
                spoken.first_audio_ms,
                spoken.played_ms as f64 / 1000.0,
                spoken.reason,
                if spoken.underruns + spoken.lost > 0 { format!(", {} underruns, {} frames lost", spoken.underruns, spoken.lost) } else { String::new() }
            ),
            BridgeEvent::Listening { id, trigger } => println!("utterance {id}: listening ({trigger})"),
            BridgeEvent::Utterance { utterance: u, duration_ms, peak_db, wav } => println!(
                "utterance {}: {:.1} s, {}{}{}{}",
                u.id,
                duration_ms as f64 / 1000.0,
                u.reason.replace('_', " "),
                if u.speech || u.reason == "no_speech" { "" } else { ", no speech" },
                peak_db.map(|db| format!(", peak {db:.0} dBFS")).unwrap_or_default(),
                match (u.lost_frames, wav) {
                    (0, None) => String::new(),
                    (0, Some(w)) => format!(" -> {w}"),
                    (n, None) => format!(", {n} frames lost"),
                    (n, Some(w)) => format!(", {n} frames lost -> {w}"),
                }
            ),
            BridgeEvent::TimerFired { text, error: None, .. } => println!("timer: {text}"),
            BridgeEvent::TimerFired { text, error: Some(e), .. } => println!("timer: {text} (not said: {e})"),
            BridgeEvent::ClaudeAlert { text, error: None, .. } => println!("claude: {text}"),
            BridgeEvent::ClaudeAlert { text, error: Some(e), .. } => eprintln!("claude: {text} (not given: {e})"),
            BridgeEvent::Settings { faces, rotation } => println!(
                "board settings: faces {}/{}, rotation {}/{}",
                faces.cpu.name(),
                faces.gpu.name(),
                rotation.cpu.degrees(),
                rotation.gpu.degrees()
            ),
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
    // A reply still being spoken holds the sidecars too: stop them anyway.
    let voice = config.voice.lock().unwrap();
    voice.stt.iter().for_each(|s| s.shutdown());
    voice.tts.iter().for_each(|t| t.shutdown());
    voice.agent.iter().for_each(|a| a.llm().shutdown());
    if fatal.load(Ordering::Relaxed) { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// `--llm qwen3-4b-2507` (a model from `dualeye models`) or `--llm path/to/model.gguf`.
fn llm_config(args: &LlmArgs) -> Result<LlmConfig, String> {
    let server = llm::find_server().ok_or("llama-server not found: install llama.cpp (brew install llama.cpp)")?;
    let model = args.llm.as_deref().unwrap_or(models::DEFAULT_LLM);
    let path = match Model::by_id(model) {
        Some(m) if m.kind == Kind::Llm && m.is_installed() => m.path().ok_or("no data folder for the models")?,
        Some(m) if m.kind == Kind::Llm => return Err(format!("the {model} model isn't downloaded: dualeye models download {model}")),
        _ if std::path::Path::new(model).is_file() => model.into(),
        _ => {
            let ids: Vec<&str> = Model::of_kind(Kind::Llm).map(|m| m.id).collect();
            return Err(format!("{model}: not a language model ({}) or a GGUF file", ids.join(", ")));
        }
    };
    Ok(LlmConfig { server, model: path, gpu_layers: args.llm_gpu_layers })
}

fn agent(args: &LlmArgs) -> Result<Arc<Agent>, String> {
    Ok(Arc::new(Agent::new(Arc::new(Llm::new(llm_config(args)?)))))
}

/// The board's tools for `dualeye ask`, through a running bridge or the port.
struct BoardTools {
    board: Board,
    language: String,
}

impl Toolbox for BoardTools {
    fn tools(&self) -> Vec<Tool> {
        voice::voice_tools(self.board.list_tools().unwrap_or_default())
    }

    fn call(&self, name: &str, arguments: &Value) -> Result<String, String> {
        if name == "get_metrics" {
            let snapshot = match self.board.bridge_snapshot() {
                Some((s, _)) => s,
                None => {
                    let mut collector = Collector::new();
                    collector.sample();
                    thread::sleep(Duration::from_millis(500));
                    collector.sample()
                }
            };
            return Ok(snapshot.metrics_json().to_string());
        }
        if Timers::tools().iter().any(|t| t.name == name) {
            return self.board.timer_tool(name, arguments, &self.language);
        }
        let result = self.board.call_tool(name, arguments.clone()).map_err(|e| e.to_string())?;
        match (result.is_error, name, &result.structured_content) {
            (true, ..) => Err(result.text()),
            (false, "get_state", Some(state)) => Ok(voice::trim_state(state).to_string()),
            _ => Ok(result.text()),
        }
    }
}

/// "10m", "90s", "1h30m", "25" (minutes).
fn parse_span(s: &str) -> Option<u64> {
    if let Ok(min) = s.parse::<f64>() {
        return (min > 0.0).then(|| (min * 60.0).round() as u64);
    }
    let (mut total, mut num) = (0u64, String::new());
    for c in s.chars() {
        match c {
            '0'..='9' => num.push(c),
            'h' | 'm' | 's' => {
                let n: u64 = num.parse().ok()?;
                num.clear();
                total += n * match c {
                    'h' => 3600,
                    'm' => 60,
                    _ => 1,
                };
            }
            _ => return None,
        }
    }
    (num.is_empty() && total > 0).then_some(total)
}

fn timer(language: Option<&str>, args: &[String]) -> ExitCode {
    let language = language.unwrap_or(dualeye_core::claude::alerts::system_language());
    let rest = |from: usize| args.get(from..).unwrap_or_default().join(" ");
    let first = args.first().map(String::as_str);
    let (tool, arguments) = match first {
        None | Some("list" | "ls") => ("get_timers", json!({})),
        Some(action @ ("cancel" | "pause" | "resume" | "stop")) => {
            let action = if action == "stop" { "cancel" } else { action };
            let which = rest(1);
            ("control_timer", if which.is_empty() { json!({"action": action}) } else { json!({"action": action, "which": which}) })
        }
        Some("remind") => {
            let when = args.get(1).map(String::as_str).unwrap_or_default();
            let text = rest(2);
            match (timers::parse_clock(when).filter(|_| when.contains([':', '.'])), parse_span(when)) {
                (Some(at), _) => ("set_reminder", json!({"text": text, "at": timers::clock_text(at)})),
                (None, Some(secs)) => ("set_reminder", json!({"text": text, "in_minutes": secs as f64 / 60.0})),
                (None, None) => {
                    eprintln!("usage: dualeye timer remind 17:30|20m <text>");
                    return ExitCode::FAILURE;
                }
            }
        }
        Some("pomodoro") => match args.get(1).map(String::as_str) {
            Some("stop") => ("pomodoro", json!({"action": "stop"})),
            _ => {
                let n = |i: usize, default: u64| args.get(i).and_then(|a| a.parse().ok()).unwrap_or(default);
                ("pomodoro", json!({"action": "start", "work_minutes": n(1, 25), "break_minutes": n(2, 5), "rounds": n(3, 4)}))
            }
        },
        Some(span) => match parse_span(span) {
            Some(secs) => {
                let label = rest(1);
                ("set_timer", if label.is_empty() { json!({"seconds": secs}) } else { json!({"seconds": secs, "label": label}) })
            }
            None => {
                eprintln!("unknown timer command `{span}`: see dualeye timer --help");
                return ExitCode::FAILURE;
            }
        },
    };
    let board = board(None);
    match board.timer_tool(tool, &arguments, language) {
        Ok(text) if tool == "get_timers" => {
            let list: Value = serde_json::from_str(&text).unwrap_or_default();
            let timers = list["timers"].as_array().cloned().unwrap_or_default();
            if timers.is_empty() {
                println!("no timers");
            }
            for t in timers {
                let left = t["left_s"].as_u64().unwrap_or(0);
                let name = t["label"].as_str().map(|l| format!(" {l}")).unwrap_or_default();
                let ends = t["ends_at"].as_str().map(|e| format!(", ends {e}")).unwrap_or_default();
                println!("{}{name}: {}:{:02}:{:02} left of {} s ({}{ends})", t["kind"].as_str().unwrap_or(""), left / 3600, left % 3600 / 60, left % 60, t["total_s"], t["state"].as_str().unwrap_or(""));
            }
            if board.last_route() != Some(dualeye_core::Route::Hub) {
                eprintln!("(the app isn't running: these ring once it, or `dualeye`, runs)");
            }
            ExitCode::SUCCESS
        }
        Ok(text) => {
            println!("{text}");
            if board.last_route() != Some(dualeye_core::Route::Hub) {
                eprintln!("(the app isn't running: it rings once the app, or `dualeye`, runs)");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn ask(text: &str, language: Option<&str>, port: Option<String>, args: &LlmArgs) -> ExitCode {
    let agent = match agent(args) {
        Ok(a) => a,
        Err(why) => {
            eprintln!("{why}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("loading {}", agent.llm().config().model.display());
    if let Err(e) = agent.llm().warm_up() {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }
    let language = language.unwrap_or_else(|| intents::guess_language(text));
    let result = agent.respond(text, language, &BoardTools { board: board(port), language: language.to_string() });
    agent.llm().shutdown();
    match result {
        Ok(turn) => {
            for a in &turn.actions {
                println!("{}({}) -> {}{}", a.tool, a.arguments, if a.ok { "" } else { "failed: " }, a.result);
            }
            println!("{}", turn.reply);
            eprintln!(
                "{} requests, {} ms{}",
                turn.rounds,
                turn.elapsed_ms,
                turn.first_action_ms.map(|ms| format!(", first action after {ms} ms")).unwrap_or_default()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn eval(args: &LlmArgs, rules: bool, set: &str, language: Option<&str>, verbose: bool, json: bool) -> ExitCode {
    let set = match EvalSet::named(set) {
        Ok(s) => s,
        Err(why) => {
            eprintln!("{why}");
            return ExitCode::FAILURE;
        }
    };
    let agent = if rules {
        None
    } else {
        match agent(args) {
            Ok(a) => Some(a),
            Err(why) => {
                eprintln!("{why}");
                return ExitCode::FAILURE;
            }
        }
    };
    if let Some(agent) = &agent {
        eprintln!("loading {}", agent.llm().config().model.display());
        if let Err(e) = agent.llm().warm_up() {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }
    let responder = match &agent {
        Some(a) => Responder::Llm(a),
        None => Responder::Rules,
    };
    if let Err(e) = eval::prime(&set, &responder) {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }
    let cases: Vec<_> = set.cases.iter().filter(|c| language.is_none_or(|l| c.lang == l)).collect();
    let mut results = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        let result = match eval::run_case(&set, case, &responder) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        if !json && (verbose || !result.pass) {
            let mark = if result.pass { "ok  " } else { "FAIL" };
            println!("{mark} {:>2} [{}] {}", i + 1, result.lang, result.say.join(" / "));
            for a in &result.actions {
                println!("          {}({}){}", a.tool, a.arguments, if a.ok { String::new() } else { format!(" -> {}", a.result) });
            }
            println!("          \"{}\" ({} ms{})", result.reply, result.reply_ms, if result.wrong_language { ", wrong language" } else { "" });
            if let Some(why) = &result.why {
                println!("          {why}");
            }
        } else if !json {
            eprint!(".");
        }
        results.push(result);
    }
    if let Some(agent) = &agent {
        agent.llm().shutdown();
    }
    let s = eval::summarize(&results);
    if json {
        println!("{}", serde_json::to_string_pretty(&json!({"summary": s, "results": results})).unwrap_or_default());
    } else {
        eprintln!();
        let pct = |(p, n): (usize, usize)| if n == 0 { 0.0 } else { 100.0 * p as f64 / n as f64 };
        println!(
            "{} of {} right ({:.0} %): Italian {}/{}, English {}/{}; {} replies in the wrong language",
            s.passed,
            s.cases,
            pct((s.passed, s.cases)),
            s.passed_it.0,
            s.passed_it.1,
            s.passed_en.0,
            s.passed_en.1,
            s.wrong_language
        );
        println!(
            "first action (or answer): median {} ms, 90th percentile {} ms; reply: median {} ms, 90th percentile {} ms",
            s.first_ms_median, s.first_ms_p90, s.reply_ms_median, s.reply_ms_p90
        );
    }
    ExitCode::SUCCESS
}

/// `--stt small` (a model from `dualeye models`) or `--stt path/to/ggml-model.bin`.
fn stt_config(model: &str, language: SttLanguage) -> Result<SttConfig, String> {
    let server = stt::find_server().ok_or("whisper-server not found: install whisper.cpp (brew install whisper-cpp)")?;
    let file = std::path::Path::new(model);
    let model = if file.is_file() {
        file.to_path_buf()
    } else {
        let known = Model::by_id(model).ok_or_else(|| format!("{model}: no such file, and not one of {}", model_ids()))?;
        if !known.is_installed() {
            return Err(format!("the {model} model isn't downloaded: dualeye models download {model}"));
        }
        known.path().ok_or("no data folder for the models")?
    };
    Ok(SttConfig { server, model, language })
}

/// `--tts`: the default voices, or those `--tts-voice` names.
fn tts_config(voices: &[String]) -> Result<TtsConfig, String> {
    let mut config = TtsConfig::with_default_voices();
    for id in voices {
        let m = Model::by_id(id).filter(|m| m.kind == Kind::Voice).ok_or_else(|| format!("{id}: not a voice; see dualeye models"))?;
        if !m.is_installed() {
            return Err(format!("the {id} voice isn't downloaded: dualeye models download {id}"));
        }
        config.voices.insert(m.language.unwrap_or("en").to_string(), id.clone());
    }
    if config.voices.is_empty() {
        let defaults: Vec<&str> = ["it", "en"].into_iter().filter_map(models::default_voice).collect();
        return Err(format!("no voice downloaded: dualeye models download {}", defaults.join(" (and) ")));
    }
    if let Some(e) = config.engines().find(|e| !config.pythons.contains_key(e)) {
        return Err(format!("{} isn't installed: dualeye {} install", e.name(), e.key()));
    }
    Ok(config)
}

fn engine_command(engine: tts::Engine, action: Option<EngineAction>) -> ExitCode {
    let name = engine.name();
    match action {
        None => match engine.python() {
            Some(python) => println!("{name} installed: {}", python.display()),
            None => {
                println!("{name} isn't installed: dualeye {} install", engine.key());
                return ExitCode::FAILURE;
            }
        },
        Some(EngineAction::Install { python }) => {
            let Some(python) = python.or_else(|| tts::system_python(engine)) else {
                let versions = if engine == tts::Engine::Kokoro { "3.10 to 3.13" } else { "3.9 or later" };
                eprintln!("no Python {versions} on the PATH: install one, or pass --python");
                return ExitCode::FAILURE;
            };
            eprintln!("installing {} with {}", engine.requirement(), python.display());
            match tts::install(engine, &python, |line| eprintln!("  {line}")) {
                Ok(p) => println!("{}", p.display()),
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::FAILURE;
                }
            }
        }
    }
    ExitCode::SUCCESS
}

fn model_ids() -> String {
    models::MODELS.iter().map(|m| m.id).collect::<Vec<_>>().join(", ")
}

fn models_command(action: Option<ModelsAction>) -> ExitCode {
    let find = |id: &str| Model::by_id(id).ok_or_else(|| format!("unknown model {id:?}: {}", model_ids()));
    let result = match action {
        None => {
            let hw = Hardware::detect();
            let rec = hardware::recommend(&hw);
            println!("This computer: {}\n  {}\n", hw.summary(), rec.why);
            let kinds = [(Kind::Whisper, "Speech-to-text (Whisper, --stt)"), (Kind::Voice, "Voices (Piper or Kokoro, --tts)"), (Kind::Llm, "Language models (llama.cpp, --llm)")];
            for (kind, title) in kinds {
                println!("{title}");
                for m in Model::of_kind(kind) {
                    let mark = if m.is_installed() { "installed" } else { "" };
                    let default = m.id == models::DEFAULT_MODEL || m.id == models::DEFAULT_LLM || m.language.and_then(models::default_voice) == Some(m.id);
                    let recommended = m.id == rec.whisper || Some(m.id) == rec.llm;
                    let tag = match (default, recommended) {
                        (true, true) => " (default, recommended here)",
                        (false, true) => " (recommended here)",
                        (true, false) => " (default)",
                        (false, false) => "",
                    };
                    println!("  {:<22} {:>5} MB  {:<9}  {}{tag}", m.id, m.bytes() / 1_000_000, mark, m.note);
                    if kind != Kind::Whisper {
                        println!("  {:<22}                     license: {}", "", m.license);
                    }
                }
            }
            if let Some(dir) = stt::models_dir() {
                println!("\nin {}", dir.display());
            }
            Ok(())
        }
        Some(ModelsAction::Download { id }) => find(&id).and_then(|m| {
            let mut last = -1i32;
            let path = m
                .download(&AtomicBool::new(false), |p| {
                    let p = p.unwrap_or(0.0) as i32;
                    if p / 10 != last / 10 {
                        last = p;
                        eprint!("\r{} {p:>3} %", m.id);
                    }
                })
                .map_err(|e| format!("\n{}: {e}", m.id))?;
            eprintln!();
            println!("{}", path.display());
            Ok(())
        }),
        Some(ModelsAction::Remove { id }) => find(&id).and_then(|m| m.remove().map_err(|e| e.to_string())),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("{why}");
            ExitCode::FAILURE
        }
    }
}

fn board(port: Option<String>) -> Board {
    Board::new(port, &format!("dualeye-cli/{}", env!("CARGO_PKG_VERSION")))
}

fn tools(port: Option<String>, json: bool) -> ExitCode {
    let result = board(port).list_tools().map_err(|e| e.to_string());
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
    let result = board(port).call_tool(tool, arguments).map_err(|e| e.to_string());
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

fn image(port: Option<String>, file: Option<&std::path::Path>, screen: &str, clear: bool) -> ExitCode {
    let board = board(port);
    let result = match file {
        _ if clear => media::clear(&board, screen).map(|()| format!("{screen}: picture removed")),
        Some(path) => std::fs::read(path).map_err(|e| format!("{}: {e}", path.display())).and_then(|bytes| {
            let mut shown = -1;
            let sent = media::send(&board, screen, &bytes, |p| {
                let pct = (p * 100.0) as i32;
                if pct / 10 != shown / 10 {
                    shown = pct;
                    eprint!("\rsending {pct}%");
                }
            });
            eprintln!();
            sent.map(|p| match p.frames {
                1 => format!("{screen}: picture, {} KB", p.bytes / 1024),
                n => format!("{screen}: {n} frames ({} of the original), {:.1} s, {} KB", p.source_frames, f64::from(p.duration_ms) / 1000.0, p.bytes / 1024),
            })
        }),
        None => Err("give a file, or --clear".into()),
    };
    match result {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
