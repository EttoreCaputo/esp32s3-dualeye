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
| `tools/list` | none, or `{"cursor":"4"}` | `{"tools":[Tool, …]}`, plus `"nextCursor":"4"` when more tools follow (firmware 1.3.1): a frame holds only so many, so ask again with that `cursor` until there is no `nextCursor`. Earlier firmware sends them all at once, and stopped answering once they no longer fit in a frame |
| `tools/call` | `{"name":"set_face","arguments":{…}}` | `CallToolResult` |
| `voice/state` | `{"state":"thinking"}`: `idle` · `listening` · `thinking` · `speaking` · `error` | `{}`; what the voice overlay shows. For the host's voice pipeline (M4 on), so not a tool. `error` (firmware 1.0) is a red ring with a short two-note sound, for 1.5 s: the host failed, or heard no words. A state other than `idle` goes back to `idle` by itself after 30 s (`listening`: 13 s, `error`: 1.5 s). `-32602` without voice |
| `voice/listen` | none, or `{"follow_up":true}` | `{}`; stream an utterance as if the wake word had been heard (push-to-talk, `trigger` `host`). With `follow_up` (firmware 1.0; the host asks right after a spoken answer) the `trigger` is `follow_up`, it gives up after 4 s without speech, and speech in its first 0.8 s (the echo of the answer) doesn't count. `-32602` without voice or muted |
| `voice/stop` | none | `{}`; end the utterance being streamed (`reason` `host`), if any |
| `audio/stop` | none | `{}`; stop talking: what's buffered from `audio_down` is dropped and the stream ends (`reason` `stopped`) |
| `media/info` | none | `{"slot_size":4161536,"chunk":2304,"screens":{"left":{"frames":1,"bytes":…},"right":{…}}}` (firmware 1.1): room for each screen's picture, and what's there |
| `media/begin` | `{"screen":"left","size":bytes,"crc32":n}` | `{"chunk":2304}`; start replacing that screen's picture (the old one goes at once). The format is in `main/media.h`: 240 × 240 RGB565 frames, raw or run-length coded, made by the host |
| `media/write` | `{"offset":n,"data":base64}` | `{"written":n}`; the next `chunk` bytes at most, in order. The flash is erased as the data arrives |
| `media/end` | none | `{"frames":n}` once the CRC-32 (the zlib one) matches and the picture checks out; then the `image` face shows it and it survives a reboot |
| `media/clear` | `{"screen":"right"}` | `{}`; remove that screen's picture |
| `music/info` | none | `{"size":240,"chunk":2880,"art":n}` (firmware 1.3): the cover's side, the most it takes per `music/art`, the id of the cover it has (0: none) |
| `music/art` | `{"id":n,"offset":n,"data":base64}` | `{"received":n}`, plus `"done":true` with the last chunk; the cover of what's playing, for the `music` face (firmware 1.3): 240 × 240 RGB565, little-endian, raw (115200 bytes), at most `chunk` bytes at a time and in order from offset 0. `id` (1 to 2^30) names it: the last chunk puts it on screen wherever the snapshot's `music.art` says that id. Kept in PSRAM, not across a reboot |
| `eyes/gaze` | `{"x":-0.42,"y":0.1}` | Sent as a notification (no `id`, no answer), up to 20 times a second while the host's mouse pointer moves (firmware 1.3): where it is, -1 (left, top) to 1 (right, bottom) of all the host's screens. The `eyes` face looks there for 6 s after the last one, then about on its own; once the host has sent any, a minute without one (nor a conversation) and the eyes doze off, until the next |
| `debug/audio` | `{"cmd":"tone 440 500"}` | `{}`; the M0 audio self-test, its output comes as `log` lines (see `main/audio_selftest.h`) |

`Tool` and `CallToolResult` have the shapes of the [MCP](https://modelcontextprotocol.io/specification/2025-06-18/server/tools) `tools/list` and `tools/call` results (`name`, `description`, `inputSchema`; `content`, `structuredContent`, `isError`), so the host's MCP server (M2) can pass them through unchanged. A tool that runs but fails (bad argument, out of range) returns `isError: true` with the reason as text. Protocol errors use the standard JSON-RPC codes: `-32700` parse error, `-32600` invalid request, `-32601` unknown method, `-32602` invalid params (including an unknown tool), `-32603` a reply too long for a frame (firmware 1.3.1; earlier ones sent nothing).

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

Firmware 1.2 also reads `"timer":{"kind":"timer","state":"run","left_s":272.4,"total_s":600,"label":"PASTA","more":1,"round":2,"rounds":4,"screen":"right"}`, the host's timer that ends first (absent when none runs), for the `timer` face: `kind` is `timer`, `work` or `break` (a pomodoro's, with its `round` of `rounds`) or `reminder`; `state` is `run`, `pause` or `ring`; `left_s` counts from the snapshot (the board counts down on its own between snapshots); `label` is ASCII capitals, up to 27 characters; `more` is how many other timers run; `screen` (`left` or `right`, optional) is the screen that shows the timer face instead of its own while the data is live. While `state` is `ring` the board plays a short chime every 2 s (every 2.5 s before firmware 1.3.3), unless the voice overlay or the speaker is busy, and stops after 10 s of it (firmware 1.3.3) even if `state` stays `ring`. The host keeps the timers, ends the ringing (after 10 s, on the wake word, or when told) and says what the timer was for.

Firmware 1.3 also reads `"music":{"state":"play","title":"Zitti e buoni","artist":"Maneskin","pos_s":61.2,"dur_s":195.0,"art":656393640}`, what's playing on the host (absent when nothing is), for the `music` face: `state` is `play` or `pause`; `title` (up to 63 characters) and `artist` (up to 47, optional) are ASCII; `pos_s` and `dur_s` are optional, and the board counts the position on between snapshots while it plays; `art` (optional) is the id of the cover sent with `music/art`, which the host sends before the snapshot that names it. Without the cover that `art` names, the face shows a record instead.

Firmware 1.4 also reads `"pet":{"min":845,"idle":12}`, for the pet's mood: `min` is the local time in minutes since midnight (lively by day, sleepy at night), `idle` (optional) the seconds since the last mouse or keyboard input on the host (away after 15 minutes: no idle scenes; a greeting on coming back). The host also sends `music` whenever something plays, not only for the `music` face: the pet dances to it.

Firmware 1.1 also reads `"net":{"rx_bps":…,"tx_bps":…}` (bytes per second, every interface but loopback), `"disk":{"used_gb":…,"total_gb":…,"read_bps":…,"write_bps":…}` (the system disk; the rates are optional) and `"bat":{"pct":…,"charging":…,"plugged":…,"mins":…}` (absent without a battery; `mins` to empty, or to full while charging, optional), for the `net`, `disk` and `battery` faces; older firmware skips them.

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

   Firmware 1.3 adds the capabilities `music` (`music/art`) and `gaze` (`eyes/gaze`). `voice` (and `audio_up`) are there when the wake word runs (ESP-SR models found in the `model` partition), `speaker` (and `audio_down`) when the board can play speech.

3. The host reads faces and rotation with `get_state` and adopts them (the CLI pushes its own with `set_face` / `set_rotation` when given on the command line), then streams `metrics`. Later changes on the host are pushed with `tools/call`.

When the host sees `ready` it repeats step 3. A board that never answers `hello` but prints protocol 1's version line (`{"dualeye":"0.3.0",…}`, in answer to the text line `?version`) is reported as that version, so the app can offer to update it.

## Board tools

Screens are named `left` (the CPU screen) and `right` (the GPU screen); `both` is the default where a tool takes `screen`. Faces, rotation, brightness, the mic mute, the wake word and the speaker volume are kept in NVS and survive a reboot.

| Tool | Arguments | Effect |
|------|-----------|--------|
| `set_face` | `face`: `classic` · `rings` · `plus` · `bar` · `claude` · `clawd` · `net` · `disk` · `battery` · `image` (those four: firmware 1.1) · `timer` (firmware 1.2) · `music` · `eyes` (firmware 1.3); `source`: `cpu` · `gpu` (firmware 1.1, optional); `screen` | Switch the watch face. `source` is whose metrics classic, rings, plus and bar show on that screen, kept until changed (by default the CPU on the left, the GPU on the right) |
| `set_rotation` | `degrees`: 0 · 90 · 180 · 270; `screen` | Turn the screen clockwise on top of the DualEye mounting |
| `set_brightness` | `percent`: 0–100; `screen` | Backlight level (0 turns it off) |
| `show_text` | `text` (up to 120 characters, ASCII); `screen`; `seconds`: 1–30, default 4 | Show a message over the face, then hide it |
| `set_mic` | `muted`: boolean; `wake_sound`: boolean (firmware 1.3.3; at least one) | `muted`: stop or restart listening for the wake word. Muted, the mic isn't read at all. `wake_sound`: play a short chime when the wake word is heard (`true`, the default). `get_state` has it as `voice.wake_sound` |
| `set_wake_word` | `word`: `alexa` (default) · `hiesp` | Listen for "Alexa" or "Hi ESP" from now on. Refused when the `model` partition has no model for it |
| `set_volume` | `percent`: 0–100 | Speaker volume (default 60) |
| `set_eyes` | `on`: boolean; `idle`: boolean (at least one) | `on`: during a conversation, show animated eyes over the whole screens (`true`, the default) or the ring round the watch face (`false`). `idle` (firmware 1.0.2): while nobody is talking, the eyes play a short scene every 30–120 s (`true`, the default). `get_state` has them as `voice.eyes` and `voice.idle_eyes` |
| `play_eyes` | `name`: `look_around` · `sleepy` · `suspicious` · `happy` · `surprised` · `wink` · `angry` · `sad` · `dizzy` · `cross_eyed` · `eye_roll` · `curious` · `love` · `scan` · `shy` · `flutter`; firmware 1.4: `yawn` · `sneeze` · `giggle` · `excited` · `bored` · `confused` · `scared` · `peekaboo` · `nod` · `shake` · `hiccup` · `mischief` · `dance` · `sing` · `purr` · `sigh` · `focus` · `snore` · `glitch` · `proud` · `hot` · `relieved` · `tired` · `charged`; optional | Play that scene now (a random one without `name`), a few seconds over the watch faces, with its sounds (firmware 1.4, while pet sounds are on). Refused during a conversation |
| `play_sound` | `name` (firmware 1.4): the earcons `wake` · `error` · `alarm` · `volume`, and the pet's `boot` · `hello` · `mute` · `unmute` · `think` · `chirp` · `happy` · `giggle` · `excited` · `yawn` · `sneeze` · `snore` · `purr` · `sigh` · `gasp` · `startle` · `boing` · `aww` · `grumble` · `heartbeat` · `smooch` · `hmm` · `confused` · `uh_oh` · `tada` · `yes` · `nope` · `sing` · `beat` · `scan` · `lock` · `hiccup` · `whistle` · `wink` · `boop` · `pfft` · `eep` · `flutter` · `mischief` · `whimper` · `boo` · `glitch` · `pant` · `phew` · `drain` · `charge` | Play one of the board's own sounds, made on the board (after those queued before it; dropped while the host's speech plays). Refused for a pet sound while they're off |
| `set_pet` | `sounds`: boolean; `react`: boolean (firmware 1.4; at least one) | The board as a pet. `sounds`: whether it makes its pet sounds (`true`, the default): in the eyes' scenes, `boot` and `hello` (the host connecting), `mute`/`unmute`, `think` (heard you), `startle` and a few `snore`s on the eyes face; the earcons play either way, and `set_volume` plays `volume` at the new level. `react`: whether it reacts to what the snapshots tell (`true`, the default): `hot` over 88 °C and `relieved` under 75 °C, `dance` to music, `focus` and `proud` as Claude starts and finishes, `tired` on a low battery and `charged` when plugged in, `yawn` at bedtime, a greeting when you come back. Its mood (energy, happiness, affection, kept in NVS) picks the idle scenes and how often they come, tints the eyes face and pitches its sounds; `get_state` has it as `pet` (`mood`, `energy`, `happiness`, `affection`, `away`, `sounds`, `reactions`) |
| `get_pet` | none (firmware 1.4) | The pet's mood as `structuredContent`: `mood` (`content` · `happy` · `excited` · `loving` · `bored` · `grumpy` · `sad` · `sleepy` · `hot`), `energy`, `happiness`, `affection` (0–1), `away`, `now` (what's moving it: `hot` · `music` · `claude` · `battery_low` · `charging` · `night` · `away` · `bored`), `minute` and `idle_s` as the host last sent them, `lonely_s` (since someone last talked or played with it, or music played), `pitch` (its voice, 1 as written), `pace` (the idle scenes come every 30–120 s times this), `recent` (its last six reactions, newest first: `what` it reacted to, the `scene`, `ago_s`), and the settings `sounds`, `reactions`, `idle_scenes` |
| `get_state` | none | Firmware, uptime, metrics state, each screen's face, source (firmware 1.1), whether it has a picture (`image`, firmware 1.1), rotation and brightness, voice (`available`, `wake_word`, `wake_word_id`, `model`, `wake_words` the board has models for, `muted`, `state`), audio (`speaker`, `volume`, `playing`), the pet (firmware 1.4: `mood`, `energy`, `happiness`, `affection`, `away`, `sounds`, `reactions`), UI load (`busy_pct` and `max_frame_ms` of `lv_timer_handler` over the last 5 s), free memory, link counters (as `structuredContent`) |

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
| `tools/list`, `tools/call`, `media/...` | As on `ctrl` | Passed to the board unchanged, one at a time |
| `host/snapshot` | none | `{"snapshot":<latest sample or null>,"age_ms":…}` |
| `host/timers` | `{"name":"set_timer","arguments":{"minutes":10},"language":"it"}`: one of the timer tools (`set_timer`, `set_reminder`, `pomodoro`, `control_timer`, `get_timers`) | `{"text":…,"is_error":false}`, run on the bridge's timers; `-32000` without a bridge (then `Board::timer_tool` uses `timers.json` directly) |
| `host/say` | `{"text":"Ciao!","language":"it"}` (`language` optional: what the text looks like) | Once played: `{"text":…,"first_audio_ms":…,"played_ms":…,"reason":"done","underruns":0,"lost":0}`. `-32000` unless the bridge has text-to-speech (the app with spoken replies on, or `dualeye --tts`) |

While no board is attached (not found, rebooting, esptool flashing it) board methods fail with `-32000` and the reason as the message. After a successful `set_face` or `set_rotation` the bridge reads `get_state` and updates its own settings, so the app shows the change and keeps it.
