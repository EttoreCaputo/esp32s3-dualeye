//! The apps on this computer, for "apri Spotify": [`open`] finds the
//! installed app whose name is closest to what was said and starts it.
//!
//! Only apps found where the system keeps them can be opened, never a path
//! or a command: `.app` bundles in the Applications folders on macOS, the
//! Start menu's shortcuts on Windows, the `.desktop` entries on Linux.
//! Whisper's spellings ("spotifai") and a few Italian names ("calcolatrice")
//! still find them.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use crate::link::Tool;

/// An app that can be opened.
#[derive(Debug, Clone, PartialEq)]
pub struct App {
    /// As the system shows it: "Google Chrome".
    pub name: String,
    /// The bundle, shortcut or desktop entry.
    pub path: PathBuf,
}

/// Italian (and short) names for apps whose name on disk is another.
const ALIASES: &[(&str, &str)] = &[
    ("calcolatrice", "calculator"),
    ("calendario", "calendar"),
    ("contatti", "contacts"),
    ("note", "notes"),
    ("posta", "mail"),
    ("foto", "photos"),
    ("musica", "music"),
    ("mappe", "maps"),
    ("messaggi", "messages"),
    ("meteo", "weather"),
    ("orologio", "clock"),
    ("anteprima", "preview"),
    ("terminale", "terminal"),
    ("impostazioni", "settings"),
    ("impostazioni di sistema", "system settings"),
    ("preferenze di sistema", "system settings"),
    ("chrome", "google chrome"),
    ("vs code", "visual studio code"),
    ("vscode", "visual studio code"),
    ("word", "microsoft word"),
    ("excel", "microsoft excel"),
    ("powerpoint", "microsoft powerpoint"),
    ("outlook", "microsoft outlook"),
    ("teams", "microsoft teams"),
    ("esplora risorse", "file explorer"),
    ("blocco note", "notepad"),
];

/// Lowercase letters and digits, accents dropped, words split by one space.
fn normalize(s: &str) -> String {
    let plain: String = s
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ä' => 'a',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        })
        .collect();
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (row[j + 1] + 1).min(row[j] + 1).min(diagonal + usize::from(ca != *cb));
            diagonal = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

/// How well `name` (normalized) fits `said` (normalized): higher is better, 0 not at all.
fn score(said: &str, name: &str) -> u32 {
    if said == name {
        return 100;
    }
    let compact = |s: &str| s.replace(' ', "");
    let (said_c, name_c) = (compact(said), compact(name));
    if said_c == name_c {
        return 95;
    }
    let name_words: Vec<&str> = name.split(' ').collect();
    // "chrome" for "google chrome", "studio code" for "visual studio code".
    if said.split(' ').all(|w| name_words.contains(&w)) {
        return 80;
    }
    if name.starts_with(said) && said.len() >= 3 {
        return 70;
    }
    // Whisper's spellings: "spotifai", "whatsup".
    let distance = levenshtein(&said_c, &name_c);
    if said_c.len() >= 4 && distance * 4 <= said_c.len().max(name_c.len()) {
        return 60 - distance as u32;
    }
    0
}

/// The app among `apps` that `said` names, if one fits.
pub fn find<'a>(said: &str, apps: &'a [App]) -> Option<&'a App> {
    let said = normalize(said);
    if said.is_empty() {
        return None;
    }
    let alias = ALIASES.iter().find(|(a, _)| *a == said).map(|(_, to)| *to);
    apps.iter()
        .filter_map(|app| {
            let name = normalize(&app.name);
            let s = score(&said, &name).max(alias.map_or(0, |a| score(a, &name).saturating_sub(1)));
            (s > 0).then_some((s, app))
        })
        // The best fit, the shortest name of those: "Music" over "Music Maker".
        .max_by(|(sa, a), (sb, b)| sa.cmp(sb).then(b.name.len().cmp(&a.name.len())))
        .map(|(_, app)| app)
}

/// Open the app `said` names: its name as shown, or why not.
pub fn open(said: &str) -> Result<String, String> {
    let apps = installed();
    let app = find(said, &apps).ok_or_else(|| format!("no app called \"{said}\" on this computer"))?;
    launch(app)?;
    Ok(app.name.clone())
}

/// The host's app tool, for the voice.
pub fn tools() -> Vec<Tool> {
    vec![Tool {
        name: "open_app".into(),
        description: "Open an app on this computer, like Spotify or Safari.".into(),
        input_schema: json!({"type": "object", "properties": {
            "app": {"type": "string", "description": "the app's name as the user said it"}
        }, "required": ["app"]}),
    }]
}

/// Run one of [`tools`]; `None` for another tool.
pub fn call_tool(name: &str, args: &Value) -> Option<Result<String, String>> {
    (name == "open_app").then(|| {
        let said = args["app"].as_str().filter(|s| !s.trim().is_empty()).ok_or("app must be a non-empty string")?;
        open(said).map(|name| format!("{name} opened"))
    })
}

/// Entries of `dir`, without the unreadable.
fn entries(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The apps installed, as [`find`] takes them.
pub fn installed() -> Vec<App> {
    platform::installed()
}

fn launch(app: &App) -> Result<(), String> {
    platform::launch(app)
}

/// Run `command` and say what went wrong if it did.
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn run(mut command: Command) -> Result<(), String> {
    let out = command.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if err.is_empty() { format!("exited with {}", out.status) } else { err })
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    /// `.app` bundles in the Applications folders, and one folder down
    /// ("Adobe Photoshop/Adobe Photoshop.app").
    pub fn installed() -> Vec<App> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut dirs: Vec<PathBuf> = ["/Applications", "/Applications/Utilities", "/System/Applications", "/System/Applications/Utilities"]
            .iter()
            .map(PathBuf::from)
            .collect();
        dirs.extend(home.map(|h| h.join("Applications")));
        let mut apps = Vec::new();
        for dir in dirs {
            for path in entries(&dir) {
                if path.extension().is_some_and(|e| e == "app") {
                    apps.push(App { name: stem(&path), path });
                } else if path.is_dir() && !path.ends_with("Utilities") {
                    apps.extend(entries(&path).into_iter().filter(|p| p.extension().is_some_and(|e| e == "app")).map(|p| App { name: stem(&p), path: p }));
                }
            }
        }
        // Finder is in CoreServices.
        let finder = PathBuf::from("/System/Library/CoreServices/Finder.app");
        if finder.exists() {
            apps.push(App { name: "Finder".into(), path: finder });
        }
        apps
    }

    pub fn launch(app: &App) -> Result<(), String> {
        let mut command = Command::new("open");
        command.arg(&app.path);
        run(command)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;

    /// The Start menu's shortcuts, the user's and everyone's.
    pub fn installed() -> Vec<App> {
        let roots = [std::env::var_os("APPDATA"), std::env::var_os("PROGRAMDATA")];
        let mut apps = Vec::new();
        for root in roots.into_iter().flatten() {
            let mut stack = vec![PathBuf::from(root).join(r"Microsoft\Windows\Start Menu\Programs")];
            while let Some(dir) = stack.pop() {
                for path in entries(&dir) {
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk") || e.eq_ignore_ascii_case("url")) {
                        let name = stem(&path);
                        // Not "Uninstall Foo" for "Foo".
                        if !name.to_lowercase().contains("uninstall") {
                            apps.push(App { name, path });
                        }
                    }
                }
            }
        }
        // Store apps have no shortcut there.
        for (name, uri) in [("Settings", "ms-settings:"), ("Calculator", "calculator:"), ("File Explorer", "explorer.exe")] {
            if !apps.iter().any(|a| a.name.eq_ignore_ascii_case(name)) {
                apps.push(App { name: name.into(), path: PathBuf::from(uri) });
            }
        }
        apps
    }

    pub fn launch(app: &App) -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]).arg(&app.path).creation_flags(CREATE_NO_WINDOW);
        run(command)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    /// What a `.desktop` file says: its name and command, unless hidden.
    fn desktop_entry(path: &Path) -> Option<(String, String)> {
        let text = fs::read_to_string(path).ok()?;
        let (mut name, mut exec, mut in_entry) = (None, None, false);
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_entry = line == "[Desktop Entry]";
                continue;
            }
            if !in_entry {
                continue;
            }
            match line.split_once('=') {
                Some(("Name", v)) => name = name.or(Some(v.trim().to_string())),
                Some(("Exec", v)) => exec = exec.or(Some(v.trim().to_string())),
                Some(("NoDisplay" | "Hidden", v)) if v.trim() == "true" => return None,
                Some(("Type", v)) if v.trim() != "Application" => return None,
                _ => {}
            }
        }
        Some((name?, exec?))
    }

    fn dirs() -> Vec<PathBuf> {
        let home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
        let data = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        home.into_iter()
            .chain(data.split(':').filter(|d| !d.is_empty()).map(PathBuf::from))
            .chain([PathBuf::from("/var/lib/flatpak/exports/share"), PathBuf::from("/var/lib/snapd/desktop")])
            .map(|d| d.join("applications"))
            .collect()
    }

    pub fn installed() -> Vec<App> {
        let mut apps: Vec<App> = Vec::new();
        for dir in dirs() {
            for path in entries(&dir) {
                if path.extension().is_some_and(|e| e == "desktop")
                    && let Some((name, _)) = desktop_entry(&path)
                    // The user's own entry hides the system's of the same name.
                    && !apps.iter().any(|a| a.path.file_name() == path.file_name())
                {
                    apps.push(App { name, path });
                }
            }
        }
        apps
    }

    pub fn launch(app: &App) -> Result<(), String> {
        let (_, exec) = desktop_entry(&app.path).ok_or("the app's desktop entry is gone")?;
        // Without the field codes (%U, %f...): no files to open.
        let line: Vec<&str> = exec.split_whitespace().filter(|w| !(w.len() == 2 && w.starts_with('%'))).collect();
        let (program, args) = line.split_first().ok_or("the app's desktop entry has no command")?;
        let mut child = Command::new(program.trim_matches('"'))
            .args(args.iter().map(|a| a.trim_matches('"')))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("{program}: {e}"))?;
        // It runs on its own; only reap it.
        std::thread::spawn(move || child.wait());
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod platform {
    use super::*;

    pub fn installed() -> Vec<App> {
        Vec::new()
    }

    pub fn launch(_app: &App) -> Result<(), String> {
        Err("opening apps isn't supported on this system".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps() -> Vec<App> {
        ["Spotify", "Safari", "Google Chrome", "Calculator", "Music", "Music Maker", "Visual Studio Code", "WhatsApp", "System Settings", "Claude"]
            .iter()
            .map(|n| App { name: n.to_string(), path: PathBuf::from(format!("/Applications/{n}.app")) })
            .collect()
    }

    fn found(said: &str) -> Option<String> {
        find(said, &apps()).map(|a| a.name.clone())
    }

    #[test]
    fn finds_apps_by_what_was_said() {
        assert_eq!(found("spotify").as_deref(), Some("Spotify"));
        assert_eq!(found("Spotifai").as_deref(), Some("Spotify"));
        assert_eq!(found("chrome").as_deref(), Some("Google Chrome"));
        assert_eq!(found("calcolatrice").as_deref(), Some("Calculator"));
        assert_eq!(found("vs code").as_deref(), Some("Visual Studio Code"));
        assert_eq!(found("Whats App").as_deref(), Some("WhatsApp"));
        assert_eq!(found("musica").as_deref(), Some("Music"));
        assert_eq!(found("impostazioni").as_deref(), Some("System Settings"));
        assert_eq!(found("claude").as_deref(), Some("Claude"));
        assert_eq!(found("photoshop"), None);
        assert_eq!(found(""), None);
    }

    #[test]
    fn the_tool_wants_a_name() {
        assert!(call_tool("open_app", &json!({})).unwrap().is_err());
        assert!(call_tool("set_face", &json!({})).is_none());
    }
}
