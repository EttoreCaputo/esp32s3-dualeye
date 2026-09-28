# USB protocol v2

The board and the host talk over the ESP32-S3's USB Serial/JTAG port (`303a:1001`). Since firmware 0.4.0 the link carries **framed, multiplexed, binary-safe** messages in both directions. Firmware up to 0.3 spoke protocol 1 (newline JSON, host → board only) and must be reflashed.

Implementations: `main/link.c` and `main/link_frame.c` (board), `host/dualeye-core/src/protocol.rs` and `link.rs` (host), `tools/dualeye_link.py` (test scripts).

## Frames

```
0x00 | COBS( chan | len | payload | crc ) | 0x00
```

| Field | Size | Meaning |
|-------|------|---------|
| `chan` | u8 | Channel, see below |
| `len` | u16 LE | Payload length, at most **4096** |
| `payload` | `len` bytes | Channel-specific |
| `crc` | u16 LE | CRC-16/CCITT-FALSE (poly `0x1021`, init `0xFFFF`, no reflection, no final XOR) over `chan`, `len` and `payload` |

The header, payload and CRC are [COBS](https://en.wikipedia.org/wiki/Consistent_Overhead_Byte_Stuffing)-encoded, so the only `0x00` bytes on the wire are delimiters. Each frame is written with a leading **and** a trailing delimiter, so it resynchronises right after any stray bytes. Receivers split the stream on `0x00`, skip empty pieces, and drop pieces that fail COBS decoding, the length check or the CRC.

### Bytes outside frames

The ROM, the second-stage bootloader, early startup and the panic handler write plain text to the same port and don't know about frames. A piece that isn't a valid frame but is mostly printable text is shown as **raw console text**: the host's board console keeps showing boot messages and crash backtraces. Anything else counts as a bad frame and is dropped.

### Flow control

There is none. The board drops input it has no room for (the USB Serial/JTAG driver doesn't hold the host back), so the host paces what it sends. Measured on firmware 0.4.0 at 160 MHz: snapshots are all taken at 1000 per second (250 KB/s), and 640-byte audio frames at 635 KB/s, against 32 KB/s for real-time audio. A burst of hundreds of JSON-RPC requests written back to back does lose some; send one at a time and wait for its answer. Dropped input shows up as `rx_bad` in `get_state`. In the other direction the board waits up to 200 ms for the host to read before dropping a frame, and drops log lines at once while the host isn't reading (`tx_dropped`).

## Channels

| # | Name | Direction | Payload |
|---|------|-----------|---------|
| 0 | `ctrl` | both | One JSON-RPC 2.0 message (UTF-8 JSON) |
| 1 | `metrics` | host → board | One sensor snapshot (JSON, below) |
| 2 | `log` | board → host | One log line, UTF-8, no trailing newline |
| 3 | `audio_up` | board → host | Reserved (M4): PCM s16le, 16 kHz, mono |
| 4 | `audio_down` | host → board | Reserved (M5): PCM s16le, 16 kHz, mono |

Receivers ignore channels they don't know.

### `ctrl`

[JSON-RPC 2.0](https://www.jsonrpc.org/specification). The host sends requests with a numeric `id`; the board answers each one with a response carrying the same `id`. The board sends notifications (no `id`) and never requests. Batches are not supported.

| Method | Params | Result |
|--------|--------|--------|
| `hello` | `{"protocol":2,"client":"dualeye-cli/0.1.0"}` | The board's [identity](#handshake) |
| `tools/list` | none | `{"tools":[Tool, …]}` |
| `tools/call` | `{"name":"set_face","arguments":{…}}` | `CallToolResult` |
| `debug/audio` | `{"cmd":"tone 440 500"}` | `{}`; the M0 audio self-test, its output comes as `log` lines (see `main/audio_selftest.h`) |

`Tool` and `CallToolResult` have the shapes of the [MCP](https://modelcontextprotocol.io/specification/2025-06-18/server/tools) `tools/list` and `tools/call` results (`name`, `description`, `inputSchema`; `content`, `structuredContent`, `isError`), so the host's MCP server (M2) can pass them through unchanged. A tool that runs but fails (bad argument, out of range) returns `isError: true` with the reason as text. Protocol errors use the standard JSON-RPC codes: `-32700` parse error, `-32600` invalid request, `-32601` unknown method, `-32602` invalid params (including an unknown tool).

Notifications from the board:

| Method | Params | When |
|--------|--------|------|
| `ready` | Same as the `hello` result | Once at boot, when the link is up. A host that sees it knows the board rebooted |

### `metrics`

The snapshot the host sends about once a second. Its shape is protocol 1's line without the trailing newline and without `face` and `rot`, which are now board state, set with tools:

```json
{"v":2,"ts":1700000000,"cpu":{"temp_c":36.3,"load_pct":1.8,"clock_mhz":1572,"power_w":14.6,"mem":{"used_mb":12288,"total_mb":31744}},"gpu":{"temp_c":31.0},"fans":[{"id":"cpu","rpm":3770}],"claude":{"tok":1234567,"today":4500000,"left_min":133,"s_pct":42.0,"state":"work","model":"OPUS 5.5"}}
```

The board ignores a snapshot with neither temperature, and marks its data stale 3 s after the last one.

### `log`

Every `ESP_LOG*` line after the link starts, one line per frame. The level letter and timestamp are part of the text (`I (5120) link: …`), as on a plain serial console.

## Handshake

1. The host opens the port and sends `hello`. The OS raises DTR and RTS on open, and the USB Serial/JTAG resets the chip whenever RTS is high while DTR is low: lower **RTS first, then DTR**. Opening the port may have reset the board, so the host retries every 500 ms for up to 5 s.
2. The board answers:

   ```json
   {"protocol":2,"firmware":"0.4.0","idf":"v6.1","board":"dualeye","max_payload":4096,"channels":["ctrl","metrics","log"],"capabilities":["tools"]}
   ```

3. The host pushes its settings (faces, rotation) with `tools/call`, then streams `metrics`.

When the host sees `ready` it repeats step 3. A board that never answers `hello` but prints protocol 1's version line (`{"dualeye":"0.3.0",…}`, in answer to the text line `?version`) is reported as that version, so the app can offer to update it.

## Board tools

Screens are named `left` (the CPU screen) and `right` (the GPU screen); `both` is the default where a tool takes `screen`. Faces, rotation and brightness are kept in NVS and survive a reboot.

| Tool | Arguments | Effect |
|------|-----------|--------|
| `set_face` | `face`: `classic` · `rings` · `plus` · `bar` · `claude` · `clawd`; `screen` | Switch the watch face |
| `set_rotation` | `degrees`: 0 · 90 · 180 · 270; `screen` | Turn the screen clockwise on top of the DualEye mounting |
| `set_brightness` | `percent`: 0–100; `screen` | Backlight level (0 turns it off) |
| `show_text` | `text` (up to 120 characters, ASCII); `screen`; `seconds`: 1–30, default 4 | Show a message over the face, then hide it |
| `get_state` | none | Firmware, uptime, metrics state, each screen's face, rotation and brightness, free memory, link counters (as `structuredContent`) |

Example:

```json
→ {"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"set_face","arguments":{"screen":"left","face":"rings"}}}
← {"jsonrpc":"2.0","id":7,"result":{"content":[{"type":"text","text":"left: rings"}],"isError":false}}
```

From the CLI: `dualeye tools`, `dualeye call set_face --screen left --face rings`, or `dualeye call set_face '{"screen":"left","face":"rings"}'`. The CLI opens the port itself, so quit the app first (until M2).
