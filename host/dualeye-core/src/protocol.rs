//! Protocol v2 framing, shared with `main/link_frame.c` (see `docs/protocol.md`):
//! `0x00 | COBS(chan | len u16 LE | payload | crc16 LE) | 0x00`.

/// Protocol version the `hello` handshake announces.
pub const PROTOCOL: u32 = 2;
pub const MAX_PAYLOAD: usize = 4096;
const OVERHEAD: usize = 5;
/// Longest piece between delimiters a valid frame can be.
const MAX_ENCODED: usize = MAX_PAYLOAD + OVERHEAD + (MAX_PAYLOAD + OVERHEAD) / 254 + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Channel {
    Ctrl = 0,
    Metrics = 1,
    Log = 2,
    AudioUp = 3,
    AudioDown = 4,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Raw channel byte: receivers ignore channels they don't know.
    pub channel: u8,
    pub payload: Vec<u8>,
}

/// What the stream between two delimiters turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chunk {
    Frame(Frame),
    /// Plain text from outside the framing: ROM, bootloader, panic handler,
    /// or firmware that predates protocol v2.
    Text(String),
    /// Neither: a damaged frame or line noise.
    Bad,
}

/// CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF).
pub fn crc16(data: &[u8]) -> u16 {
    crc16_update(0xFFFF, data)
}

fn crc16_update(mut crc: u16, data: &[u8]) -> u16 {
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

fn cobs_encode(data: &[u8], out: &mut Vec<u8>) {
    let mut code_at = out.len();
    out.push(0);
    let mut run = 1u8;
    for &byte in data {
        if byte != 0 {
            out.push(byte);
            run += 1;
        }
        if byte == 0 || run == 0xFF {
            out[code_at] = run;
            code_at = out.len();
            out.push(0);
            run = 1;
        }
    }
    out[code_at] = run;
}

fn cobs_decode(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        let code = data[i] as usize;
        i += 1;
        if code == 0 || i + code - 1 > data.len() {
            return None;
        }
        out.extend_from_slice(&data[i..i + code - 1]);
        i += code - 1;
        if code != 0xFF && i < data.len() {
            out.push(0);
        }
    }
    Some(out)
}

/// One frame, delimiters included, ready to write.
pub fn encode(channel: Channel, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() <= MAX_PAYLOAD, "payload of {} bytes is over {MAX_PAYLOAD}", payload.len());
    let len = payload.len() as u16;
    let mut body = Vec::with_capacity(payload.len() + OVERHEAD);
    body.push(channel as u8);
    body.extend_from_slice(&len.to_le_bytes());
    body.extend_from_slice(payload);
    let crc = crc16(&body);
    body.extend_from_slice(&crc.to_le_bytes());

    let mut out = Vec::with_capacity(body.len() + body.len() / 254 + 3);
    out.push(0);
    cobs_encode(&body, &mut out);
    out.push(0);
    out
}

/// Decode one piece of the stream between two delimiters.
pub fn decode(piece: &[u8]) -> Option<Frame> {
    let body = cobs_decode(piece)?;
    if body.len() < OVERHEAD {
        return None;
    }
    let (head, crc) = body.split_at(body.len() - 2);
    let len = u16::from_le_bytes([head[1], head[2]]) as usize;
    if len != head.len() - 3 || len > MAX_PAYLOAD || crc16(head) != u16::from_le_bytes([crc[0], crc[1]]) {
        return None;
    }
    Some(Frame { channel: head[0], payload: head[3..].to_vec() })
}

/// A piece that isn't a frame is text when it is mostly printable. Bytes of a
/// frame cut short (a panic in the middle of one) are dropped from it.
fn as_text(piece: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(piece);
    let printable = |c: char| !c.is_control() || matches!(c, '\n' | '\r' | '\t' | '\x1b');
    let good = text.chars().filter(|&c| printable(c) && c != char::REPLACEMENT_CHARACTER).count();
    let total = text.chars().count();
    (total > 0 && good * 10 >= total * 9).then(|| {
        text.chars().filter(|&c| printable(c) && c != char::REPLACEMENT_CHARACTER).collect()
    })
}

/// Splits the byte stream from the board into [`Chunk`]s.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
    overflow: bool,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8], mut out: impl FnMut(Chunk)) {
        for &byte in bytes {
            if byte != 0 {
                if self.buf.len() < MAX_ENCODED {
                    self.buf.push(byte);
                } else {
                    // No frame is this long: text without delimiters, pass it on.
                    self.overflow = true;
                    self.emit(&mut out);
                    self.buf.push(byte);
                }
                continue;
            }
            self.emit(&mut out);
            self.overflow = false;
        }
    }

    /// Nothing more arrived for a while. Firmware before protocol v2 never
    /// sends a delimiter, so text still waiting for one is passed on here if
    /// it ends a line. A frame is written in one go, so it is never left
    /// half-way when the line goes quiet.
    pub fn idle(&mut self, mut out: impl FnMut(Chunk)) {
        if self.buf.last() == Some(&b'\n') {
            self.emit(&mut out);
        }
    }

    fn emit(&mut self, out: &mut impl FnMut(Chunk)) {
        if self.buf.is_empty() {
            return;
        }
        let chunk = match (!self.overflow).then(|| decode(&self.buf)).flatten() {
            Some(frame) => Chunk::Frame(frame),
            None => as_text(&self.buf).map_or(Chunk::Bad, Chunk::Text),
        };
        self.buf.clear();
        out(chunk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(decoder: &mut Decoder, bytes: &[u8]) -> Vec<Chunk> {
        let mut out = Vec::new();
        decoder.push(bytes, |c| out.push(c));
        out
    }

    #[test]
    fn crc_check_value() {
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn known_frame_bytes() {
        // What main/link_frame.c writes for the same frame.
        assert_eq!(encode(Channel::Log, b"hi"), [0x00, 0x03, 0x02, 0x02, 0x05, b'h', b'i', 0xEB, 0xC7, 0x00]);
    }

    #[test]
    fn round_trips_any_payload() {
        let long_run: Vec<u8> = (0..600).map(|i| (i % 255 + 1) as u8).collect();
        let zeros = vec![0u8; 300];
        let mixed: Vec<u8> = (0..=MAX_PAYLOAD).map(|i| (i * 7 % 256) as u8).take(MAX_PAYLOAD).collect();
        for payload in [&b""[..], b"{}", &long_run, &zeros, &mixed, &long_run[..254], &long_run[..253]] {
            let wire = encode(Channel::Ctrl, payload);
            assert_eq!(wire.iter().filter(|&&b| b == 0).count(), 2, "only the delimiters are zero");
            assert_eq!(decode(&wire[1..wire.len() - 1]), Some(Frame { channel: 0, payload: payload.to_vec() }));
        }
    }

    #[test]
    fn rejects_damage() {
        let mut wire = encode(Channel::Metrics, b"{\"v\":2}");
        let n = wire.len();
        wire[5] ^= 0x01;
        assert_eq!(decode(&wire[1..n - 1]), None);
        assert_eq!(decode(&[0x01]), None);
        assert_eq!(decode(&[0x05, 1, 2]), None);
    }

    #[test]
    fn splits_frames_and_text_in_a_stream() {
        let mut stream = b"ESP-ROM:esp32s3-20210327\r\n".to_vec();
        stream.extend(encode(Channel::Log, b"I (10) link: up"));
        stream.extend(encode(Channel::Ctrl, b"{}"));
        stream.extend(b"Guru Meditation Error\r\n");
        stream.extend(encode(Channel::Log, b"after"));
        let mut decoder = Decoder::new();
        // Byte by byte: a read may end anywhere.
        let mut got = Vec::new();
        for b in &stream {
            got.extend(chunks(&mut decoder, std::slice::from_ref(b)));
        }
        assert_eq!(
            got,
            vec![
                Chunk::Text("ESP-ROM:esp32s3-20210327\r\n".into()),
                Chunk::Frame(Frame { channel: 2, payload: b"I (10) link: up".to_vec() }),
                Chunk::Frame(Frame { channel: 0, payload: b"{}".to_vec() }),
                Chunk::Text("Guru Meditation Error\r\n".into()),
                Chunk::Frame(Frame { channel: 2, payload: b"after".to_vec() }),
            ]
        );
    }

    #[test]
    fn a_damaged_frame_does_not_lose_the_next() {
        let mut first = encode(Channel::Log, b"one");
        first[3] ^= 0x40;
        let mut stream = first[..first.len() - 1].to_vec();
        stream.extend(encode(Channel::Log, b"two"));
        let got = chunks(&mut Decoder::new(), &stream);
        assert_eq!(got.last(), Some(&Chunk::Frame(Frame { channel: 2, payload: b"two".to_vec() })));
        assert!(!got.iter().any(|c| matches!(c, Chunk::Frame(f) if f.payload == b"one")));
    }

    #[test]
    fn legacy_text_comes_out_when_the_line_goes_quiet() {
        let mut decoder = Decoder::new();
        assert!(chunks(&mut decoder, b"{\"dualeye\":\"0.3.0\",\"idf\":\"v6.1\"}\n").is_empty());
        let mut got = Vec::new();
        decoder.idle(|c| got.push(c));
        assert_eq!(got, vec![Chunk::Text("{\"dualeye\":\"0.3.0\",\"idf\":\"v6.1\"}\n".into())]);
    }

    #[test]
    fn binary_noise_is_not_text() {
        let got = chunks(&mut Decoder::new(), &[0x00, 0x9F, 0x01, 0x02, 0xFE, 0x80, 0x00]);
        assert_eq!(got, vec![Chunk::Bad]);
    }
}
