# Licenses

DualEye itself (firmware, desktop app, CLI) is MIT-licensed, see [LICENSE](../LICENSE). This page lists what comes with it, or what it fetches, and under which terms. Everything in it runs on your computer or on the board; nothing is sent anywhere else.

## Shipped with the desktop app

| Component | Version | License | How |
|-----------|---------|---------|-----|
| [whisper.cpp](https://github.com/ggml-org/whisper.cpp) `whisper-server` | v1.9.4 | MIT | Built by `tools/build_sidecars.sh`, bundled next to the app; speech-to-text |
| [llama.cpp](https://github.com/ggml-org/llama.cpp) `llama-server` | b11146 | MIT | Same; the voice agent's language model |
| [Tauri](https://tauri.app) and the Rust crates in `host/Cargo.lock` | | MIT or Apache-2.0 (a few: BSD, ISC, Zlib, Unicode-3.0) | Linked into the app |
| [Svelte](https://svelte.dev) | | MIT | The app's interface |

The license texts of whisper.cpp and llama.cpp are in the app's resources (`licenses/`). The two servers run as separate processes on `127.0.0.1`.

## Fetched on first use

| Component | License | When |
|-----------|---------|------|
| Python from [python-build-standalone](https://github.com/astral-sh/python-build-standalone) | PSF-2.0 | The first time the app flashes the board or installs Piper, into DualEye's data folder |
| [esptool](https://github.com/espressif/esptool) | GPL-2.0-or-later | With it, to flash the board; run as a separate process |
| [Piper](https://github.com/OHF-Voice/piper1-gpl) (`piper-tts` 1.8.0) with onnxruntime (MIT) and espeak-ng (GPL-3.0) | GPL-3.0 | Only when you press *Install* in the Voice tab (or run `dualeye piper install`), into a virtualenv; run as a separate process, never linked into DualEye |

## Models, downloaded when you pick them

Each file is pinned by size and SHA-256 in `host/dualeye-core/src/models.rs`.

| Model | Use | License |
|-------|-----|---------|
| Whisper `base`, `small`, `large-v3-turbo-q5_0` ([ggerganov/whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp)) | Speech-to-text | MIT (OpenAI's weights) |
| Qwen3 4B Instruct 2507, Qwen3.5 2B and 4B (GGUF by [Unsloth](https://huggingface.co/unsloth)) | Language model | Apache-2.0 |
| Piper voice `it_IT-paola-medium` | Italian voice | Dataset CC0 1.0 (paolapersico1/Voice-Dataset-Italian); fine-tuned from lessac |
| Piper voice `it_IT-riccardo-x_low` | Italian voice | Dataset M-AILABS (BSD-style) |
| Piper voice `en_GB-alba-medium` | English voice | Dataset CC BY 4.0 (Edinburgh DataShare 10283/3270); fine-tuned from lessac |
| Piper voice `en_US-ljspeech-medium` | English voice | Dataset public domain (LJ Speech) |

The app shows each model's license next to it in the Voice tab.

## In the firmware

| Component | License |
|-----------|---------|
| [ESP-IDF](https://github.com/espressif/esp-idf) 6.1 | Apache-2.0 |
| [ESP-SR](https://github.com/espressif/esp-sr) (audio front end, WakeNet and its "Alexa" and "Hi ESP" models) | ESPRESSIF MIT License: use limited to Espressif chips |
| esp-dl, dl_fft | MIT |
| esp_new_jpeg | ESPRESSIF MIT License |
| esp_codec_dev, esp_lcd_gc9a01, cmake_utilities | Apache-2.0 |
| [LVGL](https://lvgl.io) | MIT |
| cJSON | MIT |
