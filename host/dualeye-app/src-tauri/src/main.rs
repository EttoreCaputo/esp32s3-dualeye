// No console window next to the app in Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dualeye_core::claude::statusline;
use dualeye_core::mcp;

fn main() {
    // Claude Code runs us as its status line command and as an MCP server;
    // handle those before the single-instance check would hand them to the
    // running app.
    match std::env::args().nth(1).as_deref() {
        Some(statusline::FLAG) => {
            statusline::run(std::io::stdin().lock(), std::io::stdout().lock());
            return;
        }
        Some(mcp::FLAG) => {
            if let Err(e) = mcp::serve_stdio(None) {
                eprintln!("dualeye mcp: {e}");
                std::process::exit(1);
            }
            return;
        }
        _ => {}
    }
    dualeye_app_lib::run();
}
