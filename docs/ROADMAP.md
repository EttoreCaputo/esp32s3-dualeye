# Roadmap — voice control & local LLM

Background and design choices: [voice-feasibility.md](voice-feasibility.md).

Principles:
- **Small sprints.** Each milestone is 1–3 sprints of about one week. Each sprint ends with something you can run and see.
- **Every milestone is shippable.** Nothing half-done lands in a release; voice stays behind an opt-in toggle until M7.
- **De-risk first.** Hardware and transport come before AI.
- **Backward compatible.** ~~The app keeps working with firmware 0.3 boards (legacy mode).~~ Dropped for M1: the app recognises a 0.3 board and offers the update instead.

## Overview

```
M0 Audio bring-up ─► M1 Protocol v2 ─► M2 MCP server ──────────────┐
                          │                                         ▼
                          └─► M3 Wake word ─► M4 Mic → STT ─► M5 TTS → speaker ─► M6 Local LLM agent ─► M7 Polish & release
```

| # | Milestone | Firmware | Result you can see |
|---|-----------|----------|--------------------|
| M0 | Audio bring-up | 0.4.0-dev | Tone from the speaker; a mic recording saved as WAV on the host |
| M1 | Protocol v2 (framed JSON-RPC over USB) | 0.4.0 | `dualeye call set_face --screen left --face rings` works; logs and metrics unaffected |
| M2 | MCP server | 0.4.x | Claude Code controls the screens |
| M3 | Wake word ("Hey Duo") + "listening" UI | 0.5.0 | Say "Hey Duo" and the eyes react |
| M4 | Mic streaming + STT | 0.5.x | Transcript (IT/EN) shown in the app |
| M5 | TTS → speaker | 0.6.0 | The board speaks a reply |
| M6 | Local LLM agent (llama.cpp) | 0.6.x | "Metti la faccia rings a sinistra" / "Put rings on the left" works end to end |
| M7 | Polish & release | 1.0.0 | Voice assistant in a public release |

---

## M0 — Audio bring-up ✅

Goal: prove the audio hardware works and pin down its limits.

- [x] I2S/I2C pins, amplifier-enable pin, mic count and AEC loopback, recorded in `main/board_audio.h`
- [x] Add `espressif/esp_codec_dev`; initialise ES8311 (out) and ES7210 (in) at 16 kHz / 16-bit
- [x] Chime and sine on the speaker (`!audio tone`), optional boot chime (`CONFIG_DUALEYE_AUDIO_BOOT_CHIME`)
- [x] Record up to 8 s of all four ES7210 channels and dump them to the host (base64 over the log, CRC-checked), then play back (`tools/audio_selftest.py`)
- [x] Measure CPU and heap with LVGL running, idle and while playing
- [x] Decide: **full duplex with AEC** (channel 1 is a clean speaker loopback)

Done when: audible tone, intelligible recording, no UI regressions, and the AEC decision recorded. Results are in [voice-feasibility.md](voice-feasibility.md#m0-results).

## M1 — Protocol v2 ✅

Goal: a robust, bidirectional, binary-safe link that everything else builds on.

- [x] Spec [`docs/protocol.md`](protocol.md): frame format (COBS, channel, length, CRC16), channels (`ctrl`, `metrics`, `log`, `audio_up`, `audio_down`), handshake with version and capabilities
- [x] Firmware: `usb_serial_jtag` driver in binary mode; `link` task with a mux/demux; `ESP_LOG` → `log` frames (`main/link.c`, `main/link_frame.c`)
- [x] Firmware: JSON-RPC dispatcher with `hello`, `tools/list`, `tools/call` (`main/rpc.c`, cJSON); `ready` notification at boot
- [x] First tools: `set_face`, `set_rotation`, `set_brightness` (LEDC PWM), `show_text` (toast on one or both screens), `get_state` (`main/board_tools.c`); faces, rotation and brightness are board state in NVS
- [x] Host (`dualeye-core`): framing codec (`protocol.rs`), `Link` with handshake and JSON-RPC (`link.rs`); metrics move to the `metrics` channel; a 0.3 board is recognised and offered the update. ~~Legacy fallback for firmware ≤ 0.3~~: dropped
- [x] CLI: `dualeye tools` and `dualeye call <tool> [json-args | --name value …]`
- [x] Tests: codec unit tests (Rust, and the C codec round-tripped on the host); flood, garbage, host stall, host restart and board reboot on hardware. Unplug and replug: to do by hand
- [x] Found on the way: opening the port reset the board (DTR lowered before RTS); fixed in the Rust host and `tools/dualeye_link.py`
- [x] `tools/audio_selftest.py` moved to protocol v2 (`debug/audio` method)

Done when: the app, the CLI and board-console logs work on 0.4 firmware. ~~A 0.3 board still works with the new app~~ (dropped).

## M2 — MCP server

Goal: board tools available to any MCP client. This is a useful milestone on its own.

- [x] `dualeye-core`: MCP server with `rmcp` 3.5 (`mcp.rs`, feature `mcp`); tools proxied from the board's `tools/list`, last list cached in `board-tools.json`, `tools/list_changed` when the board's tools turn up later
- [x] `dualeye mcp` subcommand and `dualeye-app --mcp` (stdio transport), with setup instructions for Claude Code and Claude Desktop in the README
- [x] Share the serial port between the bridge and MCP: the bridge runs a hub on `127.0.0.1` (port and token in `hub.json`, `hub.rs`); MCP and `dualeye call` go through it, or open the port for one call when nothing streams. Faces and rotation changed through the hub flow back to the app's settings; on connect the bridge adopts the board's faces and rotation instead of overwriting them
- [x] Host tools: `get_metrics`, `get_claude_usage`
- [x] App: "MCP server" section (Settings → Display) with connected clients, last call and copy-paste snippets for Claude Code and Claude Desktop
- [ ] Try it from Claude Code and Claude Desktop by hand

Done when: Claude Code changes faces and shows text on the board.

## M3 — Wake word ("Hey Duo") + listening UI

Goal: always-on wake-word detection without hurting the UI.

- [x] Custom partition table with a `model` partition (`partitions.csv`, right after a 3 MB app; NVS where it was). `idf.py merge-bin` puts `srmodels.bin` in the merged image, so the flasher (one image at 0x0) and `firmware.rs` (first app partition) needed no change; the image grows to 3.5 MB
- [x] ESP-SR 2.5.5 on IDF 6.1: AFE (`MRNN`: mic, speaker loopback) with AEC and VAD + WakeNet9 "Hi ESP" as a bootstrap (`main/voice.c`); feed and fetch tasks on core 1, LVGL and UI refresh pinned to core 0; CPU at 240 MHz, caches 32 KB I / 64 KB D. NS is not in the SR pipeline by design and AGC is left for the M4 stream. LVGL's 128 KB heap moved to PSRAM (`main/linker.lf`) to make room in internal RAM
- [x] Mic at 37.5 dB (maximum), loopback kept at 30 dB
- [ ] Spike: train a **"Hey Duo"** model with microWakeWord (Piper-generated IT/EN samples plus negatives); run it on the board after the AFE
- [ ] Switch the default wake word to "Hey Duo" once it meets the targets below; keep "Hi ESP" as a fallback option
- [x] "Eyes" states: idle → listening → thinking → speaking: a ring round both screens (`main/ui_voice.c`), mirrored in the app; the host sets thinking and speaking with `voice/state`
- [x] Wake event sent to the host (`wake` and `voice_state` notifications, `BridgeEvent::Wake` / `VoiceState`); `set_mic` tool, mute kept in NVS
- [x] UI load in `get_state` (`ui.busy_pct`, `ui.max_frame_ms`)
- [ ] Measurements: wake-word hit rate and false triggers per hour, frame-time impact on LVGL

Done when: at least 90 % detection at 1–2 m in a quiet room, fewer than 1 false trigger per hour, and no visible UI stutter.

## M4 — Mic streaming + speech-to-text

Goal: from wake word to transcript on the host.

- [ ] Firmware: after the wake word, stream PCM on `audio_up` until VAD end of speech or a timeout; the listening UI shows the level
- [ ] Host: voice pipeline skeleton (state machine, audio buffer, WAV debug dump)
- [ ] Host: whisper.cpp sidecar (`whisper-server`); language auto-detect IT/EN
- [ ] Model manager: download, verify and store models (Whisper first); settings for model size
- [ ] App: "Voice" tab with an enable toggle, transcript log and model status

Done when: IT and EN phrases are transcribed within 1.5 s (CPU) of the end of speech.

## M5 — Text-to-speech → speaker

Goal: the board talks back.

- [ ] Host: Piper sidecar; voice per language; sentence-level streaming
- [ ] Firmware: `audio_down` jitter buffer → ES8311; "speaking" UI; half-duplex mic mute (or AEC from M0)
- [ ] Temporary rule-based intents (for example "rings a sinistra" → `set_face`) so the full loop is testable before the LLM
- [ ] Volume tool plus app setting

Done when: a spoken command gets a spoken confirmation plus the screen action, in IT and EN.

## M6 — Local LLM agent

Goal: natural conversation with a small local model using the MCP tools.

- [ ] `llama-server` sidecar (`--jinja`, tool calling); model manager adds GGUF models
- [ ] Agent loop in `dualeye-core`: transcript → LLM with MCP tools → tool calls → reply → TTS; short conversation memory
- [ ] System prompt: reply in the user's language, concise, spoken style
- [ ] Eval set: about 50 IT/EN commands with expected tool calls; benchmark Qwen3 1.7B and 4B (and newer candidates) for accuracy and latency on Mac, NVIDIA and CPU-only hosts
- [ ] Replace the M5 rule-based intents (keep them as an offline fallback if useful)

Done when: at least 90 % of the eval set is correct with the default model, with the median time to first sound or action under 3 s on the reference hosts.

## M7 — Polish & release

- [ ] Barge-in (if AEC is available), follow-up without the wake word for a few seconds, error sounds and states
- [ ] Hardware capability check and model recommendation in the app
- [ ] Packaging of the sidecar binaries per OS in the Tauri bundle; license table
- [ ] Docs: README voice section, troubleshooting, privacy note (everything stays local)
- [ ] Release 1.0.0 and firmware changelog
