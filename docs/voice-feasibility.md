# Voice control & local LLM — feasibility study

Status: draft for review · Target: firmware 0.4 → 1.0 · Board: [Waveshare ESP32-S3-DualEye-LCD-1.28](https://docs.waveshare.com/ESP32-S3-DualEye-LCD-1.28)

## Goal

Talk to the board, in **Italian or English**, and have it act on the two screens. The end state is a conversation with a **small LLM running only on the host PC** (llama.cpp) that controls the board through **MCP tools**. The board stays connected over **USB only**.

The milestones and sprint plan are in [ROADMAP.md](ROADMAP.md).

## Verdict

**Feasible**, with one architectural constraint: the ESP32-S3 is the audio front end, and the host does all the heavy work.

| Stage | Where | Why |
|-------|-------|-----|
| Mic capture, wake word, voice activity detection | Board | Low latency, audio only leaves the board after the wake word, ESP-SR is built for this chip |
| Speech-to-text | Host (whisper.cpp) | Whisper doesn't fit on an MCU; ESP-SR MultiNet only knows English and Chinese |
| LLM + tool calling | Host (llama.cpp `llama-server`) | Even a 0.6B model is several hundred MB and needs GFLOPs per token |
| Text-to-speech | Host (Piper) | Good Italian and English voices, faster than real time on CPU |
| Speaker playback, "eyes" feedback | Board | ES8311 + amplifier; LVGL animations |
| Screen-control tools | Board, exposed via MCP on the host | Tools are defined once, in the firmware; the host is a thin proxy |

## Hardware

From the Waveshare page and the current `sdkconfig`:

- **ESP32-S3R8**: 2× LX7 @ 240 MHz, 512 KB SRAM, **8 MB octal PSRAM** (already enabled, 80 MHz), **16 MB flash**.
- **ES8311** codec + power amplifier → speaker header.
- **ES7210** 4-channel ADC → onboard microphone.
- 2× GC9A01 240×240 over SPI (already driven by the firmware).
- TF card slot; touch only on the touch variant of the board (treated as optional, detected at runtime).

ES8311 + ES7210 is the same pairing as Espressif's reference voice boards (and [xiaozhi-esp32](https://github.com/78/xiaozhi-esp32)), so `esp_codec_dev` and ESP-SR are known to work with it.

### M0 results

Measured on the board with `tools/audio_selftest.py` (firmware at 160 MHz, UI running):

| Item | Result |
|------|--------|
| Pins | I2C SCL 10 / SDA 11; I2S MCLK 12, BCLK 13, WS 14, DIN 15 (ES7210), DOUT 16 (ES8311); PA enable 9. No clash with the LCDs |
| Input channels | ES7210 TDM slot **0 = microphone**, slot **1 = speaker loopback (AEC reference)**, slots 2–3 unconnected |
| AEC reference | Clean: −91 dBFS when silent, the 1 kHz test tone at −27 dBFS while playing. **Full duplex with AEC and barge-in is possible** |
| Mic, 30 dB analog gain | Room noise −74 dBFS RMS; speech at 30–50 cm −50…−55 dBFS RMS, peaks −35…−40 dBFS. About 20 dB SNR but **~25 dB quieter than ideal** → raise the analog gain (max 37.5 dB) and rely on the AFE's AGC in M3 |
| Speaker | Chime and tones clearly audible at volume 60/100 |
| Internal heap | 112 KB free, 79 KB minimum since boot, **largest block 36 KB**. Tight for the ESP-SR AFE: keep its buffers in PSRAM and measure first thing in M3 |
| PSRAM | 8 MB, all free |
| CPU | Core 0: LVGL busy mainly during full redraws; core 1 about 99 % idle. The self-test's sine generator takes about 32 % of core 1 (software `sinf` plus the codec's mono → 32-bit slot expansion); real playback in M5 streams PCM instead |

Follow-ups for M3:
- The CPU runs at 160 MHz; move to 240 MHz before adding the AFE and wake word.
- The LVGL task has about 1 KB of stack headroom; grow it before adding the listening overlay.
- ~~On macOS, opening the port resets the board.~~ Fixed in M1: the OS raises DTR and RTS on open, and lowering DTR before RTS passed through the reset state (RTS high, DTR low). Both hosts now lower RTS first.

## Current firmware and host, and what has to change

| Area | Today | Needed |
|------|-------|--------|
| Transport | `stdin`/`stdout` over USB Serial/JTAG, newline JSON, one-way (host → board snapshots). The board replies with a version line and plain `ESP_LOG` text | Framed, bidirectional, binary-safe protocol: JSON-RPC control plus audio frames plus logs, on one channel |
| Control | `face` and `rotation` fields inside each metrics snapshot | Explicit commands (JSON-RPC methods) with responses and errors |
| Partitions | `partitions_singleapp` | Custom table with a `model` partition for ESP-SR (WakeNet about 300–400 KB, VADNet), plus room for OTA later. The flasher in `host/dualeye-core/src/firmware.rs` must handle it |
| Tasks | `lvgl`, `ui_refresh`, `metrics_io` | Plus `audio_in` (I2S → AFE → WakeNet/VAD), `audio_out` (playback), `link` (framing/mux); cores pinned deliberately |
| Host | `dualeye-core` bridge, CLI, Tauri app | Plus MCP server, voice pipeline (STT → LLM → TTS), model manager, app UI for voice |

## Key design decisions (proposed)

### 1. Board-defined tools, host-side MCP server

The firmware exposes a small JSON-RPC API over serial (`tools/list`, `tools/call`, events). `dualeye-core` runs an **MCP server** (Rust, official `rmcp` SDK) that:
- advertises the board's tools dynamically (whatever `tools/list` returns for that firmware version);
- serves them over **stdio** (for Claude Code / Claude Desktop) and in-process to the local voice agent.

This makes M1 and M2 useful on their own, before any audio work: you can already say "put the rings face on the left" to Claude Code. It also keeps a single source of truth, since new firmware tools show up without host changes.

Why not an MCP server on the board itself? MCP over HTTP needs Wi-Fi. MCP-over-stdio needs a process on the host anyway. Proxying keeps the firmware small and USB-only.

### 2. One USB link, framed and multiplexed

USB Serial/JTAG is full speed, about 1 MB/s in practice. Audio is 16 kHz × 16-bit × mono = **32 KB/s** per direction, so bandwidth is not the issue. Coexistence is:
- replace text-mode `stdio` with the `usb_serial_jtag` driver in binary mode;
- frames with a channel byte (`ctrl` JSON-RPC, `audio_up`, `audio_down`, `log`, `metrics`), length, CRC, and resync by COBS or SLIP;
- `ESP_LOG` redirected into `log` frames so logs never corrupt data. The app's board console keeps working;
- **version handshake**: the host detects the firmware protocol version and falls back to legacy newline JSON for firmware ≤ 0.3, so older boards keep working with a new app.

Audio is raw PCM on USB; Opus is unnecessary without Wi-Fi.

### 3. Wake word on the board, the rest on the host

- ESP-SR **AFE** (noise suppression, AEC if a reference exists, AGC) → **WakeNet** → **VAD** end-of-utterance.
- Wake word: ~~**"Hey Duo"**~~ **"Alexa"**, a built-in WakeNet9 model, with "Hi ESP" as the other choice (decided in M3: training a custom word is put off). "Hey Duo" is not one of WakeNet's built-in words ("Hi ESP", "Alexa", "Jarvis", "Computer", …), so it has to be trained:
  - **Preferred: [microWakeWord](https://github.com/kahrendt/microWakeWord)** (TensorFlow Lite Micro, used by ESPHome on ESP32-S3). It is trained on synthetic samples generated with Piper, so it is free and reproducible, and the model is about 50 KB. It runs after ESP-SR's AFE (or directly on the mic stream).
  - Alternative: Espressif's paid WakeNet customization service (needs a recorded dataset and has a turnaround time).
  - Bootstrap: develop M3 with a built-in WakeNet word ("Hi ESP") so the pipeline doesn't wait on training; swap in "Hey Duo" once its model meets the M3 targets.
- A wake word is language independent, so it works for Italian and English users alike.
- Optional later: a push-to-talk hotkey on the host, the app, or touch, for noisy rooms.

### 4. Host inference stack: llama.cpp family

| Component | Choice | Notes |
|-----------|--------|-------|
| LLM runtime | **llama.cpp `llama-server`** | Metal, CUDA, Vulkan, CPU; OpenAI-compatible API with native tool calling (`--jinja`); MIT |
| LLM model | Qwen3 1.7B / 4B, GGUF Q4_K_M (benchmark in M6) | Good multilingual quality and tool calling at this size; Apache-2.0. Re-evaluate newer small models at M6 time |
| STT | **whisper.cpp** (multilingual `base` / `small`, or `large-v3-turbo` on a GPU) | Same GGML ecosystem as llama.cpp; automatic IT/EN detection; MIT |
| TTS | **Piper** | Italian and English voices; **check each voice's license** (some are non-commercial) |
| VAD fallback | Silero VAD | Only if the board's VAD proves unreliable |

Integration: start with **sidecar processes** (`llama-server`, `whisper-server`, `piper`) managed by `dualeye-core`, which are easy to update and crash-isolated. Move to in-process bindings (`llama-cpp-2`, `whisper-rs`) only if packaging demands it.

Models are **downloaded on demand** by the app, not bundled. Sizes: Whisper small about 470 MB, Qwen3-4B Q4 about 2.5 GB, a Piper voice about 60 MB.

### 5. Language

Whisper detects IT or EN per utterance. The LLM system prompt says to reply in the user's language. Piper selects the voice by the detected language. Tool names and descriptions stay in English, which is what small models handle best.

## Latency budget (target, end of M6)

| Step | Target |
|------|--------|
| End of speech → VAD closes | 300–500 ms |
| STT (5 s utterance) | 300 ms (GPU) · 1–1.5 s (CPU) |
| LLM: tool call or short answer (about 50 tokens) | 0.5–1.5 s |
| TTS first audio | < 300 ms (sentence streaming) |
| **Total to first sound or action** | **≈ 1.5–3 s** |

Screen-only commands ("switch to rings") can skip TTS and just animate, which feels faster.

## Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| CPU/RAM contention between LVGL (2 displays) and the AFE + WakeNet | UI stutter, missed wake words | Pin audio to core 1 and LVGL to core 0; PSRAM for AFE buffers; measure in M3; lower the UI refresh rate while listening |
| ~~No AEC reference on this board~~ | — | Resolved in M0: the ES7210 channel 1 is a clean loopback |
| Low mic level at 30 dB gain | Weak wake-word and STT input | Max analog gain plus the AFE's AGC; re-measure in M3 |
| USB link reliability (host not reading, drops, reconnects) | Audio glitches, stuck states | Framing + CRC + resync; bounded ring buffers; reconnect tests in M1 |
| Small-model tool-calling accuracy | Wrong or no action | Few, well-described tools; constrained grammar or JSON schema in `llama-server`; eval set of IT/EN commands in M6 |
| Weak host (no GPU) | Slow replies | Smaller models (Whisper base, Qwen3 1.7B); capability check in the app |
| Protocol break with existing boards | App can't talk to older firmware | Version handshake plus legacy mode |
| Model and voice licenses | Redistribution issues | Download at runtime from the original source; license table in docs |
| Scope creep | Never ships | Every milestone ends in a usable release; voice is opt-in |

## Out of scope for now

Wi-Fi or standalone mode, on-board LLM, cloud STT/LLM/TTS, multiple boards on one host.

## Decisions

- Wake word: **"Alexa"** (WakeNet9, built in); "Hi ESP" selectable. ~~"Hey Duo", trained with microWakeWord~~: put off in M3.
- The MCP server also exposes **host tools** (metrics, Claude Code usage), so the assistant can answer "how hot is the GPU?".
- Voice is an **opt-in** toggle in the app until M7.

## Open questions

1. ~~Is microWakeWord accuracy on "Hey Duo" good enough with the board's mic and AFE? (M3 spike)~~ Put off: M3 ships a built-in word
