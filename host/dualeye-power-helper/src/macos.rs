use std::fs::{self, Permissions};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, ExitCode, Stdio};
use std::ptr::null;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::data::CFData;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use plist::{Dictionary, Value};
use security_framework::os::macos::code_signing::{Flags, GuestAttributes, SecCode, SecRequirement};

/// Keep in sync with `SOCKET` in dualeye-core's `sensors/power_helper.rs`.
const SOCKET: &str = "/var/run/com.dualeye.monitor.power.sock";

const POWERMETRICS: &str = "/usr/bin/powermetrics";

/// How often powermetrics reports, in ms; the app samples once a second.
const INTERVAL_MS: &str = "1000";

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecCodeCopySigningInformation(code: CFTypeRef, flags: u32, information: *mut CFDictionaryRef) -> i32;
    static kSecCodeInfoTeamIdentifier: CFStringRef;
}

/// `kSecCSSigningInformation`.
const SIGNING_INFORMATION: u32 = 1 << 1;

#[derive(Default)]
struct State {
    clients: Vec<UnixStream>,
    /// powermetrics is running, feeding `clients`.
    sampling: bool,
}

pub fn run() -> ExitCode {
    let _ = fs::remove_file(SOCKET);
    let listener = match UnixListener::bind(SOCKET) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot listen on {SOCKET}: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Anyone may connect; `allowed` turns away code from another team.
    if let Err(e) = fs::set_permissions(SOCKET, Permissions::from_mode(0o666)) {
        eprintln!("cannot open {SOCKET} to other users: {e}");
    }
    let requirement = match own_team() {
        Some(team) => {
            eprintln!("serving code signed by team {team}");
            let text = format!("anchor apple generic and certificate leaf[subject.OU] = \"{team}\"");
            match text.parse::<SecRequirement>() {
                Ok(r) => Some(r),
                Err(e) => {
                    eprintln!("bad code requirement {text}: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        None => {
            eprintln!("not signed by a team: serving any client");
            None
        }
    };

    let state = Arc::new(Mutex::new(State::default()));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if !allowed(&stream, requirement.as_ref()) {
            eprintln!("turned away a client not signed by this team");
            continue;
        }
        // A client that stops reading must not hold up the others.
        let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
        let mut s = state.lock().unwrap();
        s.clients.push(stream);
        if !s.sampling {
            s.sampling = true;
            let state = Arc::clone(&state);
            thread::spawn(move || sample(&state));
        }
    }
    ExitCode::SUCCESS
}

/// Run powermetrics until the last client leaves, sending each sample to all.
fn sample(state: &Mutex<State>) {
    let child = Command::new(POWERMETRICS)
        .args(["--samplers", "cpu_power,gpu_power", "-i", INTERVAL_MS, "-f", "plist"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cannot run {POWERMETRICS}: {e}");
            // The clients retry, which retries powermetrics.
            let mut s = state.lock().unwrap();
            s.clients.clear();
            s.sampling = false;
            return;
        }
    };
    let mut out = BufReader::new(child.stdout.take().expect("piped stdout"));
    // The plist documents are separated by NUL bytes.
    let mut doc = Vec::new();
    loop {
        doc.clear();
        if out.read_until(0, &mut doc).unwrap_or(0) == 0 {
            break;
        }
        let Some(line) = parse(&doc) else { continue };
        let mut s = state.lock().unwrap();
        s.clients.retain_mut(|c| c.write_all(line.as_bytes()).is_ok());
        if s.clients.is_empty() {
            s.sampling = false;
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let mut s = state.lock().unwrap();
    if s.sampling {
        // powermetrics quit by itself; drop the clients so they reconnect.
        eprintln!("{POWERMETRICS} exited");
        s.clients.clear();
        s.sampling = false;
    }
}

/// One powermetrics report as a JSON line, `None` without a CPU power.
fn parse(doc: &[u8]) -> Option<String> {
    let start = doc.iter().position(|b| !b.is_ascii_whitespace() && *b != 0)?;
    let end = doc.iter().rposition(|b| !b.is_ascii_whitespace() && *b != 0)? + 1;
    let report: Value = plist::from_bytes(&doc[start..end]).ok()?;
    let report = report.as_dictionary()?;
    let processor = report.get("processor")?.as_dictionary()?;
    let secs = number(report, "elapsed_ns").map(|ns| ns / 1e9).filter(|s| *s > 0.0);
    // `*_power` is the average in mW; older releases only give `*_energy`,
    // the mJ spent over the interval.
    let watts = |dict: &Dictionary, name: &str| {
        number(dict, &format!("{name}_power"))
            .map(|mw| mw / 1000.0)
            .or_else(|| Some(number(dict, &format!("{name}_energy"))? / 1000.0 / secs?))
    };
    let cpu = watts(processor, "cpu")?;
    let gpu = watts(processor, "gpu").or_else(|| watts(report.get("gpu")?.as_dictionary()?, "gpu"));
    let line = serde_json::json!({
        "cpu_w": cpu,
        "gpu_w": gpu,
        "ane_w": watts(processor, "ane"),
        "package_w": number(processor, "combined_power").map(|mw| mw / 1000.0),
    });
    Some(format!("{line}\n"))
}

fn number(dict: &Dictionary, key: &str) -> Option<f64> {
    match dict.get(key)? {
        Value::Real(r) => Some(*r),
        Value::Integer(i) => i.as_signed().map(|i| i as f64).or_else(|| i.as_unsigned().map(|u| u as f64)),
        _ => None,
    }
    .filter(|v| v.is_finite() && *v >= 0.0)
}

/// The team that signed this helper; `None` when ad-hoc signed or unsigned.
fn own_team() -> Option<String> {
    let me = SecCode::for_self(Flags::NONE).ok()?;
    let mut info: CFDictionaryRef = null();
    // A dynamic code object stands in for the static one.
    if unsafe { SecCodeCopySigningInformation(me.as_CFTypeRef(), SIGNING_INFORMATION, &mut info) } != 0
        || info.is_null()
    {
        return None;
    }
    let info = unsafe { CFDictionary::<CFString, CFType>::wrap_under_create_rule(info) };
    let key = unsafe { CFString::wrap_under_get_rule(kSecCodeInfoTeamIdentifier) };
    Some(info.find(&key)?.downcast::<CFString>()?.to_string())
}

/// Whether the process at the other end satisfies `requirement`, checked by
/// its audit token (a pid could be reused).
fn allowed(stream: &UnixStream, requirement: Option<&SecRequirement>) -> bool {
    let Some(requirement) = requirement else {
        return true;
    };
    let mut token = [0u8; 32];
    let mut len = token.len() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERTOKEN,
            token.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if rc != 0 {
        return false;
    }
    let token = CFData::from_buffer(&token[..len as usize]);
    let mut attrs = GuestAttributes::new();
    attrs.set_audit_token(token.as_concrete_TypeRef());
    SecCode::copy_guest_with_attribues(None, &attrs, Flags::NONE)
        .and_then(|code| code.check_validity(Flags::NONE, requirement))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(processor: &str) -> Vec<u8> {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
             <key>elapsed_ns</key><integer>500000000</integer>\
             <key>processor</key><dict>{processor}</dict></dict></plist>\n\0"
        )
        .into_bytes()
    }

    fn json(line: &str) -> serde_json::Value {
        serde_json::from_str(line).unwrap()
    }

    #[test]
    fn reads_power_in_milliwatts() {
        let doc = report(
            "<key>cpu_power</key><real>3250.5</real><key>gpu_power</key><integer>120</integer>\
             <key>ane_power</key><integer>0</integer><key>combined_power</key><real>3370.5</real>",
        );
        let v = json(&parse(&doc).unwrap());
        assert_eq!(v["cpu_w"], 3.2505);
        assert_eq!(v["gpu_w"], 0.12);
        assert_eq!(v["ane_w"], 0.0);
        assert_eq!(v["package_w"], 3.3705);
    }

    #[test]
    fn falls_back_to_energy_over_the_interval() {
        let doc = report("<key>cpu_energy</key><integer>1500</integer><key>gpu_energy</key><integer>50</integer>");
        let v = json(&parse(&doc).unwrap());
        assert_eq!(v["cpu_w"], 3.0);
        assert_eq!(v["gpu_w"], 0.1);
        assert!(v["package_w"].is_null());
    }

    #[test]
    fn skips_reports_without_cpu_power() {
        assert!(parse(&report("<key>gpu_power</key><integer>120</integer>")).is_none());
        assert!(parse(b"\n\0").is_none());
    }
}
