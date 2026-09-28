"""Protocol v2 for test scripts: frames, JSON-RPC and log lines (docs/protocol.md).

Mirrors main/link_frame.c and host/dualeye-core/src/protocol.rs. Needs pyserial.
"""

import json
import sys
import time

try:
    import serial
    from serial.tools import list_ports
except ImportError:
    sys.exit("pyserial missing: pip install pyserial (or run from the ESP-IDF Python env)")

ESPRESSIF_VID = 0x303A
CTRL, METRICS, LOG = 0, 1, 2
MAX_PAYLOAD = 4096


def crc16(data, crc=0xFFFF):
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF if crc & 0x8000 else (crc << 1) & 0xFFFF
    return crc


def cobs_encode(data):
    out = bytearray([0])
    code_at, run = 0, 1
    for byte in data:
        if byte:
            out.append(byte)
            run += 1
        if not byte or run == 0xFF:
            out[code_at] = run
            code_at, run = len(out), 1
            out.append(0)
    out[code_at] = run
    return bytes(out)


def cobs_decode(data):
    out, i = bytearray(), 0
    while i < len(data):
        code = data[i]
        i += 1
        if code == 0 or i + code - 1 > len(data):
            return None
        out += data[i:i + code - 1]
        i += code - 1
        if code != 0xFF and i < len(data):
            out.append(0)
    return bytes(out)


def encode(chan, payload):
    body = bytes([chan]) + len(payload).to_bytes(2, "little") + payload
    return b"\0" + cobs_encode(body + crc16(body).to_bytes(2, "little")) + b"\0"


def decode(piece):
    body = cobs_decode(piece)
    if body is None or len(body) < 5:
        return None
    head, crc = body[:-2], body[-2:]
    n = int.from_bytes(head[1:3], "little")
    if n != len(head) - 3 or crc16(head) != int.from_bytes(crc, "little"):
        return None
    return head[0], head[3:]


def find_port():
    ports = [p.device for p in list_ports.comports()
             if p.vid == ESPRESSIF_VID and not p.device.startswith("/dev/tty.")]
    if len(ports) != 1:
        sys.exit(f"expected one DualEye board, found {ports or 'none'}; pass --port")
    return ports[0]


class Link:
    """Open the board's port without resetting it and talk protocol v2."""

    def __init__(self, name=None):
        self.port = serial.Serial(name or find_port(), 115200, timeout=0.1)
        # The OS raised DTR and RTS on open; RTS high with DTR low resets the chip.
        self.port.rts = False
        self.port.dtr = False
        self.buf = bytearray()
        self.next_id = 1
        self.pending = []  # (chan, payload) received while waiting for something else

    def send(self, chan, payload):
        self.port.write(encode(chan, payload))

    def frames(self, timeout):
        """Yield (chan, payload) until `timeout` seconds pass without a frame.
        Raw console text comes out as (None, line)."""
        while self.pending:
            yield self.pending.pop(0)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            data = self.port.read(4096)
            for byte in data:
                if byte:
                    self.buf.append(byte)
                    continue
                if self.buf:
                    piece, self.buf = bytes(self.buf), bytearray()
                    frame = decode(piece)
                    if frame is not None:
                        deadline = time.monotonic() + timeout
                        yield frame
                    else:
                        for line in piece.decode(errors="replace").splitlines():
                            if line.strip():
                                yield None, line.encode()

    def call(self, method, params=None, timeout=2.0):
        """JSON-RPC request. Other frames that arrive meanwhile are kept for frames()."""
        rid = self.next_id
        self.next_id += 1
        msg = {"jsonrpc": "2.0", "id": rid, "method": method, "params": params or {}}
        self.send(CTRL, json.dumps(msg).encode())
        other = []
        try:
            for chan, payload in self.frames(timeout):
                if chan == CTRL:
                    reply = json.loads(payload)
                    if reply.get("id") == rid:
                        if "error" in reply:
                            raise RuntimeError(f"{method}: {reply['error'].get('message')}")
                        return reply["result"]
                    if "id" in reply:
                        continue  # the answer to a call given up on
                other.append((chan, payload))
            raise TimeoutError(f"no reply to {method}")
        finally:
            # Kept for the next frames(); not re-read while this call waits.
            self.pending.extend(other)

    def hello(self, total=5.0):
        """Handshake, retried while the board boots."""
        deadline = time.monotonic() + total
        while True:
            try:
                return self.call("hello", {"protocol": 2, "client": "dualeye_link.py"}, timeout=0.5)
            except TimeoutError:
                if time.monotonic() > deadline:
                    raise
