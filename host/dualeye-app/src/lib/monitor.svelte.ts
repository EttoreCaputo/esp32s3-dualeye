// Live model of the bridge and of what the board is showing right now.
//
// In the Tauri app the data comes from the Rust side (`dualeye-core`); opened
// in a plain browser (`npm run dev`) it falls back to a synthetic feed so the
// UI can be worked on without hardware.

import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { DEFAULT_FACES, DEFAULT_ROTATIONS, type Faces, type Rotations } from "./firmware";
import { updateFor, type Update } from "./updates";
import bundledVersion from "../../../../version.txt?raw";

/** In MiB. System RAM under `cpu`, VRAM under `gpu`. */
export type Memory = { used_mb: number; total_mb: number };
export type Metrics = { temp_c?: number; load_pct?: number; clock_mhz?: number; power_w?: number; mem?: Memory };
export type Fan = { id: string; rpm: number };
export type ClaudeState = "work" | "idle" | "sleep";
/** `ClaudeMetrics` in dualeye-core: tokens in the 5-hour window and today, limits from the status line. */
export type ClaudeMetrics = {
  tok: number;
  today: number;
  left_min?: number;
  s_pct?: number;
  w_pct?: number;
  state: ClaudeState;
  model?: string;
};
export type ClaudeLink = { connected: boolean; chained: string | null; last_update_s: number | null; settings_path: string | null };
export type Snapshot = { v: number; ts: number; cpu?: Metrics; gpu?: Metrics; fans?: Fan[]; face?: Faces; rot?: Rotations; claude?: ClaudeMetrics };
export type PortInfo = { name: string; vid: number; pid: number; product: string | null; is_board: boolean };
export type Reading = { source: string; label: string; value: number; unit: string };
export type Esptool = { python: string; version: string };
/** The app descriptor of the bundled image. */
export type ImageInfo = { version: string; project: string; idf: string; built: string };
export type FirmwareInfo = { size: number; bundled: ImageInfo | null; esptool: Esptool | null };
/** `BoardFirmware` in dualeye-core: what the board said it runs. */
export type BoardFirmware = { state: "version"; version: string; idf: string | null } | { state: "legacy" } | { state: "missing" };
export type ChipInfo = {
  port: string;
  chip: string | null;
  features: string | null;
  crystal: string | null;
  mac: string | null;
  flash_size: string | null;
};
/** What esptool is doing with the board, if anything. */
export type DeviceJob = "idle" | "identify" | "flash";

type FlashEvent =
  | { kind: "log"; line: string }
  | { kind: "progress"; percent: number }
  | { kind: "setup"; message: string; percent: number | null };

type BridgeEvent =
  | { kind: "waiting"; reason: string }
  | { kind: "connected"; port: string }
  | { kind: "snapshot"; snapshot: Snapshot; sent: boolean }
  | { kind: "board_log"; line: string }
  | { kind: "firmware"; firmware: BoardFirmware }
  | { kind: "disconnected"; port: string; reason: string; permission_denied: boolean };

type Status = {
  link: Link;
  port: string | null;
  message: string | null;
  last: Snapshot | null;
  sent: Snapshot | null;
  sent_age_ms: number | null;
  connected_age_ms: number | null;
  firmware: BoardFirmware | null;
  logs: string[];
  port_setting: string | null;
  faces: Faces;
  rotation: Rotations;
};

export type Link = "searching" | "connected" | "offline";
/** Mirrors `metrics_ui_state_t` plus the moments the firmware is not running the UI. */
export type BoardState = "off" | "boot" | "waiting" | "live" | "stale";
export type Sample = { t: number; cpuT?: number; cpuL?: number; gpuT?: number; gpuL?: number };

/** `METRICS_STALE_MS_DEFAULT` in main/metrics_model.h. */
export const STALE_MS = 3000;
/** Opening the port resets the S3; the UI is up again after about this long. */
const BOOT_MS = 1100;
const HISTORY = 180;
const LOG_LINES = 300;

class Monitor {
  readonly preview = !isTauri();

  link = $state<Link>("searching");
  port = $state<string | null>(null);
  portSetting = $state<string | null>(null);
  message = $state("");
  permissionDenied = $state(false);
  /** Latest sample taken on the host. */
  last = $state<Snapshot | null>(null);
  /** Latest line actually written to the board: what its screens hold. */
  shown = $state<Snapshot | null>(null);
  sentAt = $state(0);
  connectedAt = $state(0);
  now = $state(Date.now());
  history = $state<Sample[]>([]);
  logs = $state<string[]>([]);
  /** Faces picked in the app; the board switches with the next line it gets. */
  faces = $state<Faces>({ ...DEFAULT_FACES });
  /** How each screen is turned; also applies from the next line. */
  rotation = $state<Rotations>({ ...DEFAULT_ROTATIONS });

  job = $state<DeviceJob>("idle");
  /** Output of the last esptool run. */
  jobLog = $state<string[]>([]);
  jobError = $state("");
  flashPercent = $state(0);
  flashedAt = $state(0);
  /** First-use setup of esptool (Python download, virtualenv, pip), while it runs. */
  setup = $state<{ message: string; percent: number | null } | null>(null);
  chip = $state<ChipInfo | null>(null);
  /** What the connected board runs; `null` until it says (or if it never does). */
  boardFirmware = $state<BoardFirmware | null>(null);
  /** The firmware the app flashes. */
  bundled = $state<ImageInfo | null>(null);
  /** Set when the board runs older DualEye firmware than the bundled one. */
  update: Update | null = $derived(this.link === "connected" ? updateFor(this.boardFirmware, this.bundled?.version) : null);

  boardState: BoardState = $derived.by(() => {
    if (this.link !== "connected") return "off";
    if (this.now - this.connectedAt < BOOT_MS) return "boot";
    if (!this.shown || this.sentAt < this.connectedAt) return "waiting";
    return this.now - this.sentAt > STALE_MS ? "stale" : "live";
  });

  #started = false;

  start() {
    if (this.#started) return;
    this.#started = true;
    setInterval(() => (this.now = Date.now()), 250);
    if (this.preview) startPreviewFeed((e) => this.#apply(e), () => this.faces, () => this.rotation);
    else void this.#connect();
    void this.firmwareInfo();
  }

  async #connect() {
    await listen<BridgeEvent>("bridge", (e) => this.#apply(e.payload));
    await listen<FlashEvent>("flash", (e) => this.#applyFlash(e.payload));
    const s = await invoke<Status>("status");
    const now = Date.now();
    this.link = s.link;
    this.port = s.port;
    this.portSetting = s.port_setting;
    this.faces = s.faces;
    this.rotation = s.rotation;
    this.message = s.message ?? "";
    this.last = s.last;
    this.shown = s.sent;
    if (s.sent_age_ms != null) this.sentAt = now - s.sent_age_ms;
    if (s.connected_age_ms != null) this.connectedAt = now - s.connected_age_ms;
    this.boardFirmware = s.firmware;
    this.logs = s.logs;
  }

  #apply(e: BridgeEvent) {
    const now = Date.now();
    switch (e.kind) {
      case "waiting":
        this.link = "searching";
        this.message = e.reason;
        break;
      case "connected":
        this.link = "connected";
        this.port = e.port;
        this.message = "";
        this.permissionDenied = false;
        this.connectedAt = now;
        this.shown = null;
        this.boardFirmware = null;
        break;
      case "snapshot":
        this.last = e.snapshot;
        if (e.sent) {
          this.shown = e.snapshot;
          this.sentAt = now;
        }
        this.#record(now, e.snapshot);
        break;
      case "board_log":
        this.logs.push(e.line);
        if (this.logs.length > LOG_LINES) this.logs.splice(0, this.logs.length - LOG_LINES);
        break;
      case "firmware":
        this.boardFirmware = e.firmware;
        break;
      case "disconnected":
        this.link = "offline";
        this.message = e.reason;
        this.permissionDenied = e.permission_denied;
        break;
    }
  }

  #applyFlash(e: FlashEvent) {
    if (e.kind === "progress") {
      this.setup = null;
      this.flashPercent = e.percent;
    } else if (e.kind === "setup") {
      this.setup = { message: e.message, percent: e.percent };
      // Keep the setup's own output (venv, pip) so a failure can be read back.
      if (e.percent === null) this.jobLog.push(e.message);
    } else {
      this.setup = null;
      this.jobLog.push(e.line);
    }
  }

  async firmwareInfo(): Promise<FirmwareInfo> {
    const info: FirmwareInfo = this.preview
      ? { size: 559360, bundled: { version: bundledVersion.trim(), project: "esp32s3-dualeye-pcmonitor", idf: "v6.1", built: "Sep 26 2026 16:05:04" }, esptool: previewEsptool }
      : await invoke<FirmwareInfo>("firmware_info");
    this.bundled = info.bundled;
    return info;
  }

  /** Ask the chip who it is (resets the board). `port` null picks the detected board. */
  async identify(port: string | null) {
    await this.#runJob("identify", async () => {
      this.chip = this.preview ? await previewIdentify((e) => this.#applyFlash(e)) : await invoke<ChipInfo>("identify_board", { port });
    });
  }

  /** Write the bundled firmware and reboot the board into it. */
  async flash(port: string | null) {
    this.flashPercent = 0;
    await this.#runJob("flash", async () => {
      if (this.preview) await previewFlash((e) => this.#applyFlash(e), (e) => this.#apply(e));
      else await invoke("flash_board", { port });
      this.flashedAt = Date.now();
    });
  }

  async #runJob(job: DeviceJob, run: () => Promise<void>) {
    if (this.job !== "idle") return;
    this.job = job;
    this.jobLog = [];
    this.jobError = "";
    this.flashedAt = 0;
    try {
      await run();
    } catch (err) {
      this.jobError = String(err);
    } finally {
      this.job = "idle";
      this.setup = null;
    }
  }

  #record(t: number, s: Snapshot) {
    this.history.push({ t, cpuT: s.cpu?.temp_c, cpuL: s.cpu?.load_pct, gpuT: s.gpu?.temp_c, gpuL: s.gpu?.load_pct });
    if (this.history.length > HISTORY) this.history.splice(0, this.history.length - HISTORY);
  }

  async listPorts(): Promise<PortInfo[]> {
    if (this.preview) return [{ name: "/dev/ttyACM0", vid: 0x303a, pid: 0x1001, product: "USB JTAG/serial debug unit", is_board: true }];
    return invoke<PortInfo[]>("list_ports");
  }

  async setPort(port: string | null) {
    this.portSetting = port;
    if (!this.preview) await invoke("set_port", { port });
  }

  async setFaces(faces: Faces) {
    this.faces = faces;
    if (!this.preview) await invoke("set_faces", { faces });
  }

  async setRotation(rotation: Rotations) {
    this.rotation = rotation;
    if (!this.preview) await invoke("set_rotation", { rotation });
  }

  async claudeLink(): Promise<ClaudeLink> {
    if (this.preview) return previewClaudeLink;
    return invoke<ClaudeLink>("claude_link");
  }

  async claudeConnect(connect: boolean): Promise<ClaudeLink> {
    if (this.preview) {
      previewClaudeLink = { ...previewClaudeLink, connected: connect, last_update_s: connect ? 3 : null };
      return previewClaudeLink;
    }
    return invoke<ClaudeLink>(connect ? "claude_connect" : "claude_disconnect");
  }

  async readings(): Promise<Reading[]> {
    if (this.preview) return previewReadings(this.last);
    return invoke<Reading[]>("readings");
  }
}

export const monitor = new Monitor();

export function fanRpm(s: Snapshot | null, id: string): number | undefined {
  return s?.fans?.find((f) => f.id === id)?.rpm;
}

// ── Preview feed ────────────────────────────────────────────────────────────

let previewClaudeLink: ClaudeLink = { connected: false, chained: null, last_update_s: null, settings_path: "~/.claude/settings.json" };

function startPreviewFeed(emit: (e: BridgeEvent) => void, faces: () => Faces, rotation: () => Rotations) {
  const boot = [
    "ESP-ROM:esp32s3-20210327",
    "I (24) boot: ESP-IDF v6.1 2nd stage bootloader",
    "I (810) board_display: Dual GC9A01 ready (L:+90 CCW, R:+90 CW)",
    "I (890) ui_watch: Watch UI created",
    "I (900) metrics_io: Reading snapshot JSON from USB serial",
    "I (900) dualeye: Watch UI ready, waiting for USB metrics",
  ];
  setTimeout(() => emit({ kind: "connected", port: "/dev/ttyACM0" }), 600);
  boot.forEach((line, i) => setTimeout(() => emit({ kind: "board_log", line }), 900 + i * 90));
  // Firmware from before versioning, so the update offer shows up.
  setTimeout(() => emit({ kind: "firmware", firmware: { state: "legacy" } }), 8000);

  const t0 = performance.now();
  const wave = (t: number, period: number, phase = 0) => Math.sin((t / period) * Math.PI * 2 + phase);
  setTimeout(() => {
    setInterval(() => {
      const t = (performance.now() - t0) / 1000;
      // A slow "workload" envelope with bursts so every colour state shows up.
      const burst = Math.max(0, wave(t, 47)) ** 3;
      const cpuLoad = clamp(6 + 30 * burst + 8 * Math.abs(wave(t, 5.3)) + Math.random() * 4, 0, 100);
      const gpuLoad = clamp(3 + 92 * Math.max(0, wave(t, 31, 1.2)) ** 2 + Math.random() * 3, 0, 100);
      const snapshot: Snapshot = {
        v: 1,
        ts: Math.floor(Date.now() / 1000),
        cpu: {
          temp_c: r1(40 + cpuLoad * 0.48 + wave(t, 13) * 1.5),
          load_pct: r1(cpuLoad),
          clock_mhz: Math.round(900 + cpuLoad * 42 + Math.random() * 120),
          power_w: r1(9 + cpuLoad * 1.6),
          mem: { used_mb: Math.round(12400 + cpuLoad * 60 + wave(t, 90) * 900), total_mb: 31744 },
        },
        gpu: {
          temp_c: r1(34 + gpuLoad * 0.5),
          load_pct: r1(gpuLoad),
          clock_mhz: gpuLoad > 8 ? Math.round(1400 + gpuLoad * 5) : 210,
          power_w: r1(21 + gpuLoad * 3.3),
          mem: { used_mb: Math.round(1100 + gpuLoad * 190), total_mb: 24576 },
        },
        fans: [
          { id: "cpu", rpm: Math.round(3780 + cpuLoad * 9 + Math.random() * 40) },
          { id: "gpu", rpm: gpuLoad > 25 ? Math.round(900 + gpuLoad * 14) : 0 },
        ],
        face: { ...faces() },
        // Like the bridge: left out when both screens are upright.
        ...(rotation().cpu || rotation().gpu ? { rot: { ...rotation() } } : {}),
        // Claude works in bursts and naps between them.
        claude: {
          tok: Math.round(820_000 + t * 2400),
          today: Math.round(3_900_000 + t * 2400),
          left_min: Math.max(0, 133 - Math.floor(t / 60)),
          ...(previewClaudeLink.connected ? { s_pct: Math.min(100, Math.round(42 + t / 20)), w_pct: 18 } : {}),
          state: t % 90 < 50 ? "work" : t % 90 < 80 ? "idle" : "sleep",
          model: "OPUS 5.5",
        },
      };
      emit({ kind: "snapshot", snapshot, sent: true });
      const line = `I (${Math.round(t * 1000 + 2000)}) metrics_io: cpu ${Math.round(snapshot.cpu!.temp_c!)}C gpu ${Math.round(snapshot.gpu!.temp_c!)}C`;
      emit({ kind: "board_log", line });
    }, 1000);
  }, 2600);
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// The preview walks through the first-use setup once, like a fresh install.
let previewEsptool: Esptool | null = null;

async function previewSetup(emit: (e: FlashEvent) => void) {
  if (previewEsptool) return;
  for (let p = 0; p <= 100; p += 5) {
    emit({ kind: "setup", message: "Downloading Python 3.12.14", percent: p });
    await sleep(60);
  }
  for (const message of ["Unpacking Python", "Creating the virtual environment", "Collecting esptool>=5.1,<6", "Successfully installed esptool-5.4.0"]) {
    emit({ kind: "setup", message, percent: null });
    await sleep(500);
  }
  previewEsptool = { python: "~/.local/share/com.dualeye.monitor/esptool/venv/bin/python", version: "5.4.0" };
}

async function previewIdentify(emit: (e: FlashEvent) => void): Promise<ChipInfo> {
  await previewSetup(emit);
  for (const line of ["esptool v5.4.0", "Connected to ESP32-S3 on /dev/ttyACM0:", "Chip type: ESP32-S3 (QFN56) (revision v0.2)"]) {
    emit({ kind: "log", line });
    await sleep(250);
  }
  return {
    port: "/dev/ttyACM0",
    chip: "ESP32-S3 (QFN56) (revision v0.2)",
    features: "Wi-Fi, BT 5 (LE), Dual Core + LP Core, 240MHz, Embedded PSRAM 8MB (AP_3v3)",
    crystal: "40MHz",
    mac: "dc:da:0c:2a:91:f4",
    flash_size: "16MB",
  };
}

async function previewFlash(emit: (e: FlashEvent) => void, bridge: (e: BridgeEvent) => void) {
  await previewSetup(emit);
  for (const line of ["esptool v5.4.0", "Connected to ESP32-S3 on /dev/ttyACM0:", "Flash will be erased from 0x00000000 to 0x00088fff..."]) {
    emit({ kind: "log", line });
    await sleep(300);
  }
  for (let p = 0; p <= 100; p += 4) {
    emit({ kind: "progress", percent: p });
    await sleep(120);
  }
  for (const line of ["Wrote 559360 bytes (321722 compressed) at 0x00000000 in 4.1 seconds.", "Hash of data verified.", "Hard resetting via RTS pin..."]) {
    emit({ kind: "log", line });
    await sleep(200);
  }
  bridge({ kind: "connected", port: "/dev/ttyACM0" });
  setTimeout(() => bridge({ kind: "firmware", firmware: { state: "version", version: bundledVersion.trim(), idf: "v6.1" } }), 1200);
}

function previewReadings(s: Snapshot | null): Reading[] {
  const c = s?.cpu ?? {};
  const g = s?.gpu ?? {};
  const out: Reading[] = [
    { source: "coretemp (hwmon4)", label: "Package id 0", value: (c.temp_c ?? 40) + 2, unit: "°C" },
  ];
  [0, 4, 12, 13, 14, 15, 16, 20].forEach((core, i) =>
    out.push({ source: "coretemp (hwmon4)", label: `Core ${core}`, value: (c.temp_c ?? 40) - 1 + (i % 3), unit: "°C" }),
  );
  out.push(
    { source: "nct6799 (hwmon6)", label: "SYSTIN", value: 35, unit: "°C" },
    { source: "nct6799 (hwmon6)", label: "CPUTIN", value: 37, unit: "°C" },
    { source: "nct6799 (hwmon6)", label: "fan1", value: 841, unit: "RPM" },
    { source: "nct6799 (hwmon6)", label: "fan2", value: 561, unit: "RPM" },
    { source: "nct6799 (hwmon6)", label: "fan7", value: fanRpm(s, "cpu") ?? 3813, unit: "RPM" },
    { source: "nvml:0 NVIDIA GeForce RTX 3090", label: "GPU Temp", value: g.temp_c ?? 34, unit: "°C" },
    { source: "nvml:0 NVIDIA GeForce RTX 3090", label: "GPU Load", value: g.load_pct ?? 0, unit: "%" },
    { source: "nvml:0 NVIDIA GeForce RTX 3090", label: "Power", value: g.power_w ?? 21, unit: "W" },
    { source: "nvml:0 NVIDIA GeForce RTX 3090", label: "VRAM used", value: g.mem?.used_mb ?? 1100, unit: "MB" },
    { source: "nvml:0 NVIDIA GeForce RTX 3090", label: "VRAM total", value: 24576, unit: "MB" },
    { source: "memory", label: "RAM used", value: c.mem?.used_mb ?? 12400, unit: "MB" },
    { source: "memory", label: "RAM total", value: 31744, unit: "MB" },
  );
  return out;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const r1 = (v: number) => Math.round(v * 10) / 10;
