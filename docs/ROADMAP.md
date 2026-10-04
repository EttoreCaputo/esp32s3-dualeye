# Roadmap — voice control & local LLM

Background and design choices: [voice-feasibility.md](voice-feasibility.md).

Principles:
- **Small sprints.** Each milestone is 1–3 sprints of about one week. Each sprint ends with something you can run and see.
- **Every milestone is shippable.** Nothing half-done lands in a release; voice stays behind an opt-in toggle (it still is in 1.0: off until the person turns it on).
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
| M3 | Wake word ("Alexa") + "listening" UI | 0.5.0 (shipped in 0.6.0) | Say "Alexa" and the eyes react |
| M4 | Mic streaming + STT | 0.5.x (shipped in 0.6.0) | Transcript (IT/EN) shown in the app |
| M5 | TTS → speaker | 0.6.0 | The board speaks a reply |
| M6 | Local LLM agent (llama.cpp) | 0.6.x (no firmware change) | "Metti la faccia rings a sinistra" / "Put rings on the left" works end to end |
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

## M3 — Wake word ("Alexa") + listening UI

Goal: always-on wake-word detection without hurting the UI.

- [x] Custom partition table with a `model` partition (`partitions.csv`, right after a 3 MB app; NVS where it was). `idf.py merge-bin` puts `srmodels.bin` in the merged image, so the flasher (one image at 0x0) and `firmware.rs` (first app partition) needed no change; the image grows to 3.8 MB with two WakeNet models
- [x] ESP-SR 2.5.5 on IDF 6.1: AFE (`MRNN`: mic, speaker loopback) with AEC and VAD + WakeNet9 (`main/voice.c`); feed and fetch tasks on core 1, LVGL and UI refresh pinned to core 0; CPU at 240 MHz, caches 32 KB I / 64 KB D. NS is not in the SR pipeline by design and AGC is left for the M4 stream. LVGL's 128 KB heap moved to PSRAM (`main/linker.lf`) to make room in internal RAM
- [x] Mic at 37.5 dB (maximum), loopback kept at 30 dB
- [x] Wake word: built-in WakeNet9 **"Alexa"** (`wn9_alexa`) by default, **"Hi ESP"** (`wn9_hiesp`) as the other choice; `set_wake_word` switches at runtime (rebuilds the AFE, about 110 ms) and keeps it in NVS. One WakeNet runs at a time: `afe_config_init()` would load the first two in the partition
- ~~Spike: train a "Hey Duo" model with microWakeWord~~: put off (datasets of several GB and hours of training); a custom word can come back after M7
- [x] "Eyes" states: idle → listening → thinking → speaking: a ring round both screens (`main/ui_voice.c`), mirrored in the app; the host sets thinking and speaking with `voice/state`
- [x] Wake event sent to the host (`wake` and `voice_state` notifications, `BridgeEvent::Wake` / `VoiceState`); `set_mic` tool, mute kept in NVS
- [x] UI load in `get_state` (`ui.busy_pct`, `ui.max_frame_ms`)
- [ ] Measurements: wake-word hit rate and false triggers per hour, frame-time impact on LVGL. So far (Mac speaker at about 1 m, `say`): "Alexa" 8/8, "Hi ESP" 1/3; voice tasks 20 % of core 1; UI `max_frame_ms` about 58 with voice, 51 muted, `busy_pct` 6–12 against 5.5

Done when: at least 90 % detection at 1–2 m in a quiet room, fewer than 1 false trigger per hour, and no visible UI stutter.

## M4 — Mic streaming + speech-to-text

Goal: from wake word to transcript on the host.

- [x] Firmware: after the wake word, stream PCM on `audio_up` until VAD end of speech or a timeout; the listening UI shows the level. `utterance_start` / `utterance_end` around each stream, 4-byte header (id, sequence) on each frame; ends 0.75 s after the last word (VAD hangover cut to 500 ms), after 5 s without speech (the wake word's own VAD tail doesn't count) or at 12 s; WakeNet off meanwhile. `voice/listen` and `voice/stop` for push-to-talk. The level is a short bright arc at the top of the ring (UI `busy_pct` about 19 % while listening; hiding the ring costs one ~80 ms frame)
- [x] Host: voice pipeline skeleton (`voice.rs`: reassembly with lost frames filled with silence, a pipeline thread per connection, WAV debug dump with `dualeye --voice-dump`; `BridgeEvent::Listening` / `Utterance`). Tried with "Alexa" + an IT and an EN sentence from the Mac speaker: no frames lost, speech at about −33 dBFS RMS over a −65 dBFS floor
- [x] Host: whisper.cpp sidecar (`whisper-server`); language auto-detect IT/EN. `stt.rs`: started in the background with the model loaded, restarted if it dies, stopped on exit (and a leftover one from a killed host is stopped through a pid file); plain HTTP on 127.0.0.1. Whisper's detection, falling back to the likelier of IT/EN when it picks another language; a vocabulary prompt (face names) in the language decoded. `dualeye --stt [SIZE|FILE] [--stt-language auto|it|en]`, `BridgeEvent::Transcript` / `VoiceError`. With `ggml-small` on an M1 Pro: 10/10 languages right on 5 IT + 5 EN commands (macOS voices from the speaker), 0.6–0.7 s per transcript, so about 1.4 s from the last word; a few words wrong ("rinusa" for "rings a")
- [x] Model manager: download, verify and store models (Whisper first); settings for model size. `models.rs`: base, small (default) and large-v3-turbo-q5_0, pinned by size and SHA-256, downloaded to `.part` and renamed once checked (`download.rs`, shared with esptool's setup); `dualeye models [download|remove ID]`
- [x] App: "Voice" tab with an enable toggle, transcript log and model status. Off by default; language (auto, it, en), WAV recordings for debugging, model list with download progress, stop and delete; turning it on or switching model swaps the bridge's voice config without a reconnect. SIGTERM and SIGINT quit the app (and the CLI) cleanly, so the sidecar stops too
- [ ] Try the Voice tab by hand with a real voice at 1–2 m; `whisper-server` still has to be installed by hand (bundling is M7)

Done when: IT and EN phrases are transcribed within 1.5 s (CPU) of the end of speech.

## M5 — Text-to-speech → speaker

Goal: the board talks back.

- [x] Host: Piper sidecar; voice per language; sentence-level streaming. Piper is now [`piper-tts`](https://github.com/OHF-Voice/piper1-gpl) 1.8 (Python, GPL-3.0; the old C++ `rhasspy/piper` is archived), run as `python -m piper.http_server` from a virtualenv in DualEye's data folder: `dualeye piper install`, or the app's Install button with esptool's Python. `tts.rs`: one server for every voice, loaded at start; restarts and pid-file cleanup shared with whisper-server in `sidecar.rs`. Voices in the model manager (ONNX + JSON, SHA-256 pinned): `it_IT-paola-medium` (default), `it_IT-riccardo-x_low`, `en_GB-alba-medium` (default), `en_US-ljspeech-medium`, license of each shown. 22 kHz → 16 kHz with a windowed-sinc resampler; the reply goes out a sentence at a time, paced in real time 0.5 s ahead of the speaker (`Speaker` in `voice.rs`). A sentence takes 0.1–0.27 s to synthesize once the voice is loaded (M1 Pro)
- [x] Firmware: `audio_down` jitter buffer → ES8311; "speaking" UI; half-duplex mic mute (or AEC from M0). `main/playback.c`: 4 s ring in PSRAM, speaker on at 150 ms buffered, task on core 1 above the voice tasks, silence on underrun and `starved` after 1.5 s, `playback_end` with played ms, lost frames and underruns, `audio/stop`. The ring turns green with the speaker's level on it. The mic stays on with AEC: the wake word is ignored while the speaker plays and for 0.3 s after; the board saying "Alexa, …" through its own speaker didn't even reach WakeNet. On the ES7210 loopback, what the DAC plays correlates 0.87 with what was sent, no gaps. UI while speaking: `busy_pct` about 29 %, `max_frame_ms` about 100 (the ring appearing), against 12 % and 61 idle
- [x] Temporary rule-based intents (for example "rings a sinistra" → `set_face`) so the full loop is testable before the LLM. `intents.rs`: faces (with Whisper's spellings: "rinza", "rinusa", "clod"), screens, brightness and screens off/on, rotation, volume up/down/value, text on screen, CPU/GPU temperatures, time, help; answers in the language Whisper heard. `BridgeEvent::Reply` / `Spoken`; faces and rotation changed by voice flow back to the app's settings. Also a `speak` MCP host tool and `dualeye say`, through the hub's new `host/say`
- [x] Volume tool plus app setting. `set_volume` (NVS, default 60), `get_state.audio`; slider in the app's Voice tab, next to "Answer out loud", the voices with Test buttons, and each reply under its transcript
- [ ] Try it by hand with a real voice at 1–2 m, and the Voice tab's Piper install on a machine without it

Tried from the Mac speaker (macOS voices, "Alexa" then a command), with the CLI and in the app: 14 of 15 commands right on the first try, the one miss being Whisper's "rinza" for "rings a", now matched. From the end of speech to the first word of the answer about 1.5 s (0.75 s end of speech, 0.6 s Whisper, 0.1–0.27 s Piper), no underruns or lost frames. Firmware 0.6.0 (0.5.0 was never released: M3–M5 ship together).

Done when: a spoken command gets a spoken confirmation plus the screen action, in IT and EN.

## M6 — Local LLM agent ✅

Goal: natural conversation with a small local model using the MCP tools.

- [x] `llama-server` sidecar (`--jinja`, tool calling); model manager adds GGUF models
- [x] Agent loop in `dualeye-core`: transcript → LLM with MCP tools → tool calls → reply → TTS; short conversation memory
- [x] System prompt: reply in the user's language, concise, spoken style
- [x] Eval set: about 50 IT/EN commands with expected tool calls; benchmark Qwen3 1.7B and 4B (and newer candidates) for accuracy and latency on Mac, NVIDIA and CPU-only hosts
- [x] Replace the M5 rule-based intents (keep them as an offline fallback if useful)

Done when: at least 90 % of the eval set is correct with the default model, with the median time to first sound or action under 3 s on the reference hosts.

## M7 — Polish & release

- [x] Barge-in (if AEC is available), follow-up without the wake word for a few seconds, error sounds and states. Barge-in: the wake word over the board's speech stops it (`playback_end` reason `barge_in`) and starts a new utterance; the host stops sending the rest as soon as it sees the end (`CONFIG_DUALEYE_VOICE_BARGE_IN`, on by default). Follow-up: after an answer played to the end the host asks `voice/listen {"follow_up":true}`: trigger `follow_up`, 4 s without speech and it gives up, and VAD speech in the first 0.8 s doesn't count (with 0.3 s the answer's echo, still inside the VAD's hangover, ended the stream after 0.7 s). The app's switch "Keep listening after an answer", CLI `--no-follow-up`. Errors: a `voice/state` `error` (red ring, 1.5 s) plays two falling notes made on the board (`playback_earcon`, not reported to the host); the host sets it when Whisper fails or hears no words (not after a follow-up), or speaking fails. Tried from the Mac speaker: "metti rings a sinistra" then "e anche a destra" without the wake word, and the chain closes after 4 s of silence; "Alexa" 4 s into a long `dualeye say` stopped it at 5.6 s and "che ore sono" was answered; a non-speech sound after "Alexa" gave the red ring and the notes
- [x] Hardware capability check and model recommendation in the app. `hardware.rs`: processor, cores, memory, the GPU (Apple silicon, or NVIDIA through NVML), and the devices llama-server itself lists (`--list-devices`: 0.13 s, 24 s the first time on a Mac while Metal compiles, so the app asks in the background). Apple silicon 16 GB: `small` + `qwen3-4b-2507`; 8 GB: `qwen3.5-2b`; NVIDIA with 5 GB+ (and a CUDA llama-server): the 4B; without a usable GPU `base` + `qwen3.5-2b`, slow; less, no language model. New settings start with the recommendation; the Voice tab shows "This computer", a "recommended" tag on the models and a button to switch and download them; `dualeye models` says the same
- [x] Packaging of the sidecar binaries per OS in the Tauri bundle; license table. `tools/build_sidecars.sh` builds whisper-server (v1.9.4) and llama-server (b11146) statically, without OpenMP or OpenSSL (it checks nothing outside the OS is linked): Metal on Apple silicon, CPU (AVX2) elsewhere. CI builds them in each app job (cached by the script's hash) and adds them with `tauri.sidecars.conf.json` (`externalBin`, licenses in the resources), so local builds need neither. The host looks for `DUALEYE_WHISPER_SERVER` / `DUALEYE_LLAMA_SERVER`, then next to itself, then the PATH. On macOS: 18 MB + 5 MB, and the bundled pair answered a real voice command. Piper stays an on-demand install (GPL-3.0, separate process). [licenses.md](licenses.md)
- [x] Docs: README voice section, troubleshooting, privacy note (everything stays local)
- [x] Release 1.0.0 and firmware changelog: firmware, app, CLI and core at 1.0.0, `FIRMWARE_CHANGELOG.md` has its section
- [ ] Merge into `main`: CI builds and publishes v1.0.0. The Windows, Linux and Intel Mac sidecar builds have only run there
- [ ] Try it by hand with a real voice at 1–2 m (barge-in and follow-up above were tried with the Mac's speaker)
- [x] End of speech with music and noise about: WebRTC's VAD took them for speech, so the utterance ran to 12 s; pauses to think cut it short. Firmware 1.3.2: VADNet (`vadnet1_medium`, 287 KB; `srmodels.bin` 870 KB of the 2 MB partition) with `vad_min_speech_ms` 192 and `vad_energy_threshold` −55 dBFS, the end after 0.9 s of silence (was 0.75). Host: the music playing on the computer pauses from the wake word (or push-to-talk) to the end of the conversation, follow-ups included, and plays again after; not if the person played or paused it meanwhile ("pausa" stays paused). App switch "Pause the music while listening", CLI `--no-pause-music`
- [ ] Measure VADNet on the board: core 1 load against WebRTC's VAD (20 %), and end-of-speech with music at 1–2 m; tune `VAD_ENERGY_DB` and `END_SILENCE_MS` from the WAV recordings
