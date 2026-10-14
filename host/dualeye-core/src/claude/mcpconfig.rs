//! The DualEye MCP server in Claude Code, without a terminal.
//!
//! [`connect`] does what `claude mcp add --scope user dualeye -- <exe> --mcp`
//! does: it adds `dualeye` to `mcpServers` in Claude Code's user config,
//! `~/.claude.json` (`.claude.json` in `CLAUDE_CONFIG_DIR` when that's set).
//! Every other key of that file stays as it was. [`disconnect`] takes ours
//! out.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value, json};

use super::statusline::{backup, parse_object, read_settings, write_settings};
/// Our name in `mcpServers`.
pub const NAME: &str = "dualeye";
/// `mcp::FLAG`, here without the `mcp` feature.
const FLAG: &str = "--mcp";

#[derive(Debug, Clone, Serialize)]
pub struct McpConfigStatus {
    /// Claude Code's config has a `dualeye` server that runs this app.
    pub registered: bool,
    pub config_path: Option<PathBuf>,
}

pub fn status() -> McpConfigStatus {
    status_at(config_path())
}

/// Register `exe --mcp` as the `dualeye` MCP server for every project, with a
/// one-off backup of the config.
pub fn connect(exe: &Path) -> io::Result<McpConfigStatus> {
    connect_at(config_path(), exe)
}

pub fn disconnect() -> io::Result<McpConfigStatus> {
    disconnect_at(config_path())
}

fn config_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join(".claude.json"));
    }
    super::home_dir().map(|h| h.join(".claude.json"))
}

fn status_at(path: Option<PathBuf>) -> McpConfigStatus {
    let registered = path.as_deref().and_then(read_settings).is_some_and(|c| is_ours(c.get("mcpServers").and_then(|s| s.get(NAME))));
    McpConfigStatus { registered, config_path: path }
}

fn is_ours(server: Option<&Value>) -> bool {
    server.and_then(|s| s.get("args")).and_then(Value::as_array).is_some_and(|a| a.iter().any(|v| v.as_str() == Some(FLAG)))
}

fn connect_at(path: Option<PathBuf>, exe: &Path) -> io::Result<McpConfigStatus> {
    let file = path.as_deref().ok_or_else(|| io::Error::other("can't find the Claude Code config"))?;
    let mut config = match fs::read_to_string(file) {
        Ok(text) => parse_object(&text)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(e),
    };
    let servers = config.entry("mcpServers").or_insert_with(|| Value::Object(Map::new()));
    let servers = servers.as_object_mut().ok_or_else(|| io::Error::other("`mcpServers` in .claude.json is not an object"))?;
    let entry = json!({"type": "stdio", "command": exe.to_string_lossy(), "args": [FLAG], "env": {}});
    if servers.get(NAME) != Some(&entry) {
        if !servers.contains_key(NAME) {
            backup(file)?;
        }
        servers.insert(NAME.into(), entry);
        write_settings(file, &config)?;
    }
    Ok(status_at(path))
}

fn disconnect_at(path: Option<PathBuf>) -> io::Result<McpConfigStatus> {
    let file = path.as_deref().ok_or_else(|| io::Error::other("can't find the Claude Code config"))?;
    if let Ok(text) = fs::read_to_string(file) {
        let mut config = parse_object(&text)?;
        let servers = config.get_mut("mcpServers").and_then(Value::as_object_mut);
        if let Some(servers) = servers.filter(|s| is_ours(s.get(NAME))) {
            servers.remove(NAME);
            write_settings(file, &config)?;
        }
    }
    Ok(status_at(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_keeps_the_rest_and_disconnect_takes_ours_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        let theirs = json!({"numStartups": 12, "mcpServers": {"github": {"type": "stdio", "command": "gh-mcp", "args": []}}});
        fs::write(&path, theirs.to_string()).unwrap();

        assert!(!status_at(Some(path.clone())).registered);
        assert!(connect_at(Some(path.clone()), Path::new("/Applications/DualEye.app/dualeye-app")).unwrap().registered);
        let now: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(now["numStartups"], 12);
        assert_eq!(now["mcpServers"]["github"]["command"], "gh-mcp");
        assert_eq!(now["mcpServers"]["dualeye"]["command"], "/Applications/DualEye.app/dualeye-app");
        assert_eq!(now["mcpServers"]["dualeye"]["args"], json!(["--mcp"]));
        assert!(dir.path().join(".claude.json.dualeye-backup").exists());

        // A moved app replaces ours.
        connect_at(Some(path.clone()), Path::new("/opt/dualeye-app")).unwrap();
        let now: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(now["mcpServers"]["dualeye"]["command"], "/opt/dualeye-app");

        assert!(!disconnect_at(Some(path.clone())).unwrap().registered);
        let back: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back, theirs);
    }

    #[test]
    fn connect_creates_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        assert!(connect_at(Some(path.clone()), Path::new("/x/dualeye-app")).unwrap().registered);
        assert!(!dir.path().join(".claude.json.dualeye-backup").exists());
    }

    #[test]
    fn someone_elses_dualeye_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        let theirs = json!({"mcpServers": {"dualeye": {"command": "my-own", "args": []}}});
        fs::write(&path, theirs.to_string()).unwrap();
        assert!(!disconnect_at(Some(path.clone())).unwrap().registered);
        let back: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back, theirs);
    }
}
