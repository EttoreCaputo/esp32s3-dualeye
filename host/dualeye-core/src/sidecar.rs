//! Helper processes serving HTTP on `127.0.0.1`: whisper.cpp's
//! `whisper-server` ([`crate::stt`]), Piper's `http_server` ([`crate::tts`])
//! and llama.cpp's `llama-server` ([`crate::llm`]).
//!
//! A [`Process`] is started on a free port and waited for until it accepts
//! connections; dropping it stops it. Its pid is kept in a file, so one left
//! running by a host that was killed is stopped by the next. Requests are
//! plain HTTP/1.1, so no HTTP client is needed.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) struct Process {
    child: Child,
    pub addr: SocketAddr,
    pid_file: Option<PathBuf>,
}

impl Process {
    /// Start `cmd` with `--host 127.0.0.1 --port N` appended, its stderr in
    /// `log`, and wait up to `timeout` for it to listen. `marker` is part of
    /// its name or command line, to recognise a stale one from `pid_file`.
    pub fn start(mut cmd: Command, marker: &str, pid_file: Option<PathBuf>, log: Option<PathBuf>, timeout: Duration) -> Result<Self, String> {
        if let Some(f) = &pid_file {
            kill_stale(f, marker);
        }
        let port = free_port().map_err(|e| e.to_string())?;
        let log = log.and_then(|p| p.parent().and_then(|d| fs::create_dir_all(d).ok()).and_then(|_| File::create(p).ok()));
        let program = cmd.get_program().to_string_lossy().into_owned();
        let mut child = cmd
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log.map_or_else(Stdio::null, Stdio::from))
            .spawn()
            .map_err(|e| format!("{program}: {e}"))?;
        if let Some(f) = &pid_file {
            let _ = fs::write(f, child.id().to_string());
        }
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!("exited at startup ({status})"));
            }
            if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
                return Ok(Self { child, addr, pid_file });
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err("didn't start in time".into());
            }
            thread::sleep(Duration::from_millis(200));
        }
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(f) = &self.pid_file {
            let _ = fs::remove_file(f);
        }
    }
}

fn kill_stale(pid_file: &PathBuf, marker: &str) {
    let Some(pid) = fs::read_to_string(pid_file).ok().and_then(|s| s.trim().parse::<usize>().ok()) else {
        return;
    };
    let pid = sysinfo::Pid::from(pid);
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        sysinfo::ProcessRefreshKind::nothing().with_cmd(sysinfo::UpdateKind::Always),
    );
    if let Some(p) = sys.process(pid)
        && (p.name().to_string_lossy().contains(marker) || p.cmd().iter().any(|a| a.to_string_lossy().contains(marker)))
    {
        p.kill();
    }
}

fn free_port() -> io::Result<u16> {
    Ok(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?.local_addr()?.port())
}

/// GET `path`; the response body (status 200 only).
pub(crate) fn get(addr: SocketAddr, path: &str, timeout: Duration) -> io::Result<Vec<u8>> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(timeout))?;
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n")?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    http_body(&response)
}

/// POST `body` to `path`; the response body (status 200 only).
pub(crate) fn post(addr: SocketAddr, path: &str, content_type: &str, body: &[u8], timeout: Duration) -> io::Result<Vec<u8>> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(timeout))?;
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    http_body(&response)
}

/// The body of an HTTP/1.1 response with status 200, plain or chunked.
pub(crate) fn http_body(response: &[u8]) -> io::Result<Vec<u8>> {
    let bad = |why: &str| io::Error::new(io::ErrorKind::InvalidData, why.to_string());
    let split = response.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| bad("no HTTP header"))?;
    let head = String::from_utf8_lossy(&response[..split]).to_ascii_lowercase();
    let body = &response[split + 4..];
    let status = head.split_whitespace().nth(1).unwrap_or("");
    if status != "200" {
        let text = String::from_utf8_lossy(body);
        return Err(bad(&format!("HTTP {status}: {}", text.trim().chars().take(300).collect::<String>())));
    }
    if !head.contains("transfer-encoding: chunked") {
        return Ok(body.to_vec());
    }
    let mut out = Vec::new();
    let mut rest = body;
    loop {
        let line_end = rest.windows(2).position(|w| w == b"\r\n").ok_or_else(|| bad("bad chunk"))?;
        let size_str = String::from_utf8_lossy(&rest[..line_end]);
        let size = usize::from_str_radix(size_str.split(';').next().unwrap_or("").trim(), 16).map_err(|_| bad("bad chunk size"))?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if rest.len() < size {
            return Err(bad("short chunk"));
        }
        out.extend_from_slice(&rest[..size]);
        rest = rest.get(size + 2..).unwrap_or(&[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_chunked_bodies() {
        assert_eq!(http_body(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").unwrap(), b"{}");
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n{\"a\r\n2\r\n\"}\r\n0\r\n\r\n";
        assert_eq!(http_body(chunked).unwrap(), b"{\"a\"}");
        assert!(http_body(b"HTTP/1.1 500 Internal\r\n\r\noops").is_err());
    }
}
