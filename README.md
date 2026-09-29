# ESP32-S3 DualEye PC Monitor

![Target](https://img.shields.io/badge/target-ESP32--S3-brightgreen)
![ESP-IDF](https://img.shields.io/badge/ESP--IDF-%3E%3D5.4-blue)
![LVGL](https://img.shields.io/badge/LVGL-9.3.0-orange)
![LCD](https://img.shields.io/badge/LCD-GC9A01%20×2-lightgrey)
![Host](https://img.shields.io/badge/host-Rust-purple)
![License](https://img.shields.io/badge/license-MIT-green)

PC monitoring on **ESP32-S3 DualEye** (two 240×240 round displays): CPU/GPU temperatures, load, fans, RAM and VRAM, plus Claude Code usage with its mascot, on Apple Watch–style faces you pick per screen.

A standalone Rust program on the host reads the sensors straight from the OS (no CoolerControl or other daemon) and sends them to the board over USB Serial/JTAG as one JSON line per second.

| Piece | Role |
|-------|------|
| Firmware (`main/`) | LVGL UI + metrics ingest |
| `host/dualeye-core` | Library: sensors → snapshot → USB serial. Meant to be embedded in a [Tauri](https://v2.tauri.app/) app |
| `host/dualeye-cli` | `dualeye` command-line bridge built on the library |
| `host/dualeye-app` | Desktop app ([Tauri 2](https://v2.tauri.app/) + Svelte): live mirror of both screens, history, sensors, board console, firmware flashing |

End users only need the desktop app installer, from the [latest release](https://github.com/EttoreCaputo/esp32s3-dualeye-pcmonitor/releases/latest) (Windows, macOS, Linux; the firmware is inside); everything below the app section is for working on the source (see [Building from source](#building-from-source)).

<p align="center">
  <img src="assets/image-1.png" alt="DualEye desktop app: live mirror of both round screens with CPU/GPU history" width="720">
</p>

<p align="center">
  <img src="assets/image-2.png" alt="Display settings: pick a watch face and rotation per screen" width="720">
</p>

**Stack:** ESP-IDF ≥ 5.4 · [lvgl/lvgl](https://components.espressif.com/components/lvgl/lvgl) `9.3.0` · [espressif/esp_lcd_gc9a01](https://components.espressif.com/components/espressif/esp_lcd_gc9a01) `^2.0.4` · Rust (sysinfo, nvml-wrapper, serialport, rmcp)

## Host bridge

Install Rust from [rustup.rs](https://rustup.rs), then close `idf.py monitor` (it holds the same port) and run:

```bash
cd host
cargo run --release              # auto-detects the board (USB 303a:xxxx) and streams
cargo run --release -- --once    # print one snapshot, no serial
cargo run --release -- --sensors # every raw sensor the backends can see
cargo run --release -- --cpu-face rings --gpu-face plus
cargo run --release -- --cpu-rotation 180 --gpu-rotation 180   # board upside down
cargo run --release -- tools                                      # the board's tools
cargo run --release -- call set_face --screen left --face rings
cargo run --release -- call show_text --text "Ciao!" --seconds 5
cargo run --release -- mcp                                        # MCP server on stdio, see below
cargo run --release -- --help
```

`tools` and `call` use the board's tools (firmware 0.4 or later, see [docs/protocol.md](docs/protocol.md)). While the app or a streaming `dualeye` runs they go through it; otherwise they open the port for the call and close it right after.

The binary ends up in `host/target/release/dualeye` (`dualeye.exe` on Windows) and has no runtime dependencies.

### What each OS provides

| Data | Linux | Windows | macOS |
|------|-------|---------|-------|
| CPU load, clock | ✓ | ✓ | ✓ |
| CPU temp (avg of all CPU sensors) | hwmon: coretemp, k10temp, zenpower | ACPI thermal zone (run as admin; many boards report nothing) | SMC / IOHID |
| CPU power | RAPL (see below) | — | — |
| NVIDIA GPU (temp, load, clock, power, fan RPM) | NVML | NVML | — |
| AMD GPU | hwmon `amdgpu` | — | — |
| Mac GPU (temp, load, memory) | — | — | SMC (`Tg*` keys on Apple Silicon, `TG*` on Intel), IOAccelerator |
| Fans | hwmon (`cpu` = fastest board fan, `gpu` = fastest GPU fan) | — | — |
| RAM | ✓ | ✓ | ✓ |
| VRAM | NVML, `amdgpu` (`mem_info_vram_*`) | NVML | IOAccelerator (Apple Silicon: GPU share of the unified RAM) |

Missing values are just left out of the snapshot; the board shows what it gets.

**Linux, serial access:** `sudo usermod -aG dialout "$USER"`, then log out and back in.

**Linux, CPU power:** `energy_uj` is root-only since the RAPL side-channel fix (CVE-2020-8694). To show package power anyway, make it world-readable at boot:

```bash
echo 'z /sys/class/powercap/intel-rapl:0/energy_uj 0444 - - -' | sudo tee /etc/tmpfiles.d/dualeye-rapl.conf
```

```bash
sudo systemd-tmpfiles --create /etc/tmpfiles.d/dualeye-rapl.conf
```

## Watch faces

Each screen shows one of six faces, chosen independently (left = CPU, right = GPU):

| Face | Shows |
|------|-------|
| `classic` | Temperature, clock, power, fan RPM; load on the ring (the default) |
| `rings` | Three rings, outside in: load, temperature (cyan, orange from 80 °C, red from 90 °C), memory; temperature and both percentages in the middle |
| `plus` | Classic, plus a bar under the load and fan row for RAM (left) or VRAM (right) with GiB used/total; orange from 90 % |
| `bar` | Classic with a smaller RAM/VRAM bar under the load and fan row, no numbers |
| `claude` | Claude Code: 5-hour limit used on the outer ring and in the middle, weekly limit on the inner ring (orange from 80 %, red from 95 %), time to the 5-hour reset, and a small Clawd. Without the status line: tokens in the 5-hour window, the window's progress on the ring, and today's tokens |
| `clawd` | Claude Code's mascot, large and animated: walks while Claude works, blinks when idle, sleeps after 30 min; model name, state and tokens in the window |

Pick them in the app (Settings → **Display**, saved across restarts), with `--cpu-face` / `--gpu-face` on the CLI, or with the `set_face` tool (`dualeye call set_face --screen left --face rings`). The board keeps them in NVS and boots showing them. When the bridge connects it adopts the board's faces and rotation (so a change made over MCP while the app was closed sticks), then sets them whenever you change them; the CLI pushes its own instead when you pass any face or rotation flag.

### Rotation

If the board sits another way round (upside down, on its side), turn each screen by 0°, 90°, 180° or 270° clockwise: in the app under Settings → **Display** (the mirror stays upright and marks a turned screen with a badge), or with `--cpu-rotation` / `--gpu-rotation` on the CLI. The panel itself is turned (GC9A01 MADCTL), so it costs no frame time. The board keeps the last rotation it got in NVS, so it boots the right way round before the host connects. The bridge sets it with the `set_rotation` tool.

### Claude Code faces

The host reads Claude Code's usage from two local sources, with nothing to set up for the first:

- **Transcripts** (`~/.claude/projects/**/*.jsonl`, or `$CLAUDE_CONFIG_DIR/projects`): tokens per reply (input, output and cache writes; cache reads are left out, they would swamp the figure), counted in Claude Code's 5-hour windows and since midnight. A transcript written in the last 20 s means Claude is working. The format is internal to Claude Code, so it is read leniently.
- **Status line**, for the plan's 5-hour and weekly limits (Pro and Max): Settings → **Display** → **Connect status line** sets `statusLine` in `~/.claude/settings.json` to `dualeye-app --claude-statusline` (a copy of the file is kept as `settings.json.dualeye-backup`). Claude Code then pipes its status JSON to the app, which keeps the latest copy and prints your previous status line (or a short default: model, 5h %, 7d %). **Disconnect** puts the previous one back. The CLI takes the same `--claude-statusline` flag.

## MCP server (Claude Code, Claude Desktop)

The board's tools, plus two that read this computer, are available to any [MCP](https://modelcontextprotocol.io) client over stdio, so you can ask Claude Code to "put the rings face on the left" or "show *build done* on the board":

| Tool | From | Does |
|------|------|------|
| `set_face`, `set_rotation`, `set_brightness`, `show_text`, `set_mic`, `set_wake_word`, `get_state` | Board | Passed through as the firmware describes them in `tools/list` ([docs/protocol.md](docs/protocol.md#board-tools)); a newer firmware's tools show up without a host update |
| `get_metrics` | Host | CPU and GPU temperature, load, clock, power, memory, fans |
| `get_claude_usage` | Host | Claude Code tokens in the 5-hour window and today, plan limits used, time to reset, working or idle |

With the **app** installed, Settings → **Display** → **MCP server** shows the command and config for this computer, with Copy buttons, and how many clients are connected. It is the app's own binary with `--mcp`. On macOS:

```bash
claude mcp add --scope user dualeye -- /Applications/DualEye.app/Contents/MacOS/dualeye-app --mcp
```

With the **CLI** on your `PATH`:

```bash
claude mcp add --scope user dualeye -- dualeye mcp
```

For Claude Desktop, add the same command to `claude_desktop_config.json` (Settings → Developer → Edit Config) and restart it:

```json
{ "mcpServers": { "dualeye": { "command": "/Applications/DualEye.app/Contents/MacOS/dualeye-app", "args": ["--mcp"] } } }
```

Only one process can hold the serial port. While the app (or `dualeye` streaming) runs, the MCP server goes through it over a local socket: the bridge listens on `127.0.0.1` and leaves the port and a random token in `hub.json` in DualEye's data folder (`~/Library/Application Support/dualeye` on macOS, `%APPDATA%\dualeye` on Windows, `~/.config/dualeye` on Linux), readable by you only. A face or rotation changed this way shows up in the app and is saved in its settings. When nothing streams, the MCP server opens the port for each call and closes it right after, so the app can still start. Any number of MCP clients can run at once.

The last list of board tools is kept in `board-tools.json` in the same folder, so they are listed even when the client starts while the board is unplugged; a call then says why it can't run. The server tells the client when the board's tools turn up later.

## Wake word (preview)

The board listens for a wake word with Espressif's [ESP-SR](https://github.com/espressif/esp-sr): the ES7210 mic and the speaker loopback go through its audio front end (echo cancellation, voice activity) into WakeNet, on core 1, with the UI on core 0. The word is **"Alexa"**, or **"Hi ESP"**: both are built-in WakeNet models, picked with the `set_wake_word` tool (`dualeye call set_wake_word --word hiesp`). When it hears it, a cyan ring lights round both screens (and in the app's mirror) for a few seconds, and the host gets a `wake` notification (`dualeye` prints it). Then the board streams what you say to the host over USB until you stop talking (a bright arc on the ring follows your voice), and the host can transcribe it in Italian or English with [whisper.cpp](https://github.com/ggml-org/whisper.cpp), running locally:

```bash
brew install whisper-cpp        # or build whisper-server from source
dualeye models download small   # about 490 MB, once
dualeye --stt                   # prints each transcript
```

In the desktop app it's Settings → **Voice**: turn it on, download a model (the app checks its SHA-256) and the transcripts show there. `dualeye models` lists the same models for the CLI (`dualeye models download small`). `--stt` takes one of them or a ggml model file, `--stt-language it|en` skips language detection, and `--voice-dump` keeps each utterance as a WAV file in `voice/` in DualEye's data folder. Acting on what you say comes in the next milestones. Nothing leaves your computer.

Mute the mic with the `set_mic` tool (`dualeye call set_mic --muted true`, or ask Claude through MCP); the board remembers both. The models sit in their own `model` partition ([partitions.csv](partitions.csv)), written by the app's flasher as part of the merged image.

## Desktop app

`host/dualeye-app` runs the same bridge in the background and shows, in real time, what the two round screens are displaying. The mirror follows `main/ui_watch.c` (ring geometry, fonts, colours, warm/hot/stale/waiting states), so keep them in sync when the firmware UI changes. It also plots 3 minutes of history and lists raw sensors, serial ports and the board's log. Closing the window hides it in the tray and streaming continues.

How to run and package it is in [Building from source](#building-from-source).

### Identify and flash the board

Settings → **Device** finds the ESP32-S3 on USB, reads its chip, MAC and flash size, and flashes the firmware. The image is `build/merged-binary.bin` (bootloader + partition table + app, written at `0x0`), embedded in the app at build time, so replace that file and rebuild the app to ship a new firmware (the build warns when a file in `main/` is newer than the image, i.e. `idf.py build merge-bin` wasn't rerun). It works on a blank board too, since esptool talks to the ROM bootloader; streaming pauses meanwhile and the board reboots afterwards.

Flashing uses [esptool](https://docs.espressif.com/projects/esptool/en/latest/esp32s3/index.html) and the user installs nothing: the first time Identify or Flash is used, the app

1. takes the system Python if it is 3.10+ with `venv` (Linux, Windows), otherwise downloads a portable CPython from [python-build-standalone](https://github.com/astral-sh/python-build-standalone), pinned and SHA-256 checked (never the system one on macOS, whose stub opens the Xcode installer);
2. creates a virtualenv and `pip install`s esptool 5.x into it.

That is about 45 MB downloaded once (internet needed only then) and ~170 MB on disk, in the app's local data folder under `esptool/` (`~/.local/share/com.dualeye.monitor/esptool` on Linux, `%LOCALAPPDATA%\com.dualeye.monitor\esptool` on Windows, `~/Library/Application Support/com.dualeye.monitor/esptool` on macOS). The output of the last setup is in `setup.log` there, and the Device tab shows it too when something fails. Delete that folder to force a fresh setup. The logic is in `dualeye-core` behind the `provision` feature, which only the app enables. The equivalent command line is:

```bash
esptool --chip esp32s3 --port /dev/ttyACM0 write-flash 0x0 build/merged-binary.bin
```

### Firmware updates

The firmware knows its version (`version.txt`, which ESP-IDF builds into the image's app descriptor) and prints it as `{"dualeye":"0.2.0","idf":"v6.1"}` when it boots and whenever it reads a `?version` line. The bridge asks every 5 s after connecting until the board answers; firmware from before 0.2.0 ignores the question but keeps logging the snapshots it gets, so after two unanswered questions it counts as "before 0.2.0". A ROM looping on `invalid header` means a blank flash.

The app reads the version of the image it carries from that same app descriptor. When the board runs an older one, a banner offers the update and lists what changes, taken from the sections of [`FIRMWARE_CHANGELOG.md`](FIRMWARE_CHANGELOG.md) newer than the board's version; **Update…** opens the Device tab to flash it.

To release a new firmware: bump `version.txt`, add its `## <version>` section to `FIRMWARE_CHANGELOG.md` (the release pipeline fails without it), and rebuild the image.

The app talks to the bridge through `dualeye_core::Bridge`; its events (`waiting`, `connected`, `snapshot`, `board_log`, `firmware`, `settings`, `disconnected`) are forwarded to the webview as `bridge`.

## Building from source

### Repository layout

```
main/                     firmware (ESP-IDF component: display, LVGL UI, metrics parser, audio, wake word)
build/merged-binary.bin   firmware image the desktop app flashes (the only tracked file in build/)
version.txt               firmware version, built into the image
FIRMWARE_CHANGELOG.md     what each firmware version changes, shown by the app's update offer
.github/workflows/        release pipeline
sdkconfig.defaults        firmware config (target esp32s3, 16 MB flash, USB Serial/JTAG console)
partitions.csv            flash layout: app, then the ESP-SR `model` partition
.devcontainer/            ESP-IDF container for VS Code
host/                     Cargo workspace
  dualeye-core/           sensors, Claude Code usage, snapshot, serial bridge, esptool setup/flash (library)
  dualeye-cli/            `dualeye` command-line bridge
  dualeye-app/            desktop app: Svelte UI in src/, Tauri shell in src-tauri/
```

### Prerequisites

| Part | Needs |
|------|-------|
| Firmware | [ESP-IDF](https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/get-started/) ≥ 5.4, or the dev container in `.devcontainer/` |
| CLI | [Rust](https://rustup.rs) ≥ 1.85 (edition 2024) |
| Desktop app | Rust, [Node.js](https://nodejs.org) ≥ 20.19, plus the platform packages below |

Desktop app, per platform (once):

- **Linux (Debian/Ubuntu):**

  ```bash
  sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev build-essential
  ```

  Other distros: see [Tauri's prerequisites](https://v2.tauri.app/start/prerequisites/). Add yourself to `dialout` for serial access (see above).
- **Windows:** [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with "Desktop development with C++" and the Rust MSVC toolchain. WebView2 ships with Windows 10/11.
- **macOS:** `xcode-select --install`.

### Firmware

```bash
idf.py set-target esp32s3   # first time only
idf.py build
idf.py -p /dev/ttyACM0 flash monitor
```

To update the image the desktop app ships, merge bootloader, partition table and app into one file and commit it:

```bash
idf.py merge-bin            # writes build/merged-binary.bin
```

`.gitignore` ignores `build/` except that file. The app embeds it at compile time (`include_bytes!` in `host/dualeye-app/src-tauri/src/lib.rs`), so rebuild the app afterwards.

### CLI

```bash
cd host
cargo build --release       # host/target/release/dualeye
```

Plain `cargo` commands in `host/` only touch `dualeye-core` and `dualeye-cli` (the workspace's default members), so they work without the GUI packages.

### Watch faces

Each screen shows one of six faces, chosen independently (left = CPU, right = GPU):

| Face | Shows |
|------|-------|
| `classic` | Temperature, clock, power, fan RPM; load on the ring (the default) |
| `rings` | Three rings, outside in: load, temperature (cyan, orange from 80 °C, red from 90 °C), memory; temperature and both percentages in the middle |
| `plus` | Classic, plus a bar under the load and fan row for RAM (left) or VRAM (right) with GiB used/total; orange from 90 % |
| `bar` | Classic with a smaller RAM/VRAM bar under the load and fan row, no numbers |
| `claude` | Claude Code: 5-hour limit used on the outer ring and in the middle, weekly limit on the inner ring (orange from 80 %, red from 95 %), time to the 5-hour reset, and a small Clawd. Without the status line: tokens in the 5-hour window, the window's progress on the ring, and today's tokens |
| `clawd` | Claude Code's mascot, large and animated: walks while Claude works, blinks when idle, sleeps after 30 min; model name, state and tokens in the window |

Pick them in the app (Settings → **Display**, saved across restarts), with `--cpu-face` / `--gpu-face` on the CLI, or with the `set_face` tool (`dualeye call set_face --screen left --face rings`). The board keeps them in NVS and boots showing them. When the bridge connects it adopts the board's faces and rotation (so a change made over MCP while the app was closed sticks), then sets them whenever you change them; the CLI pushes its own instead when you pass any face or rotation flag.

### Rotation

If the board sits another way round (upside down, on its side), turn each screen by 0°, 90°, 180° or 270° clockwise: in the app under Settings → **Display** (the mirror stays upright and marks a turned screen with a badge), or with `--cpu-rotation` / `--gpu-rotation` on the CLI. The panel itself is turned (GC9A01 MADCTL), so it costs no frame time. The board keeps the last rotation it got in NVS, so it boots the right way round before the host connects. The bridge sets it with the `set_rotation` tool.

## Desktop app

```bash
cd host/dualeye-app
npm install
npm run dev                 # UI only, in a browser, with synthetic data: no board or Rust needed
npm run tauri dev           # the real app, hot reload on UI changes
npm run tauri build         # release build + installers
```

`npm run tauri build` packages for the OS it runs on (Tauri does not cross-compile), into `host/target/release/bundle/`:

| Built on | Output |
|----------|--------|
| Linux | `deb/*.deb`, `rpm/*.rpm`, `appimage/*.AppImage` |
| Windows | `msi/*.msi`, `nsis/*-setup.exe` |
| macOS | `macos/*.app`, `dmg/*.dmg` (`-- --target universal-apple-darwin` for Intel + Apple Silicon, after `rustup target add aarch64-apple-darwin x86_64-apple-darwin`) |

To ship all three from one place, run the build on a CI matrix (e.g. GitHub Actions with [tauri-action](https://github.com/tauri-apps/tauri-action)). Unsigned builds work but Windows SmartScreen and macOS Gatekeeper warn about them; signing needs a code-signing certificate / Apple Developer ID.

Where things live in the app:

| What | Where |
|------|-------|
| Screen mirror (keep in sync with `main/ui_watch.c`) | `src/lib/Board.svelte`, `Eye.svelte`, `firmware.ts` |
| Live state, bridge/flash events, synthetic preview feed | `src/lib/monitor.svelte.ts` |
| Settings drawer (Connection, Display, Device, Sensors, Console) | `src/lib/Drawer.svelte` |
| Tauri commands, tray, bridge lifecycle | `src-tauri/src/lib.rs` |
| Claude Code usage and the status line helper | `host/dualeye-core/src/claude.rs`, `claude/statusline.rs` |
| esptool runner and first-use setup | `host/dualeye-core/src/flasher.rs`, `flasher/setup.rs` |

### Releases

Every push to `main` runs [`.github/workflows/release.yml`](.github/workflows/release.yml): it builds the firmware with ESP-IDF v6.1 (`idf.py build merge-bin`), checks that the image carries `version.txt` and that the changelog has a section for it, then builds the app on Linux (AppImage, deb, rpm), Windows (msi, NSIS) and macOS (dmg, Apple Silicon and Intel) with that fresh image embedded, and uploads everything, plus the bare firmware image, to the GitHub release `v<version>` from `src-tauri/tauri.conf.json`. Bump that version to cut a new release; until then each merge replaces the assets of the current one. The builds aren't code-signed, so macOS and Windows warn on first launch.

### Checks

```bash
cd host && cargo test -p dualeye-core --features provision
```

```bash
cd host && cargo test -p dualeye-app      # the bundled image carries version.txt
```

```bash
cd host/dualeye-app && npm run check
```

## Wire format

Since firmware 0.4.0 the board and the host speak **protocol v2**: COBS frames with a channel byte and a CRC, carrying JSON-RPC (`hello`, `tools/list`, `tools/call`), sensor snapshots and the board's log on one USB link. The full spec, with the board tools, is in [docs/protocol.md](docs/protocol.md). Firmware 0.3 and older spoke newline JSON and must be updated (the app offers it).

A `metrics` frame carries one snapshot:

```json
{"v":2,"ts":1790419114,"cpu":{"temp_c":40.2,"load_pct":2.8,"clock_mhz":1210,"power_w":14.6,"mem":{"used_mb":12568,"total_mb":62277}},"gpu":{"temp_c":35.0,"load_pct":0.0,"clock_mhz":210,"power_w":22.1,"mem":{"used_mb":14,"total_mb":24576}},"fans":[{"id":"cpu","rpm":3824},{"id":"gpu","rpm":0}]}
```

Parsed by `main/metrics_parser.c`; snapshots without any temperature are ignored, and the UI goes stale after 3 s without data. `mem` is in MiB: system RAM under `cpu`, VRAM under `gpu`. Faces and rotation are no longer in the snapshot: they are board state, set with tools. The bridge adds `claude` when Claude Code has run on this machine:

```json
"claude":{"tok":1234567,"today":4500000,"left_min":133,"s_pct":42.0,"w_pct":18.0,"state":"work","model":"OPUS 5.5"}
```

`tok` and `today` are tokens in the 5-hour window and since local midnight, `left_min` the minutes until the window resets, `s_pct` / `w_pct` the 5-hour and weekly limits used (only with the status line connected), `state` one of `work`, `idle`, `sleep`.

## License

MIT — see [LICENSE](LICENSE).
