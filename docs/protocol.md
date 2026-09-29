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
| 3 | `audio_up` | board → host | An utterance's audio, see [below](#audio_up) |
| 4 | `audio_down` | host → board | Speech for the speaker, see [below](#audio_down) |

Receivers ignore channels they don't know.

### `ctrl`

[JSON-RPC 2.0](https://www.jsonrpc.org/specification). The host sends requests with a numeric `id`; the board answers each one with a response carrying the same `id`. The board sends notifications (no `id`) and never requests. Batches are not supported.

| Method | Params | Result |
|--------|--------|--------|
| `hello` | `{"protocol":2,"client":"dualeye-cli/0.1.0"}` | The board's [identity](#handshake) |
| `tools/list` | none | `{"tools":[Tool, …]}` |
| `tools/call` | `{"name":"set_face","arguments":{…}}` | `CallToolResult` |
| `voice/state` | `{"state":"thinking"}`: `idle` · `listening` · `thinking` · `speaking` · `error` | `{}`; what the voice overlay shows. For the host's voice pipeline (M4 on), so not a tool. `error` (firmware 1.0) is a red ring with a short two-note sound, for 1.5 s: the host failed, or heard no words. A state other than `idle` goes back to `idle` by itself after 30 s (`listening`: 13 s, `error`: 1.5 s). `-32602` without voice |
| `voice/listen` | none, or `{"follow_up":true}` | `{}`; stream an utterance as if the wake word had been heard (push-to-talk, `trigger` `host`). With `follow_up` (firmware 1.0; the host asks right after a spoken answer) the `trigger` is `follow_up`, it gives up after 4 s without speech, and speech in its first 0.8 s (the echo of the answer) doesn't count. `-32602` without voice or muted |
| `voice/stop` | none | `{}`; end the utterance being streamed (`reason` `host`), if any |
| `audio/stop` | none | `{}`; stop talking: what's buffered from `audio_down` is dropped and the stream ends (`reason` `stopped`) |
| `debug/audio` | `{"cmd":"tone 440 500"}` | `{}`; the M0 audio self-test, its output comes as `log` lines (see `main/audio_selftest.h`) |

`Tool` and `CallToolResult` have the shapes of the [MCP](https://modelcontextprotocol.io/specification/2025-06-18/server/tools) `tools/list` and `tools/call` results (`name`, `description`, `inputSchema`; `content`, `structuredContent`, `isError`), so the host's MCP server (M2) can pass them through unchanged. A tool that runs but fails (bad argument, out of range) returns `isError: true` with the reason as text. Protocol errors use the standard JSON-RPC codes: `-32700` parse error, `-32600` invalid request, `-32601` unknown method, `-32602` invalid params (including an unknown tool).

Notifications from the board:

| Method | Params | When |
|--------|--------|------|
| `ready` | Same as the `hello` result | Once at boot, when the link is up. A host that sees it knows the board rebooted |
| `wake` | `{"word":"Alexa","model":"wn9_alexa","volume_db":-45}` | The wake word was heard (`volume_db`: input level in dBFS). The board then shows `listening` |
| `voice_state` | `{"state":"listening"}` | The voice overlay changed: on the wake word, after a timeout, or after `voice/state` |
| `utterance_start` | `{"id":7,"trigger":"wake","rate":16000,"format":"s16le"}` | The board starts streaming what it hears on `audio_up`: after the wake word (`trigger` `wake`) or `voice/listen` (`host`, or `follow_up`). `id` counts up and wraps at 256 |
| `playback_end` | `{"id":3,"reason":"done","ms":2426,"lost":0,"overflow":0,"underruns":0}` | An `audio_down` stream ended. `reason`: `done` (played to its last frame), `stopped` (`audio/stop`), `replaced` (a stream with another id started), `starved` (no audio for 1.5 s without the last frame: the host went away) or `barge_in` (firmware 1.0: the wake word was said over it; an `utterance_start` follows). `ms` were played, `lost` frames never arrived, `overflow` samples found the buffer full, `underruns` times it ran dry mid-stream |
| `utterance_end` | `{"id":7,"reason":"end_of_speech","ms":3200,"speech":true,"frames":100,"dropped":0}` | The stream ended. `reason`: `end_of_speech` (0.75 s of silence after speech), `no_speech` (none within 5 s, 4 s for a follow-up), `max_length` (12 s), `host` (`voice/stop`, or the wake word changed) or `muted` (`set_mic`, or the self-test took the mic). `frames` were sent, `dropped` of them lost because the host didn't read in time. With `speech` the board shows `thinking` next, otherwise `idle` |

### `metrics`

The snapshot the host sends about once a second. Its shape is protocol 1's line without the trailing newline and without `face` and `rot`, which are now board state, set with tools:

```json
{"v":2,"ts":1700000000,"cpu":{"temp_c":36.3,"load_pct":1.8,"clock_mhz":1572,"power_w":14.6,"mem":{"used_mb":12288,"total_mb":31744}},"gpu":{"temp_c":31.0},"fans":[{"id":"cpu","rpm":3770}],"claude":{"tok":1234567,"today":4500000,"left_min":133,"s_pct":42.0,"state":"work","model":"OPUS 5.5"}}
```

The board ignores a snapshot with neither temperature, and marks its data stale 3 s after the last one.

### `audio_up`

The audio of one utterance, between its `utterance_start` and `utterance_end` notifications: the output of ESP-SR's front end (echo-cancelled mic), 16 kHz mono PCM s16le, 512 samples (32 ms) a frame, about 32 KB/s.

| Offset | Size | Field |
|--------|------|-------|
| 0 | u8 | Utterance `id` |
| 1 | u8 | Flags, 0 for now |
| 2 | u16 LE | Sequence number, from 0 in each utterance |
| 4 | … | PCM |

The board doesn't wait for a host that doesn't read (it drops the frame after 20 ms), so the host fills gaps in the sequence with silence. WakeNet is off while the board streams: the wake word said again mid-sentence doesn't start a new utterance. Implementation: `stream_*` in `main/voice.c`, `host/dualeye-core/src/voice.rs`.

### `audio_down`

Speech for the speaker: 16 kHz mono PCM s16le, with the same 4-byte header as `audio_up`.

| Offset | Size | Field |
|--------|------|-------|
| 0 | u8 | Stream `id`: the host counts up; a frame with a new id ends the stream playing (`replaced`) |
| 1 | u8 | Flags: bit 0 set on the stream's last frame (it may carry no PCM) |
| 2 | u16 LE | Sequence number, from 0 in each stream |
| 4 | … | PCM, up to 2046 samples (the host sends 1024, 64 ms) |

The board buffers about 4 s in PSRAM and starts the speaker once 150 ms are buffered (or the last frame is in), so the host paces the stream: it sends in real time, about 0.5 s ahead of the speaker, since nothing on the link tells it to slow down. When the buffer runs dry before the last frame the board plays silence; after 1.5 s of it the stream ends (`starved`). While a stream plays the board shows `speaking` with the speaker's level on the ring, and goes back to `idle` at the end. Echo cancellation keeps running, so the board's own voice doesn't reach WakeNet: the wake word said over it stops the stream (`barge_in`) and starts a new utterance (firmware 1.0, `CONFIG_DUALEYE_VOICE_BARGE_IN`; without it, and on 0.6, the wake word is ignored while it plays and for 0.3 s after). The host stops sending once it sees the `playback_end`. Frames of a stream that has ended are dropped. At the end the board says `playback_end`. Implementation: `main/playback.c`, `Speaker` in `host/dualeye-core/src/voice.rs`.

### `log`

Every `ESP_LOG*` line after the link starts, one line per frame. The level letter and timestamp are part of the text (`I (5120) link: …`), as on a plain serial console.

## Handshake

1. The host opens the port and sends `hello`. The OS raises DTR and RTS on open, and the USB Serial/JTAG resets the chip whenever RTS is high while DTR is low: lower **RTS first, then DTR**. Opening the port may have reset the board, so the host retries every 500 ms for up to 5 s.
2. The board answers:

   ```json
   {"protocol":2,"firmware":"1.0.0","idf":"v6.1","board":"dualeye","max_payload":4096,"channels":["ctrl","metrics","log","audio_up","audio_down"],"capabilities":["tools","voice","speaker"]}
   ```

   `voice` (and `audio_up`) are there when the wake word runs (ESP-SR models found in the `model` partition), `speaker` (and `audio_down`) when the board can play speech.

3. The host reads faces and rotation with `get_state` and adopts them (the CLI pushes its own with `set_face` / `set_rotation` when given on the command line), then streams `metrics`. Later changes on the host are pushed with `tools/call`.

When the host sees `ready` it repeats step 3. A board that never answers `hello` but prints protocol 1's version line (`{"dualeye":"0.3.0",…}`, in answer to the text line `?version`) is reported as that version, so the app can offer to update it.

## Board tools

Screens are named `left` (the CPU screen) and `right` (the GPU screen); `both` is the default where a tool takes `screen`. Faces, rotation, brightness, the mic mute, the wake word and the speaker volume are kept in NVS and survive a reboot.

| Tool | Arguments | Effect |
|------|-----------|--------|
| `set_face` | `face`: `classic` · `rings` · `plus` · `bar` · `claude` · `clawd`; `screen` | Switch the watch face |
| `set_rotation` | `degrees`: 0 · 90 · 180 · 270; `screen` | Turn the screen clockwise on top of the DualEye mounting |
| `set_brightness` | `percent`: 0–100; `screen` | Backlight level (0 turns it off) |
| `show_text` | `text` (up to 120 characters, ASCII); `screen`; `seconds`: 1–30, default 4 | Show a message over the face, then hide it |
| `set_mic` | `muted`: boolean | Stop or restart listening for the wake word. Muted, the mic isn't read at all |
| `set_wake_word` | `word`: `alexa` (default) · `hiesp` | Listen for "Alexa" or "Hi ESP" from now on. Refused when the `model` partition has no model for it |
| `set_volume` | `percent`: 0–100 | Speaker volume (default 60) |
| `get_state` | none | Firmware, uptime, metrics state, each screen's face, rotation and brightness, voice (`available`, `wake_word`, `wake_word_id`, `model`, `wake_words` the board has models for, `muted`, `state`), audio (`speaker`, `volume`, `playing`), UI load (`busy_pct` and `max_frame_ms` of `lv_timer_handler` over the last 5 s), free memory, link counters (as `structuredContent`) |

Example:

```json
→ {"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"set_face","arguments":{"screen":"left","face":"rings"}}}
← {"jsonrpc":"2.0","id":7,"result":{"content":[{"type":"text","text":"left: rings"}],"isError":false}}
```

From the CLI: `dualeye tools`, `dualeye call set_face --screen left --face rings`, or `dualeye call set_face '{"screen":"left","face":"rings"}'`. MCP clients get the same tools from `dualeye mcp` (see the README).

## Sharing the board between host processes

Only one process can hold the port. The bridge (in the app, or `dualeye` streaming) runs a **hub**: a TCP server on `127.0.0.1`, on a port the OS picks, announced with a random token in `hub.json` in DualEye's data folder (mode 0600). Other processes (`dualeye mcp`, `dualeye call`) use the board through it; when there is no hub, they open the port for one call and close it. Implementation: `host/dualeye-core/src/hub.rs`.

One JSON-RPC 2.0 message per line, one request at a time. The first request must be `hello` with the token; otherwise the hub answers `-32001` and closes the connection.

| Method | Params | Result |
|--------|--------|--------|
| `hello` | `{"token":"…","client":"dualeye-mcp/0.1.0"}` | `{"bridge":"0.1.0","board":<hello result or null>,"port":"/dev/cu.usbmodem101"}` |
| `tools/list`, `tools/call` | As on `ctrl` | Passed to the board unchanged, one at a time |
| `host/snapshot` | none | `{"snapshot":<latest sample or null>,"age_ms":…}` |
| `host/say` | `{"text":"Ciao!","language":"it"}` (`language` optional: what the text looks like) | Once played: `{"text":…,"first_audio_ms":…,"played_ms":…,"reason":"done","underruns":0,"lost":0}`. `-32000` unless the bridge has text-to-speech (the app with spoken replies on, or `dualeye --tts`) |

While no board is attached (not found, rebooting, esptool flashing it) board methods fail with `-32000` and the reason as the message. After a successful `set_face` or `set_rotation` the bridge reads `get_state` and updates its own settings, so the app shows the change and keeps it.
