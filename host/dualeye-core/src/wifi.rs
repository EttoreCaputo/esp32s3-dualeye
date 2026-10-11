//! The board on WiFi (firmware 1.4.3, `docs/protocol.md`, "WiFi"): the
//! boards paired with this computer, finding them with mDNS, and the TCP
//! session, where every frame is sealed with AES-256-GCM after both ends
//! proved they hold the pairing key the board gave over USB.

use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;

use crate::protocol::{self, Channel, Chunk, Decoder, Frame};

/// The board's TCP port.
pub const PORT: u16 = 7340;
/// What the board announces itself as.
pub const SERVICE: &str = "_dualeye._tcp.local.";
/// How a port string names a board on WiFi: `wifi` (the paired one) or `wifi:<id>`.
pub const SCHEME: &str = "wifi";
pub const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 16;
/// The board's answer to the challenge, and to `auth`.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(4);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// How long [`find`] listens for the board's mDNS answer.
pub const DISCOVER_TIMEOUT: Duration = Duration::from_secs(2);

/// A board this computer may talk to on WiFi: the key it gave over USB.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paired {
    /// The board's id: its WiFi MAC, 12 hex digits.
    pub id: String,
    /// The pairing key, hex.
    pub key: String,
    /// The network it was set up on, for the app to show.
    #[serde(default)]
    pub ssid: Option<String>,
    /// Where it was last found, tried when mDNS doesn't answer.
    #[serde(default)]
    pub last_addr: Option<String>,
}

impl Paired {
    pub fn key_bytes(&self) -> Option<[u8; KEY_BYTES]> {
        unhex(&self.key)?.try_into().ok()
    }
}

/// Where the paired boards are kept, readable by this user only.
pub fn paired_file() -> Option<PathBuf> {
    crate::claude::data_dir().map(|d| d.join("wifi-boards.json"))
}

pub fn load_paired() -> Vec<Paired> {
    paired_file().and_then(|f| fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_paired(boards: &[Paired]) -> io::Result<()> {
    let file = paired_file().ok_or_else(|| io::Error::other("no data folder"))?;
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(boards).map_err(io::Error::other)?;
    let tmp = file.with_extension("json.tmp");
    crate::cloud::write_private(&tmp, &json)?;
    fs::rename(tmp, file)
}

/// Keep the key a board gave (`wifi/set`, `wifi/pair`), replacing its old one.
pub fn remember(id: &str, key: &str, ssid: Option<&str>) -> io::Result<()> {
    if unhex(key).is_none_or(|k| k.len() != KEY_BYTES) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a pairing key"));
    }
    let mut boards = load_paired();
    let old = boards.iter().position(|b| b.id == id).map(|i| boards.remove(i));
    let ssid = ssid.map(str::to_string).or_else(|| old.as_ref().and_then(|o| o.ssid.clone()));
    let last_addr = old.and_then(|o| o.last_addr);
    // The latest one first: it's the one `wifi` means.
    boards.insert(0, Paired { id: id.to_string(), key: key.to_string(), ssid, last_addr });
    save_paired(&boards)
}

/// Forget board `id`: this computer can't reach it on WiFi any more.
pub fn forget(id: &str) -> io::Result<()> {
    let mut boards = load_paired();
    boards.retain(|b| b.id != id);
    save_paired(&boards)
}

fn remember_addr(id: &str, addr: SocketAddr) {
    let mut boards = load_paired();
    if let Some(b) = boards.iter_mut().find(|b| b.id == id)
        && b.last_addr.as_deref() != Some(&addr.to_string())
    {
        b.last_addr = Some(addr.to_string());
        let _ = save_paired(&boards);
    }
}

/// `wifi`, `wifi:<id>`: the paired board it names, if any.
pub fn target(port: &str) -> Option<Option<&str>> {
    match port.strip_prefix(SCHEME)? {
        "" => Some(None),
        rest => rest.strip_prefix(':').map(Some),
    }
}

/// The port string for the board paired last, if there is one.
pub fn default_target() -> Option<String> {
    load_paired().first().map(|b| format!("{SCHEME}:{}", b.id))
}

/// A board that answered on the network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Found {
    pub id: String,
    pub addr: SocketAddr,
    pub host: String,
    pub firmware: Option<String>,
}

/// The boards on the network, as mDNS finds them within `timeout`.
pub fn discover(timeout: Duration, wanted: Option<&str>) -> Vec<Found> {
    let Ok(daemon) = mdns_sd::ServiceDaemon::new() else { return Vec::new() };
    let Ok(rx) = daemon.browse(SERVICE) else {
        let _ = daemon.shutdown();
        return Vec::new();
    };
    let deadline = Instant::now() + timeout;
    let mut found: Vec<Found> = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let Ok(event) = rx.recv_timeout(left) else { break };
        let mdns_sd::ServiceEvent::ServiceResolved(info) = event else { continue };
        let Some(id) = info.get_property_val_str("id").map(str::to_string) else { continue };
        // IPv4 first: the board has no IPv6 address worth trying.
        let mut addrs: Vec<_> = info.get_addresses().iter().map(|a| a.to_ip_addr()).collect();
        addrs.sort_by_key(|a| !a.is_ipv4());
        let Some(ip) = addrs.first() else { continue };
        if found.iter().any(|f| f.id == id) {
            continue;
        }
        let done = wanted == Some(id.as_str());
        found.push(Found {
            id,
            addr: SocketAddr::new(*ip, info.get_port()),
            host: info.get_hostname().trim_end_matches('.').to_string(),
            firmware: info.get_property_val_str("fw").map(str::to_string),
        });
        if done {
            break;
        }
    }
    let _ = daemon.stop_browse(SERVICE);
    let _ = daemon.shutdown();
    found
}

/// Where paired board `id` is: mDNS, else where it was last.
fn locate(board: &Paired) -> io::Result<Vec<SocketAddr>> {
    let mut addrs: Vec<SocketAddr> = discover(DISCOVER_TIMEOUT, Some(&board.id)).into_iter().filter(|f| f.id == board.id).map(|f| f.addr).collect();
    if let Some(last) = board.last_addr.as_deref().and_then(|a| a.to_socket_addrs().ok()).into_iter().flatten().next()
        && !addrs.contains(&last)
    {
        addrs.push(last);
    }
    if addrs.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "board not found on the network"));
    }
    Ok(addrs)
}

/// One direction of a session: AES-256-GCM, its own key, a frame counter.
pub struct Cipher {
    aead: Aes256Gcm,
    count: u64,
}

impl Cipher {
    fn new(key: &[u8; 32]) -> Self {
        Self { aead: Aes256Gcm::new_from_slice(key).expect("32-byte key"), count: 0 }
    }

    fn nonce(&self) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[4..].copy_from_slice(&self.count.to_le_bytes());
        n
    }

    /// Encrypt one payload, the channel as associated data.
    pub fn seal(&mut self, channel: u8, payload: &[u8]) -> Vec<u8> {
        let nonce = self.nonce();
        let sealed = self.aead.encrypt(&Nonce::from(nonce), Payload { msg: payload, aad: &[channel] }).expect("AES-GCM encrypt");
        self.count += 1;
        sealed
    }

    /// Decrypt and check one payload; `None` if it was tampered with or out of step.
    pub fn open(&mut self, channel: u8, sealed: &[u8]) -> Option<Vec<u8>> {
        let nonce = self.nonce();
        let plain = self.aead.decrypt(&Nonce::from(nonce), Payload { msg: sealed, aad: &[channel] }).ok()?;
        self.count += 1;
        Some(plain)
    }
}

/// HMAC-SHA256(key, label || nb || nh): the proofs and the session keys.
pub fn derive(key: &[u8; KEY_BYTES], label: &str, nb: &[u8], nh: &[u8]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("any key length");
    mac.update(label.as_bytes());
    mac.update(nb);
    mac.update(nh);
    mac.finalize().into_bytes().into()
}

/// A session that passed the handshake: the stream, a cipher each way, and
/// what was read past the handshake.
pub struct Session {
    pub stream: TcpStream,
    pub addr: SocketAddr,
    pub tx: Cipher,
    pub rx: Cipher,
    pub decoder: Decoder,
    pub pending: Vec<Frame>,
}

/// Connect to the paired board `port` names (`wifi`, `wifi:<id>`) and pair.
pub fn open(port: &str) -> io::Result<Session> {
    let wanted = target(port).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{port}: not a WiFi board")))?;
    let boards = load_paired();
    let board = match wanted {
        Some(id) => boards.into_iter().find(|b| b.id == id),
        None => boards.into_iter().next(),
    }
    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no board paired for WiFi: set its WiFi up over USB first"))?;
    let key = board.key_bytes().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "the pairing key is damaged: pair again over USB"))?;
    let mut last = io::Error::new(io::ErrorKind::NotFound, "board not found on the network");
    for addr in locate(&board)? {
        match connect(addr, &key, Some(&board.id)) {
            Ok(session) => {
                remember_addr(&board.id, addr);
                return Ok(session);
            }
            // Refused: no point trying it elsewhere.
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Err(e),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Connect to `addr` and run the handshake: the board's challenge, our
/// `auth`, its proof. `id`: the board it must be.
pub fn connect(addr: SocketAddr, key: &[u8; KEY_BYTES], id: Option<&str>) -> io::Result<Session> {
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_millis(200)))?;
    let mut decoder = Decoder::new();
    let mut frames: Vec<Frame> = Vec::new();
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;

    let challenge = next_ctrl(&mut stream, &mut decoder, &mut frames, deadline, |m| m["method"] == "auth/challenge")?;
    let params = &challenge["params"];
    if let Some(id) = id
        && params["id"].as_str() != Some(id)
    {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{addr} is another board ({})", params["id"].as_str().unwrap_or("?"))));
    }
    let nb = params["nonce"].as_str().and_then(unhex).filter(|n| n.len() == NONCE_BYTES).ok_or_else(|| bad("challenge without a nonce"))?;
    let mut nh = [0u8; NONCE_BYTES];
    getrandom::fill(&mut nh).map_err(|e| io::Error::other(e.to_string()))?;
    let proof = derive(key, "dualeye host", &nb, &nh);
    let auth = json!({"jsonrpc": "2.0", "id": 0, "method": "auth", "params": {"nonce": hex(&nh), "proof": hex(&proof)}});
    stream.write_all(&protocol::encode(Channel::Ctrl, auth.to_string().as_bytes()))?;

    let reply = next_ctrl(&mut stream, &mut decoder, &mut frames, deadline, |m| m["id"] == 0)?;
    if let Some(err) = reply.get("error") {
        let message = err["message"].as_str().unwrap_or("refused").to_string();
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, message));
    }
    let theirs = reply["result"]["proof"].as_str().and_then(unhex).ok_or_else(|| bad("no proof in the answer"))?;
    if theirs != derive(key, "dualeye board", &nb, &nh) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the board doesn't hold our key"));
    }
    let tx = Cipher::new(&derive(key, "dualeye h2b", &nb, &nh));
    let rx = Cipher::new(&derive(key, "dualeye b2h", &nb, &nh));
    Ok(Session { stream, addr, tx, rx, decoder, pending: frames })
}

fn bad(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("WiFi handshake: {why}"))
}

/// Read until a ctrl frame `pick` likes; later frames (sealed already) are
/// kept in `rest`.
fn next_ctrl(stream: &mut TcpStream, decoder: &mut Decoder, rest: &mut Vec<Frame>, deadline: Instant, pick: impl Fn(&Value) -> bool) -> io::Result<Value> {
    let mut buf = [0u8; 1024];
    loop {
        let mut got = None;
        let mut i = 0;
        while i < rest.len() {
            let frame = &rest[i];
            if got.is_none()
                && frame.channel == Channel::Ctrl as u8
                && let Ok(msg) = serde_json::from_slice::<Value>(&frame.payload)
                && pick(&msg)
            {
                got = Some(msg);
                rest.drain(..=i);
                break;
            }
            i += 1;
        }
        if let Some(msg) = got {
            return Ok(msg);
        }
        if Instant::now() > deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "the board didn't answer the WiFi handshake"));
        }
        match stream.read(&mut buf) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "the board closed the connection")),
            Ok(n) => decoder.push(&buf[..n], |c| {
                if let Chunk::Frame(f) = c {
                    rest.push(f);
                }
            }),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {}
            Err(e) => return Err(e),
        }
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn targets() {
        assert_eq!(target("wifi"), Some(None));
        assert_eq!(target("wifi:a1b2c3d4e5f6"), Some(Some("a1b2c3d4e5f6")));
        assert_eq!(target("/dev/cu.usbmodem2101"), None);
        assert_eq!(target("wifix"), None);
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(unhex(&hex(&[0, 1, 0xab, 0xff])), Some(vec![0, 1, 0xab, 0xff]));
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
    }

    #[test]
    fn sealed_both_ways_in_step() {
        let key = [7u8; 32];
        let mut a = Cipher::new(&key);
        let mut b = Cipher::new(&key);
        for i in 0..3u8 {
            let sealed = a.seal(1, &[i; 10]);
            assert_eq!(sealed.len(), 10 + protocol::SEAL_OVERHEAD);
            assert_eq!(b.open(1, &sealed), Some(vec![i; 10]));
        }
        // Another channel, a flipped bit or a replay: refused.
        let sealed = a.seal(0, b"x");
        assert_eq!(b.clone_count().open(1, &sealed), None);
        let mut flipped = sealed.clone();
        flipped[0] ^= 1;
        assert_eq!(b.clone_count().open(0, &flipped), None);
        assert_eq!(b.open(0, &sealed), Some(b"x".to_vec()));
        assert_eq!(b.open(0, &sealed), None);
    }

    impl Cipher {
        fn clone_count(&self) -> Cipher {
            Cipher { aead: self.aead.clone(), count: self.count }
        }
    }

    /// The board's side of the handshake, as main/link.c does it.
    fn fake_board(listener: TcpListener, key: [u8; 32], id: &'static str) {
        let (mut s, _) = listener.accept().unwrap();
        let nb = [9u8; NONCE_BYTES];
        let challenge = json!({"jsonrpc": "2.0", "method": "auth/challenge", "params": {"nonce": hex(&nb), "id": id, "protocol": 2}});
        s.write_all(&protocol::encode(Channel::Ctrl, challenge.to_string().as_bytes())).unwrap();
        let mut dec = Decoder::new();
        let mut buf = [0u8; 512];
        let mut auth = None;
        while auth.is_none() {
            let n = s.read(&mut buf).unwrap_or(0);
            if n == 0 {
                return;
            }
            dec.push(&buf[..n], |c| {
                if let Chunk::Frame(f) = c {
                    auth = serde_json::from_slice::<Value>(&f.payload).ok();
                }
            });
        }
        let auth = auth.unwrap();
        let nh = unhex(auth["params"]["nonce"].as_str().unwrap()).unwrap();
        let ok = unhex(auth["params"]["proof"].as_str().unwrap()).unwrap() == derive(&key, "dualeye host", &nb, &nh);
        let reply = if ok {
            json!({"jsonrpc": "2.0", "id": 0, "result": {"proof": hex(&derive(&key, "dualeye board", &nb, &nh))}})
        } else {
            json!({"jsonrpc": "2.0", "id": 0, "error": {"code": -32001, "message": "not paired with this board"}})
        };
        s.write_all(&protocol::encode(Channel::Ctrl, reply.to_string().as_bytes())).unwrap();
        if ok {
            // A sealed frame right behind the answer must not get lost.
            let mut tx = Cipher::new(&derive(&key, "dualeye b2h", &nb, &nh));
            let sealed = tx.seal(Channel::Log as u8, b"hi");
            s.write_all(&protocol::encode_raw(Channel::Log as u8, &sealed)).unwrap();
        }
    }

    #[test]
    fn handshake_with_the_right_key() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let board = thread::spawn(move || fake_board(listener, [3; 32], "a1b2c3d4e5f6"));
        let mut session = connect(addr, &[3; 32], Some("a1b2c3d4e5f6")).unwrap();
        board.join().unwrap();
        // The log frame may still be on its way: read for it.
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut buf = [0u8; 256];
        while session.pending.is_empty() && Instant::now() < deadline {
            if let Ok(n) = session.stream.read(&mut buf) {
                let pending = &mut session.pending;
                session.decoder.push(&buf[..n], |c| {
                    if let Chunk::Frame(f) = c {
                        pending.push(f);
                    }
                });
            }
        }
        let frame = session.pending.remove(0);
        assert_eq!(session.rx.open(frame.channel, &frame.payload), Some(b"hi".to_vec()));
    }

    #[test]
    fn handshake_with_the_wrong_key() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let board = thread::spawn(move || fake_board(listener, [3; 32], "a1b2c3d4e5f6"));
        let err = connect(addr, &[4; 32], None).err().unwrap();
        board.join().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn handshake_with_another_board() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let board = thread::spawn(move || fake_board(listener, [3; 32], "ffffffffffff"));
        let err = connect(addr, &[3; 32], Some("a1b2c3d4e5f6")).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        board.join().unwrap();
    }
}
