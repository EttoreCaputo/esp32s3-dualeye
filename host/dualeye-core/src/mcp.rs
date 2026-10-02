//! MCP server over stdio, for Claude Code, Claude Desktop and other MCP
//! clients (`dualeye mcp`, or `dualeye-app --mcp`).
//!
//! The board's tools are passed through as the firmware describes them in
//! `tools/list`, so a new firmware tool shows up without a host change. The
//! board is reached through [`Board`]: the running app or `dualeye` bridge,
//! or the port opened for each call. Two host tools read this computer,
//! `get_metrics` and `get_claude_usage`, and `speak` talks through the
//! board's speaker with the bridge's text-to-speech.
//!
//! The last board tool list is kept in `board-tools.json`, so the board's
//! tools are listed even when the client starts while the board is away;
//! calling one then says why it can't run. If the board had never been seen,
//! the server tells the client once its tools become available.

use std::borrow::Cow;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities,
    ServerConfig, Tool as McpTool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, Peer, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Value, json};

use crate::claude::ClaudeUsage;
use crate::claude::alerts;
use crate::music::Music;
use crate::timers::Timers;
use crate::hub::Board;
use crate::link::{CallError, Tool, ToolResult};
use crate::sensors::Collector;
use crate::snapshot::{Snapshot, short_floats};

/// The argument that turns the app binary into the MCP server.
pub const FLAG: &str = "--mcp";

const INSTRUCTIONS: &str = "DualEye is a small USB display on the user's desk with two round screens: \
`left` and `right`; any face goes on either one: the computer's CPU or GPU (set_face's source), network, disk, battery, \
Claude Code usage (claude and clawd), a picture the user uploaded in the DualEye app (image), what's playing (music) \
or a pair of eyes that follow the mouse (eyes). \
The board's tools change what the screens show; show_text puts a short ASCII message on them. \
get_metrics and get_claude_usage read this computer, and work without the board. \
speak says a short sentence out loud through the board's speaker, in Italian or English. \
set_timer, set_reminder, pomodoro, control_timer and get_timers run timers on the host: the board counts the first one down on a screen \
and rings when it's up (they need the DualEye app or `dualeye` running to ring). \
media_control and now_playing play, pause or skip the music on this computer and say what's playing.";

/// A bridge sample older than this is not "current"; sample here instead.
const FRESH: Duration = Duration::from_secs(5);
/// While the board's tools are unknown, look for them this often.
const WATCH: Duration = Duration::from_secs(10);

/// Serve MCP on stdin/stdout until the client closes it. `port` is used only
/// when no bridge is running (`None` auto-detects the board).
pub fn serve_stdio(port: Option<String>) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let server = DualEyeMcp::new(Board::new(port, &format!("dualeye-mcp/{}", env!("CARGO_PKG_VERSION"))), cache_file());
        let running = server.clone().serve(rmcp::transport::stdio()).await.map_err(io::Error::other)?;
        let watcher = tokio::spawn(watch_tools(server, running.peer().clone()));
        let _ = running.waiting().await;
        watcher.abort();
        Ok(())
    })
}

fn cache_file() -> Option<PathBuf> {
    crate::link::tools_cache_file()
}

#[derive(Clone)]
pub struct DualEyeMcp {
    inner: Arc<Inner>,
}

struct Inner {
    board: Board,
    cache: Option<PathBuf>,
    /// Board tools as last listed to the client, and whether they came from the board itself.
    listed: Mutex<Option<(Vec<Tool>, bool)>>,
    collector: Mutex<Option<Collector>>,
    claude: Mutex<Option<ClaudeUsage>>,
}

impl DualEyeMcp {
    pub fn new(board: Board, cache: Option<PathBuf>) -> Self {
        Self {
            inner: Arc::new(Inner { board, cache, listed: Mutex::default(), collector: Mutex::default(), claude: Mutex::default() }),
        }
    }

    /// The board's tools: live, else the last ones seen. The flag says which.
    fn board_tools(&self) -> (Vec<Tool>, bool) {
        let inner = &self.inner;
        match inner.board.list_tools() {
            Ok(tools) => {
                if let Some(path) = &inner.cache
                    && let Ok(json) = serde_json::to_vec_pretty(&tools)
                {
                    let _ = path.parent().map(fs::create_dir_all);
                    let _ = fs::write(path, json);
                }
                (tools, true)
            }
            Err(_) => {
                let cached = inner.cache.as_ref().and_then(|p| fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok());
                (cached.unwrap_or_default(), false)
            }
        }
    }

    fn all_tools(&self) -> Vec<McpTool> {
        let (board, live) = self.board_tools();
        let mut tools: Vec<McpTool> = host_tools().into_iter().filter_map(|t| serde_json::from_value(t).ok()).collect();
        let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        for tool in &board {
            // Host tools win a name clash.
            if !names.contains(&tool.name)
                && let Ok(t) = serde_json::to_value(tool).and_then(serde_json::from_value)
            {
                tools.push(t);
            }
        }
        *self.inner.listed.lock().unwrap() = Some((board, live));
        tools
    }

    fn call(&self, name: &str, arguments: Value) -> Result<CallToolResult, ErrorData> {
        match name {
            "get_metrics" => Ok(json_result(self.metrics().metrics_json())),
            "get_claude_usage" => Ok(match self.claude_usage() {
                Some(usage) => json_result(usage),
                None => text_result("No Claude Code usage on this computer: Claude Code has not run here, or keeps its data elsewhere (CLAUDE_CONFIG_DIR).", false),
            }),
            "speak" => {
                let text = arguments.get("text").and_then(Value::as_str).unwrap_or("");
                if text.trim().is_empty() {
                    return Err(ErrorData::invalid_params("text must be a non-empty string", None));
                }
                Ok(match self.inner.board.say(text, arguments.get("language").and_then(Value::as_str)) {
                    Ok(spoken) => json_result(spoken),
                    Err(e) => text_result(&format!("The DualEye board can't speak: {e}."), true),
                })
            }
            name if Music::tools().iter().any(|t| t.name == name) => Ok(match Music::default().call_tool(name, &arguments) {
                Some(Ok(text)) if name == "now_playing" => json_result(serde_json::from_str(&text).unwrap_or(Value::String(text))),
                Some(Ok(text)) => text_result(&text, false),
                Some(Err(e)) => text_result(&e, true),
                None => text_result(&format!("unknown tool {name}"), true),
            }),
            name if Timers::tools().iter().any(|t| t.name == name) => {
                let language = arguments.get("language").and_then(Value::as_str).unwrap_or(alerts::system_language()).to_string();
                Ok(match self.inner.board.timer_tool(name, &arguments, &language) {
                    Ok(text) if name == "get_timers" => json_result(serde_json::from_str(&text).unwrap_or(Value::String(text))),
                    Ok(text) => text_result(&text, false),
                    Err(e) => text_result(&e, true),
                })
            }
            _ => match self.inner.board.call_tool(name, arguments) {
                Ok(result) => to_mcp(result),
                Err(CallError::Rpc { code: -32602, message }) => Err(ErrorData::invalid_params(message, None)),
                Err(e) => Ok(text_result(&format!("The DualEye board can't run {name}: {e}."), true)),
            },
        }
    }

    /// The bridge's sample while it is fresh, else one taken here.
    fn metrics(&self) -> Snapshot {
        if let Some((snapshot, _)) = self.inner.board.bridge_snapshot().filter(|(_, age)| *age < FRESH) {
            return snapshot;
        }
        let mut slot = self.inner.collector.lock().unwrap();
        let collector = match slot.as_mut() {
            Some(c) => c,
            None => {
                // CPU load is the change between two samples.
                let c = slot.insert(Collector::new());
                c.sample();
                std::thread::sleep(Duration::from_millis(500));
                c
            }
        };
        collector.sample()
    }

    fn claude_usage(&self) -> Option<Value> {
        let from_bridge = self.inner.board.bridge_snapshot().filter(|(_, age)| *age < FRESH).and_then(|(s, _)| s.claude);
        let usage = from_bridge.or_else(|| self.inner.claude.lock().unwrap().get_or_insert_with(ClaudeUsage::new).sample())?;
        Some(short_floats(&usage))
    }
}

fn host_tools() -> Vec<Value> {
    let no_args = json!({"type": "object", "properties": {}, "additionalProperties": false});
    let timers = Timers::tools().into_iter().map(|t| {
        let read_only = t.name == "get_timers";
        json!({
            "name": t.name,
            "description": t.description,
            "inputSchema": t.input_schema,
            "annotations": {"readOnlyHint": read_only, "destructiveHint": false, "openWorldHint": false},
        })
    });
    let music = Music::tools().into_iter().map(|t| {
        let read_only = t.name == "now_playing";
        json!({
            "name": t.name,
            "description": t.description,
            "inputSchema": t.input_schema,
            "annotations": {"readOnlyHint": read_only, "destructiveHint": false, "openWorldHint": false},
        })
    });
    let mut tools = vec![
        json!({
            "name": "get_metrics",
            "title": "Computer metrics",
            "description": "Current CPU and GPU temperature, load, clock, power and memory of this computer, and its fan speeds.",
            "inputSchema": no_args,
            "annotations": {"readOnlyHint": true, "openWorldHint": false},
        }),
        json!({
            "name": "get_claude_usage",
            "title": "Claude Code usage",
            "description": "Claude Code usage on this computer: tokens in the current 5-hour window (tok) and today, \
                            share of the plan's 5-hour (s_pct) and weekly (w_pct) limits used, minutes until the window resets (left_min), \
                            whether Claude is working, idle or asleep, and the latest model.",
            "inputSchema": no_args,
            "annotations": {"readOnlyHint": true, "openWorldHint": false},
        }),
        json!({
            "name": "speak",
            "title": "Speak through the board",
            "description": "Say a short text out loud through the DualEye's speaker. Returns once it has been played. \
                            Needs the DualEye app with spoken replies on (or `dualeye --tts`) running.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": {"type": "string", "maxLength": 500, "description": "Plain sentences, no markdown"},
                    "language": {"type": "string", "enum": ["it", "en"], "description": "Default: what the text looks like"},
                },
                "required": ["text"],
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false},
        }),
    ];
    tools.extend(timers);
    tools.extend(music);
    tools
}

fn to_mcp(result: ToolResult) -> Result<CallToolResult, ErrorData> {
    serde_json::to_value(result).and_then(serde_json::from_value).map_err(|e| ErrorData::internal_error(format!("board result: {e}"), None))
}

fn json_result(value: Value) -> CallToolResult {
    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
    let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
    result.structured_content = Some(value);
    result
}

fn text_result(text: &str, is_error: bool) -> CallToolResult {
    if is_error { CallToolResult::error(vec![ContentBlock::text(text)]) } else { CallToolResult::success(vec![ContentBlock::text(text)]) }
}

impl ServerHandler for DualEyeMcp {
    fn get_info(&self) -> ServerConfig {
        let mut info = Implementation::new("dualeye", env!("CARGO_PKG_VERSION"));
        info.title = Some("DualEye".into());
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_tool_list_changed().build())
            .with_server_info(info)
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListToolsResult, ErrorData> {
        let this = self.clone();
        let tools = tokio::task::spawn_blocking(move || this.all_tools()).await.map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(&self, request: CallToolRequestParams, _context: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        let this = self.clone();
        let name: Cow<'static, str> = request.name;
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        tokio::task::spawn_blocking(move || this.call(&name, arguments))
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
            .map(CallToolResponse::from)
    }
}

/// Tell the client when the board's tools turn up after it listed them without.
async fn watch_tools(server: DualEyeMcp, peer: Peer<RoleServer>) {
    loop {
        tokio::time::sleep(WATCH).await;
        let listed = server.inner.listed.lock().unwrap().clone();
        let Some((before, false)) = listed else { continue };
        let this = server.clone();
        let Ok((now, true)) = tokio::task::spawn_blocking(move || this.board_tools()).await else { continue };
        let names = |tools: &[Tool]| tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
        if names(&now) != names(&before) && peer.notify_tool_list_changed().await.is_err() {
            return;
        }
        *server.inner.listed.lock().unwrap() = Some((now, true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_tools_convert_to_mcp() {
        for tool in host_tools() {
            let t: McpTool = serde_json::from_value(tool).expect("valid MCP tool");
            // Only speak, the timers and media_control do something.
            let reads = crate::agent::is_read_only(&t.name);
            assert_eq!(t.annotations.and_then(|a| a.read_only_hint), Some(reads), "{}", t.name);
        }
    }

    #[test]
    fn board_results_pass_through() {
        let board: ToolResult = serde_json::from_value(json!({
            "content": [{"type": "text", "text": "left: rings"}],
            "isError": false,
        }))
        .unwrap();
        let mcp = to_mcp(board).unwrap();
        assert_eq!(mcp.is_error, Some(false));
        assert_eq!(serde_json::to_value(&mcp.content).unwrap(), json!([{"type": "text", "text": "left: rings"}]));
    }

    #[test]
    fn board_tools_come_from_the_cache_when_the_board_is_away() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("board-tools.json");
        let tools = json!([{"name": "set_face", "description": "Switch the watch face", "inputSchema": {"type": "object"}}]);
        fs::write(&cache, tools.to_string()).unwrap();
        // A port that isn't there, and no hub: the board can't be reached.
        let board = Board::with_hub_file(Some("/dev/does-not-exist".into()), "test", None);
        let server = DualEyeMcp::new(board, Some(cache));
        let (listed, live) = server.board_tools();
        assert!(!live);
        assert_eq!(listed[0].name, "set_face");
    }
}
