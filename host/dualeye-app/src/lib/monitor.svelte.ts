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
/** `HubStatus` in dualeye-core: MCP servers using the board through the app. */
export type HubStatus = { clients: number; calls: number; last_tool: string | null; last_call_age_ms: number | null };
/** How MCP clients start the server (this app with `--mcp`), and who is using it. */
export type McpInfo = { command: string | null; args: string[]; hub: HubStatus | null; hub_error: string | null };
export type ClaudeLink = { connected: boolean; chained: string | null; last_update_s: number | null; settings_path: string | null };
/** `AlertSettings` in dualeye-core: which Claude Code alerts the board gives. */
export type ClaudeAlertSettings = {
  needs_you: boolean;
  done: boolean;
  done_after_s: number;
  usage: boolean;
  speak: boolean;
  language: string;
};
export type ClaudeAlertsInfo = {
  settings: ClaudeAlertSettings;
  hooks: { connected: boolean; last_event_s: number | null; settings_path: string | null };
  can_speak: boolean;
  last: { text: string; error: string | null; age_s: number } | null;
};
/** Bytes per second, every interface but loopback. */
export type Net = { rx_bps: number; tx_bps: number };
/** The system disk, in GB; throughput where the OS tells. */
export type Disk = { used_gb: number; total_gb: number; read_bps?: number; write_bps?: number };
export type Battery = { pct: number; charging: boolean; plugged: boolean; mins?: number };
export type TimerKind = "timer" | "work" | "break" | "reminder";
/** `BoardTimer` in dualeye-core: the timer the board's timer face shows. */
export type BoardTimer = {
  kind: TimerKind;
  state: "run" | "pause" | "ring";
  left_s: number;
  total_s: number;
  label?: string;
  more?: number;
  round?: number;
  rounds?: number;
  /** The screen it takes over while it runs. */
  screen?: Side;
};
/** `BoardMusic` in dualeye-core: what the music face shows (firmware 1.3). */
export type BoardMusic = { state: "play" | "pause"; title: string; artist?: string; pos_s?: number; dur_s?: number; art?: number };
export type MusicAction = "play" | "pause" | "toggle" | "next" | "previous";
/** Where the mouse pointer is, -1..1 each way across all screens, and when it last moved (ms). */
export type Gaze = { x: number; y: number; movedAt: number };
/** `TimerInfo` in dualeye-core: one timer as the Timers tab lists it. */
export type TimerInfo = { id: number; kind: TimerKind; label?: string; total_s: number; left_s: number; state: "run" | "pause" | "ring"; ends_at?: string };
export type ShowOn = "left" | "right" | "none";
export type TimersInfo = {
  timers: TimerInfo[];
  pomodoro: { work_s: number; break_s: number; rounds: number; round: number } | null;
  show_on: ShowOn;
  can_speak: boolean;
};
export type Snapshot = {
  v: number;
  ts: number;
  cpu?: Metrics;
  gpu?: Metrics;
  fans?: Fan[];
  net?: Net;
  disk?: Disk;
  bat?: Battery;
  face?: Faces;
  rot?: Rotations;
  claude?: ClaudeMetrics;
  timer?: BoardTimer;
  music?: BoardMusic;
};
/** `Prepared` in dualeye-core: what went to the board. */
export type ImageSent = { frames: number; source_frames: number; bytes: number; duration_ms: number };
/** An image face's screen, as `media/...` names it. */
export type Side = "left" | "right";
export type PortInfo = { name: string; vid: number; pid: number; product: string | null; is_board: boolean };
export type Reading = { source: string; label: string; value: number; unit: string };
/** `power_helper::HelperStatus`: the macOS root helper that reads the exact CPU power. */
export type PowerHelperState = "unavailable" | "off" | "needs_approval" | "on";
export type PowerHelper = { state: PowerHelperState; error: string | null };
export type Esptool = { python: string; version: string };
/** The app descriptor of the bundled image. */
export type ImageInfo = { version: string; project: string; idf: string; built: string };
export type FirmwareInfo = { size: number; bundled: ImageInfo | null; esptool: Esptool | null };
/** `BoardFirmware` in dualeye-core: what the board said it runs. */
/** `protocol` 1 is firmware before 0.4.0, which this app can only offer to update. */
export type BoardFirmware = { state: "version"; version: string; idf: string | null; protocol: number } | { state: "legacy" } | { state: "missing" };
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
  | { kind: "settings"; faces: Faces; rotation: Rotations }
  | { kind: "wake"; word: string; volume_db: number | null }
  | { kind: "voice_state"; state: VoiceState }
  | { kind: "listening"; id: number; trigger: string }
  | { kind: "utterance"; utterance: Utterance; duration_ms: number; peak_db: number | null; wav: string | null }
  | { kind: "transcript"; id: number; transcript: Transcript | null }
  | { kind: "reply"; id: number; text: string; language: string; actions: string[]; understood: boolean; by: ReplyBy; elapsed_ms: number }
  | { kind: "spoken"; id: number; spoken: Spoken }
  | { kind: "voice_error"; message: string }
  | { kind: "timer_fired"; text: string; error: string | null }
  | { kind: "disconnected"; port: string; reason: string; permission_denied: boolean };

/** What the board's "eyes" overlay shows. */
export type VoiceState = "idle" | "listening" | "thinking" | "speaking" | "error";
export type Utterance = { id: number; trigger: string; reason: string; speech: boolean; lost_frames: number };
export type Transcript = { text: string; language: string; logprob: number | null; elapsed_ms: number };
/** How a reply went out through the board's speaker. */
export type Spoken = { text: string; first_audio_ms: number; played_ms: number; reason: string; underruns: number; lost: number };
/** Who understood the words: the language model, or the fixed phrases. */
export type ReplyBy = "llm" | "rules";
export type Reply = { text: string; actions: string[]; understood: boolean; by: ReplyBy };
/** One line of the Voice tab's log; `transcript` null when Whisper heard no words. */
export type TranscriptEntry = { id: number; at: number; transcript: Transcript | null; reply: Reply | null; spoken: Spoken | null };
export type SttLanguage = "auto" | "it" | "en";
export type VoiceSettings = {
  enabled: boolean;
  model: string;
  language: SttLanguage;
  keep_recordings: boolean;
  /** Answer out loud through the board's speaker. */
  speak: boolean;
  /** After a spoken answer, listen a few seconds more without the wake word. */
  follow_up: boolean;
  /** Pause the music playing on this computer while the board listens. */
  pause_music: boolean;
  /** Keep the downloaded models loaded, those standing in for cloud ones too. */
  keep_warm: boolean;
  /** Voice by language: `it`, `en`. */
  voices: Record<string, string>;
  /** Understand with a local language model (else fixed phrases). */
  llm: boolean;
  llm_model: string;
  /** How the language model talks: a preset id, or `custom` with `personality_custom`. */
  personality: Personality;
  personality_custom: string;
};
export type Personality = "cute" | "playful" | "calm" | "sassy" | "butler" | "minimal" | "custom";
export type ModelInfo = {
  id: string;
  kind: "whisper" | "voice" | "llm";
  language: string | null;
  /** What speaks a voice. */
  engine: TtsEngine | null;
  bytes: number;
  note: string;
  license: string;
  /** Downloaded; for a cloud model, its provider has an API key. */
  installed: boolean;
  /** The online service that runs it (`groq`), for a cloud model: its id is `provider:model`. */
  provider: string | null;
};
export type ProviderInfo = {
  id: string;
  name: string;
  note: string;
  keys_url: string;
  key_env: string;
  /** Where its API key comes from: its environment variable, or saved by the app. */
  key: "env" | "saved" | null;
};
/** A voice the ElevenLabs account can speak with. */
export type AccountVoice = { voice_id: string; name: string; category: string; description: string | null };
export type SttStatus = "off" | "starting" | "ready" | "error";
export type TtsEngine = "piper";
export type VoiceInfo = {
  settings: VoiceSettings;
  server: string | null;
  models: ModelInfo[];
  stt: SttStatus;
  stt_error: string | null;
  /** Piper's Python, once installed. */
  piper: string | null;
  /** While Piper installs: its latest output line. */
  piper_install: string | null;
  tts: SttStatus;
  tts_error: string | null;
  /** Why the downloaded voice answers instead of the cloud one, while it does. */
  tts_fallback: string | null;
  /** llama-server, if found. */
  llm_server: string | null;
  llm: SttStatus;
  llm_error: string | null;
  download: [string, number] | null;
  providers: ProviderInfo[];
  hardware: Hardware;
  recommendation: Recommendation;
};
export type Hardware = {
  cpu: string;
  cores: number;
  memory_mb: number;
  gpu: { kind: "apple" } | { kind: "nvidia"; name: string; memory_mb: number } | { kind: "none" };
};
/** The models for this computer: `llm` null means better without one. */
export type Recommendation = { whisper: string; llm: string | null; speed: "fast" | "slow" | "limited"; why: string };
const TRANSCRIPTS = 50;

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
  voice: VoiceState | null;
  transcripts: TranscriptEntry[];
  port_setting: string | null;
  faces: Faces;
  rotation: Rotations;
  follow_pointer: boolean;
};

export type Link = "searching" | "connected" | "offline";
/** The voice settings the board keeps; null where it doesn't say. */
export type BoardVoice = {
  volume: number | null;
  eyes: boolean | null;
  idle_eyes: boolean | null;
  wake_sound: boolean | null;
  pet_sounds: boolean | null;
};
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
  /** The board's voice overlay, mirrored. */
  voice = $state<VoiceState>("idle");
  /** Whether the board shows eyes for its voice (else the ring); read once it's connected. */
  eyes = $state(true);
  /** Transcribed utterances, oldest first. */
  transcripts = $state<TranscriptEntry[]>([]);
  /** What each screen's image face shows, as data URLs; null without a picture. */
  images = $state<Record<Side, string | null>>({ left: null, right: null });
  /** An image on its way to the board: which screen and how far (0–1). */
  sending = $state<{ side: Side; progress: number } | null>(null);
  /** The timers, as the Timers tab last read them; null until it does. */
  timers = $state<TimersInfo | null>(null);
  /** The cover the music face shows, by the id the board knows it by. */
  cover = $state<{ id: number; url: string } | null>(null);
  #coverWanted = 0;
  /** The mouse pointer, for the eyes face; null until it's known. */
  gaze = $state<Gaze | null>(null);
  /** The eyes face follows the pointer (else its eyes look about on their own). */
  followPointer = $state(true);

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
    this.#watchPointer();
    void this.firmwareInfo();
  }

  async #connect() {
    await listen<BridgeEvent>("bridge", (e) => this.#apply(e.payload));
    await listen<FlashEvent>("flash", (e) => this.#applyFlash(e.payload));
    await listen<{ side: Side; progress: number }>("image", (e) => (this.sending = e.payload));
    for (const side of ["left", "right"] as Side[]) {
      invoke<string | null>("image_preview", { side }).then((url) => (this.images[side] = url)).catch(() => {});
    }
    const s = await invoke<Status>("status");
    const now = Date.now();
    this.link = s.link;
    this.port = s.port;
    this.portSetting = s.port_setting;
    this.faces = s.faces;
    this.rotation = s.rotation;
    this.followPointer = s.follow_pointer ?? true;
    this.message = s.message ?? "";
    this.last = s.last;
    this.shown = s.sent;
    if (s.sent_age_ms != null) this.sentAt = now - s.sent_age_ms;
    if (s.connected_age_ms != null) this.connectedAt = now - s.connected_age_ms;
    this.boardFirmware = s.firmware;
    this.logs = s.logs;
    this.voice = s.voice ?? "idle";
    this.transcripts = s.transcripts;
    if (s.link === "connected") this.boardVoice().catch(() => {});
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
        this.voice = "idle";
        if (!this.preview) setTimeout(() => this.boardVoice().catch(() => {}), BOOT_MS + 500);
        break;
      case "snapshot":
        this.last = e.snapshot;
        if (e.sent) {
          this.shown = e.snapshot;
          this.sentAt = now;
        }
        this.#fetchCover(e.snapshot.music?.art);
        this.#record(now, e.snapshot);
        break;
      case "board_log":
        this.logs.push(e.line);
        if (this.logs.length > LOG_LINES) this.logs.splice(0, this.logs.length - LOG_LINES);
        break;
      case "firmware":
        this.boardFirmware = e.firmware;
        break;
      case "settings":
        // An MCP client changed the board.
        this.faces = e.faces;
        this.rotation = e.rotation;
        break;
      case "wake":
        break;
      case "voice_state":
        this.voice = e.state;
        break;
      case "transcript":
        this.transcripts.push({ id: e.id, at: now, transcript: e.transcript, reply: null, spoken: null });
        if (this.transcripts.length > TRANSCRIPTS) this.transcripts.splice(0, this.transcripts.length - TRANSCRIPTS);
        break;
      case "reply": {
        const entry = this.latestTranscript(e.id);
        if (entry) entry.reply = { text: e.text, actions: e.actions, understood: e.understood, by: e.by };
        break;
      }
      case "spoken": {
        const entry = this.latestTranscript(e.id);
        if (entry) entry.spoken = e.spoken;
        break;
      }
      case "timer_fired":
        if (this.timers) void this.timersInfo();
        break;
      case "listening":
      case "utterance":
      case "voice_error":
        break;
      case "disconnected":
        this.link = "offline";
        this.voice = "idle";
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

  /** The cover named by a snapshot, once: the bridge keeps it ready. */
  #fetchCover(id: number | undefined) {
    if (!id || this.cover?.id === id || this.#coverWanted === id) return;
    this.#coverWanted = id;
    const got = this.preview ? Promise.resolve(previewCover()) : invoke<string | null>("music_cover", { id });
    got.then((url) => url && (this.cover = { id, url })).catch(() => {}).finally(() => (this.#coverWanted = 0));
  }

  /** Play, pause or skip the music on this computer. */
  async musicControl(action: MusicAction) {
    if (this.preview) return previewMusic.control(action);
    await invoke("music_control", { action });
  }

  async setFollowPointer(on: boolean) {
    this.followPointer = on;
    if (!this.preview) await invoke("set_follow_pointer", { on });
  }

  /** While a screen shows the eyes face, where the pointer is: the mirror's eyes look there too. */
  #watchPointer() {
    const set = (x: number, y: number) => {
      const g = this.gaze;
      if (g && Math.abs(g.x - x) < 0.004 && Math.abs(g.y - y) < 0.004) return;
      this.gaze = { x, y, movedAt: Date.now() };
    };
    if (this.preview) {
      // The page stands in for the screens.
      window.addEventListener("mousemove", (e) => set((e.clientX / innerWidth) * 2 - 1, (e.clientY / innerHeight) * 2 - 1));
      return;
    }
    let busy = false;
    setInterval(() => {
      const wanted = this.faces.cpu === "eyes" || this.faces.gpu === "eyes";
      if (!wanted || !this.followPointer || busy || document.hidden) return;
      busy = true;
      invoke<[number, number] | null>("pointer_gaze")
        .then((g) => g && set(g[0], g[1]))
        .catch(() => {})
        .finally(() => (busy = false));
    }, 50);
  }

  async setRotation(rotation: Rotations) {
    this.rotation = rotation;
    if (!this.preview) await invoke("set_rotation", { rotation });
  }

  /** Put `file` on `side`'s image face; the board gets it already scaled and cropped. */
  async sendImage(side: Side, file: File): Promise<ImageSent> {
    const url = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result));
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(file);
    });
    if (this.preview) {
      for (let p = 0; p <= 1; p += 0.1) {
        this.sending = { side, progress: p };
        await new Promise((r) => setTimeout(r, 80));
      }
      this.sending = null;
      this.images[side] = url;
      return { frames: 1, source_frames: 1, bytes: file.size, duration_ms: 0 };
    }
    this.sending = { side, progress: 0 };
    try {
      const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
      const sent = await invoke<ImageSent>("send_image", { side, bytes });
      this.images[side] = url;
      return sent;
    } finally {
      this.sending = null;
    }
  }

  async clearImage(side: Side) {
    if (!this.preview) await invoke("clear_image", { side });
    this.images[side] = null;
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

  async claudeAlerts(): Promise<ClaudeAlertsInfo> {
    if (this.preview) return previewAlerts;
    return invoke<ClaudeAlertsInfo>("claude_alerts_info");
  }

  async setClaudeAlerts(settings: ClaudeAlertSettings): Promise<ClaudeAlertsInfo> {
    if (this.preview) return (previewAlerts = { ...previewAlerts, settings });
    return invoke<ClaudeAlertsInfo>("set_claude_alerts", { settings });
  }

  /** Add the app to Claude Code's hooks, or take it out. */
  async claudeHooks(connect: boolean): Promise<ClaudeAlertsInfo> {
    if (this.preview) return (previewAlerts = { ...previewAlerts, hooks: { ...previewAlerts.hooks, connected: connect } });
    return invoke<ClaudeAlertsInfo>("claude_hooks", { connect });
  }

  async testClaudeAlert() {
    if (this.preview) {
      previewAlerts = { ...previewAlerts, last: { text: "Claude needs you in DualEye.", error: null, age_s: 0 } };
      return;
    }
    await invoke("test_claude_alert");
  }

  async timersInfo(): Promise<TimersInfo> {
    const info = this.preview ? previewTimers.info() : await invoke<TimersInfo>("timers_info");
    this.timers = info;
    return info;
  }

  /** One of the host's timer tools: `set_timer`, `set_reminder`, `pomodoro`, `control_timer`. */
  async timerTool(name: string, args: Record<string, unknown>): Promise<TimersInfo> {
    const info = this.preview ? previewTimers.call(name, args) : await invoke<TimersInfo>("timer_tool", { name, arguments: args });
    this.timers = info;
    return info;
  }

  async setTimerScreen(show_on: ShowOn): Promise<TimersInfo> {
    if (this.preview) previewTimers.showOn = show_on;
    const info = this.preview ? previewTimers.info() : await invoke<TimersInfo>("set_timer_screen", { showOn: show_on });
    this.timers = info;
    return info;
  }

  async dismissTimers(): Promise<TimersInfo> {
    if (this.preview) previewTimers.call("control_timer", { action: "cancel" });
    const info = this.preview ? previewTimers.info() : await invoke<TimersInfo>("dismiss_timers");
    this.timers = info;
    return info;
  }

  /** The newest log line for utterance `id` (ids wrap at 256). */
  private latestTranscript(id: number): TranscriptEntry | undefined {
    for (let i = this.transcripts.length - 1; i >= 0; i--) if (this.transcripts[i].id === id) return this.transcripts[i];
  }

  async mcpInfo(): Promise<McpInfo> {
    if (this.preview) return previewMcp;
    return invoke<McpInfo>("mcp_info");
  }

  async voiceInfo(): Promise<VoiceInfo> {
    if (this.preview) return { ...previewVoice, settings: { ...previewVoice.settings } };
    return invoke<VoiceInfo>("voice_info");
  }

  async setVoice(settings: VoiceSettings): Promise<VoiceInfo> {
    if (this.preview) {
      previewVoice.settings = settings;
      previewVoice.stt = settings.enabled ? (previewVoice.models.find((m) => m.id === settings.model)?.installed ? "ready" : "error") : "off";
      previewVoice.stt_error = previewVoice.stt === "error" ? `the ${settings.model} model isn't downloaded yet` : null;
      const voices = previewVoice.models.filter((m) => Object.values(settings.voices).includes(m.id) && m.installed);
      const missing = voices.find((m) => !previewVoice[m.engine ?? "piper"]);
      previewVoice.tts = !settings.enabled || !settings.speak ? "off" : voices.length && !missing ? "ready" : "error";
      previewVoice.tts_error =
        previewVoice.tts !== "error" ? null : missing ? "Piper isn't installed yet" : "no voice downloaded yet";
      const llmReady = previewVoice.models.find((m) => m.id === settings.llm_model)?.installed;
      previewVoice.llm = !settings.enabled || !settings.llm ? "off" : llmReady ? "ready" : "error";
      previewVoice.llm_error = previewVoice.llm === "error" ? `the ${settings.llm_model} model isn't downloaded yet` : null;
      return this.voiceInfo();
    }
    return invoke<VoiceInfo>("set_voice", { settings });
  }

  /** The voices the ElevenLabs account can speak with: its own, newest first, then ElevenLabs'. */
  async elevenlabsVoices(): Promise<AccountVoice[]> {
    if (this.preview) return [...previewAccountVoices];
    return invoke<AccountVoice[]>("elevenlabs_voices");
  }

  /** Save a provider's API key (null forgets it); fails if the provider refuses it. */
  async setApiKey(provider: string, key: string | null): Promise<VoiceInfo> {
    if (this.preview) {
      const p = previewVoice.providers.find((p) => p.id === provider);
      if (p && p.key !== "env") p.key = key ? "saved" : null;
      for (const m of previewVoice.models) if (m.provider === provider) m.installed = !!p?.key;
      return this.setVoice(previewVoice.settings);
    }
    return invoke<VoiceInfo>("set_api_key", { provider, key });
  }

  async downloadModel(id: string) {
    if (this.preview) {
      for (const m of previewVoice.models) if (m.id === id) m.installed = true;
      return;
    }
    await invoke("download_model", { id });
  }

  async cancelDownload() {
    if (!this.preview) await invoke("cancel_download");
  }

  async deleteModel(id: string): Promise<VoiceInfo> {
    if (this.preview) {
      for (const m of previewVoice.models) if (m.id === id) m.installed = false;
      return this.voiceInfo();
    }
    return invoke<VoiceInfo>("delete_model", { id });
  }

  /** Install a text-to-speech engine into its own virtualenv. */
  async installEngine(engine: TtsEngine) {
    if (this.preview) {
      previewVoice[`${engine}_install`] = "Installing piper-tts";
      await new Promise((r) => setTimeout(r, 2500));
      previewVoice[`${engine}_install`] = null;
      previewVoice[engine] = `~/Library/Application Support/dualeye/${engine}/venv/bin/python`;
      await this.setVoice(previewVoice.settings);
      return;
    }
    await invoke("install_engine", { engine });
  }

  /** The board's speaker volume and eyes; null where the board doesn't say. */
  async boardVoice(): Promise<BoardVoice> {
    if (this.preview) return { volume: previewVolume, eyes: previewEyes, idle_eyes: previewIdleEyes, wake_sound: previewWakeSound, pet_sounds: previewPetSounds };
    const b = await invoke<BoardVoice>("board_voice");
    this.eyes = b.eyes ?? false;
    return b;
  }

  async setBoardVolume(percent: number) {
    if (this.preview) {
      previewVolume = percent;
      return;
    }
    await invoke("set_board_volume", { percent });
  }

  async setBoardEyes(on: boolean) {
    if (this.preview) {
      previewEyes = on;
      this.eyes = on;
      return;
    }
    await invoke("set_board_eyes", { on });
    this.eyes = on;
  }

  /** The eyes' scenes on the board while nobody is talking (firmware 1.0.2). */
  async setBoardIdleEyes(on: boolean) {
    if (this.preview) {
      previewIdleEyes = on;
      return;
    }
    await invoke("set_board_idle_eyes", { on });
  }

  /** A chime on the board when it hears the wake word (firmware 1.3.3). */
  async setBoardWakeSound(on: boolean) {
    if (this.preview) {
      previewWakeSound = on;
      return;
    }
    await invoke("set_board_wake_sound", { on });
  }

  /** The board's little sounds: scenes, hello, snoring... (firmware 1.4.0). */
  async setBoardPetSounds(on: boolean) {
    if (this.preview) {
      previewPetSounds = on;
      return;
    }
    await invoke("set_board_pet_sounds", { on });
  }

  /** One of the eyes' scenes on the board now. */
  async playScene(name: string) {
    if (this.preview) return;
    await invoke("play_board_scene", { name });
  }

  /** One of the board's sounds on its speaker. */
  async playSound(name: string) {
    if (this.preview) return;
    await invoke("play_board_sound", { name });
  }

  async testVoice(language: string) {
    if (this.preview) return new Promise((r) => setTimeout(r, 1500));
    await invoke("test_voice", { language });
  }

  async readings(): Promise<Reading[]> {
    if (this.preview) return previewReadings(this.last);
    return invoke<Reading[]>("readings");
  }

  async powerHelper(): Promise<PowerHelper> {
    if (this.preview) return { state: "off", error: null };
    return invoke<PowerHelper>("power_helper_status");
  }

  async setPowerHelper(on: boolean): Promise<PowerHelper> {
    if (this.preview) return { state: on ? "on" : "off", error: null };
    return invoke<PowerHelper>("set_power_helper", { on });
  }

  async openPowerHelperSettings() {
    if (!this.preview) await invoke("open_power_helper_settings");
  }
}

export const monitor = new Monitor();

export function fanRpm(s: Snapshot | null, id: string): number | undefined {
  return s?.fans?.find((f) => f.id === id)?.rpm;
}

// ── Preview feed ────────────────────────────────────────────────────────────

const previewMcp: McpInfo = {
  command: "/Applications/DualEye.app/Contents/MacOS/dualeye-app",
  args: ["--mcp"],
  hub: { clients: 1, calls: 3, last_tool: "set_face", last_call_age_ms: 42000 },
  hub_error: null,
};

let previewClaudeLink: ClaudeLink = { connected: false, chained: null, last_update_s: null, settings_path: "~/.claude/settings.json" };

let previewAlerts: ClaudeAlertsInfo = {
  settings: { needs_you: true, done: true, done_after_s: 30, usage: true, speak: true, language: "en" },
  hooks: { connected: false, last_event_s: null, settings_path: "~/.claude/settings.json" },
  can_speak: true,
  last: null,
};

let previewVolume = 60;
let previewEyes = true;
let previewIdleEyes = true;
let previewWakeSound = true;
let previewPetSounds = true;

function previewCloud(kind: ModelInfo["kind"], language: string | null, models: [string, string][]): ModelInfo[] {
  return models.map(([id, note]) => ({ id: `groq:${id}`, kind, language, engine: null, bytes: 0, note, license: "", installed: false, provider: "groq" }));
}

const previewAccountVoices: AccountVoice[] = [
  { voice_id: "pv-birba", name: "Birba", category: "generated", description: "Mischievous cartoon cat sidekick" },
  { voice_id: "pv-rachel", name: "Rachel", category: "premade", description: null },
];

const previewVoice: VoiceInfo = {
  settings: {
    enabled: false,
    model: "small",
    language: "auto",
    keep_recordings: false,
    speak: true,
    follow_up: true,
    pause_music: true,
    keep_warm: true,
    voices: { it: "it_IT-paola-medium", en: "en_GB-alba-medium" },
    llm: true,
    llm_model: "qwen3.5-4b",
    personality: "cute",
    personality_custom: "",
  },
  server: "/opt/homebrew/bin/whisper-server",
  models: [
    { id: "base", kind: "whisper", language: null, engine: null, bytes: 147_951_465, note: "Fastest, for slow CPUs; often wrong in Italian", license: "MIT", installed: false, provider: null },
    { id: "small", kind: "whisper", language: null, engine: null, bytes: 487_601_967, note: "Good balance: about 0.7 s a command on an M1 Pro", license: "MIT", installed: true, provider: null },
    { id: "large-v3-turbo-q5_0", kind: "whisper", language: null, engine: null, bytes: 574_041_195, note: "Most accurate; wants a GPU (Apple silicon, NVIDIA)", license: "MIT", installed: false, provider: null },
    { id: "it_IT-paola-medium", kind: "voice", language: "it", engine: "piper", bytes: 63_518_137, note: "Italian, woman's voice, natural", license: "Dataset CC0 1.0 (paolapersico1/Voice-Dataset-Italian); fine-tuned from lessac", installed: true, provider: null },
    { id: "it_IT-riccardo-x_low", kind: "voice", language: "it", engine: "piper", bytes: 28_134_952, note: "Italian, man's voice, smaller and flatter", license: "Dataset M-AILABS (BSD-style); trained from scratch", installed: false, provider: null },
    { id: "en_GB-alba-medium", kind: "voice", language: "en", engine: "piper", bytes: 63_206_182, note: "British English, woman's voice", license: "Dataset CC BY 4.0 (Edinburgh DataShare 10283/3270); fine-tuned from lessac", installed: false, provider: null },
    { id: "en_US-ljspeech-medium", kind: "voice", language: "en", engine: "piper", bytes: 63_536_351, note: "American English, woman's voice", license: "Dataset public domain (LJ Speech)", installed: false, provider: null },
    { id: "qwen3.5-2b", kind: "llm", language: null, engine: null, bytes: 1_280_835_840, note: "Lighter and faster (0.65 s); more mistakes, often answers in English", license: "Apache-2.0", installed: false, provider: null },
    { id: "qwen3.5-4b", kind: "llm", language: null, engine: null, bytes: 2_740_937_888, note: "Most accurate: 52 of 53 test commands right, about 1.4 s each on an M1 Pro; sometimes answers in the wrong language", license: "Apache-2.0", installed: true, provider: null },
    ...previewCloud("whisper", null, [
      ["whisper-large-v3-turbo", "Whisper large v3 turbo on Groq: fast and accurate, 2,000 a day free"],
      ["whisper-large-v3", "Whisper large v3 on Groq: a little more accurate, slower"],
    ]),
    ...previewCloud("llm", null, [
      ["openai/gpt-oss-20b", "GPT-OSS 20B on Groq: fast, good at tools; about 5 commands a minute free"],
      ["llama-3.3-70b-versatile", "Llama 3.3 70B on Groq: good Italian, no thinking"],
    ]),
    ...previewCloud("voice", "en", [
      ["hannah", "Orpheus on Groq, English, woman's voice; 100 sentences a day free"],
      ["troy", "Orpheus on Groq, English, man's voice"],
    ]),
  ],
  stt: "off",
  stt_error: null,
  piper: null,
  piper_install: null,
  tts: "off",
  tts_error: null,
  tts_fallback: null,
  llm_server: "/opt/homebrew/bin/llama-server",
  llm: "off",
  llm_error: null,
  download: null,
  providers: [
    { id: "groq", name: "Groq", note: "Free with daily limits; what you say is sent to Groq", keys_url: "https://console.groq.com/keys", key_env: "GROQ_API_KEY", key: null },
    {
      id: "elevenlabs",
      name: "ElevenLabs",
      note: "The voices of your account; free plan: 10,000 credits a month. The answers are sent to ElevenLabs",
      keys_url: "https://elevenlabs.io/app/developers/api-keys",
      key_env: "ELEVENLABS_API_KEY",
      key: null,
    },
  ],
  hardware: { cpu: "Apple M1 Pro", cores: 8, memory_mb: 16_384, gpu: { kind: "apple" } },
  recommendation: { whisper: "small", llm: "qwen3.5-4b", speed: "fast", why: "Apple silicon with enough memory runs the recommended models on its GPU." },
};

function startPreviewFeed(emit: (e: BridgeEvent) => void, faces: () => Faces, rotation: () => Rotations) {
  const boot = [
    "ESP-ROM:esp32s3-20210327",
    "I (24) boot: ESP-IDF v6.1 2nd stage bootloader",
    "I (810) board_display: Dual GC9A01 ready (L:+90 CCW, R:+90 CW)",
    "I (890) ui_watch: Watch UI created",
    "I (900) link: protocol v2 up",
    "I (900) dualeye: Watch UI ready, waiting for the host",
  ];
  setTimeout(() => emit({ kind: "connected", port: "/dev/ttyACM0" }), 600);
  boot.forEach((line, i) => setTimeout(() => emit({ kind: "board_log", line }), 900 + i * 90));
  // Firmware from before versioning, so the update offer shows up.
  setTimeout(() => emit({ kind: "firmware", firmware: { state: "legacy" } }), 8000);
  // Now and then someone says the wake word: listening, then thinking, then back to idle.
  const voice = (state: VoiceState, at: number) => setTimeout(() => emit({ kind: "voice_state", state }), at);
  const phrases: [string, string, string, string[]][] = [
    ["Metti la faccia rings a sinistra.", "it", "Fatto: faccia rings sullo schermo sinistro.", ["set_face: left: rings"]],
    ["What is the GPU temperature?", "en", "The GPU is at 41 degrees.", []],
  ];
  let said = 0;
  setInterval(() => {
    voice("listening", 0);
    voice("thinking", 2500);
    const [text, language, reply, actions] = phrases[said++ % phrases.length];
    const id = said;
    setTimeout(() => emit({ kind: "transcript", id, transcript: { text, language, logprob: -0.2, elapsed_ms: 640 } }), 3100);
    setTimeout(() => emit({ kind: "reply", id, text: reply, language, actions, understood: true, by: "llm", elapsed_ms: 12 }), 3150);
    voice("speaking", 3300);
    const spoken = { text: reply, first_audio_ms: 140, played_ms: 2300, reason: "done", underruns: 0, lost: 0 };
    setTimeout(() => emit({ kind: "spoken", id, spoken }), 5700);
    voice("idle", 5700);
  }, 30000);

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
        // Downloads in bursts, a trickle up.
        net: { rx_bps: Math.round(40_000 + 12_000_000 * burst + Math.random() * 30_000), tx_bps: Math.round(8_000 + 400_000 * burst * Math.random()) },
        disk: { used_gb: 612.4, total_gb: 994.7, read_bps: Math.round(2_000_000 * burst), write_bps: Math.round(300_000 + 4_000_000 * burst * Math.random()) },
        bat: { pct: Math.max(5, Math.round(84 - t / 30)), charging: false, plugged: false, mins: Math.max(10, Math.round(312 - t / 6)) },
        face: { ...faces() },
        ...(previewTimers.board() ? { timer: previewTimers.board() } : {}),
        music: previewMusic.board(),
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

/** A song for the browser preview, and a cover drawn for it. */
const previewMusic = {
  playing: true,
  start: Date.now(),
  pos: 61,
  track: 0,
  tracks: [
    { title: "Zitti e buoni", artist: "Maneskin", dur_s: 195, art: 1001, colors: ["#e8303a", "#2a0b10"] },
    { title: "Bohemian Rhapsody", artist: "Queen", dur_s: 354, art: 1002, colors: ["#d9a441", "#1b1406"] },
  ],
  now() {
    return this.pos + (this.playing ? (Date.now() - this.start) / 1000 : 0);
  },
  board(): BoardMusic {
    const t = this.tracks[this.track];
    let pos = this.now();
    if (pos >= t.dur_s) {
      this.control("next");
      pos = 0;
    }
    return { state: this.playing ? "play" : "pause", title: t.title, artist: t.artist, pos_s: r1(pos), dur_s: t.dur_s, art: t.art };
  },
  control(action: MusicAction) {
    this.pos = this.now();
    this.start = Date.now();
    if (action === "next" || action === "previous") {
      this.track = (this.track + 1) % this.tracks.length;
      this.pos = 0;
    } else this.playing = action === "play" ? true : action === "pause" ? false : !this.playing;
  },
};

function previewCover(): string {
  const t = previewMusic.tracks[previewMusic.track];
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 240 240"><defs><radialGradient id="g" cx="30%" cy="25%" r="90%">` +
    `<stop offset="0" stop-color="${t.colors[0]}"/><stop offset="1" stop-color="${t.colors[1]}"/></radialGradient></defs>` +
    `<rect width="240" height="240" fill="url(#g)"/><circle cx="170" cy="70" r="46" fill="white" opacity=".18"/>` +
    `<text x="20" y="120" font-family="Georgia" font-size="34" fill="white" opacity=".9">${t.artist}</text></svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

type PreviewTimer = { id: number; kind: TimerKind; label?: string; total_s: number; end: number; left: number; paused: boolean; rang?: number };
/** Timers for the browser preview: a small copy of `dualeye_core::timers`, without the pomodoro's turns. */
const previewTimers = {
  showOn: "right" as ShowOn,
  list: [] as PreviewTimer[],
  next: 1,
  leftOf(t: PreviewTimer): number {
    return t.paused ? t.left : Math.max(0, (t.end - Date.now()) / 1000);
  },
  tick() {
    const now = Date.now();
    for (const t of this.list) if (!t.paused && !t.rang && t.end <= now) t.rang = now;
    this.list = this.list.filter((t) => !t.rang || now - t.rang < 60_000);
  },
  info(): TimersInfo {
    this.tick();
    const timers: TimerInfo[] = this.list
      .map((t) => ({
        id: t.id,
        kind: t.kind,
        label: t.label,
        total_s: t.total_s,
        left_s: Math.ceil(this.leftOf(t)),
        state: (t.rang ? "ring" : t.paused ? "pause" : "run") as TimerInfo["state"],
        ends_at: t.paused || t.rang ? undefined : new Date(t.end).toTimeString().slice(0, 5),
      }))
      .sort((a, b) => Number(b.state === "ring") - Number(a.state === "ring") || Number(a.state === "pause") - Number(b.state === "pause") || a.left_s - b.left_s);
    const work = this.list.find((t) => t.kind === "work");
    return { timers, pomodoro: work ? { work_s: work.total_s, break_s: 300, rounds: 4, round: 1 } : null, show_on: this.showOn, can_speak: false };
  },
  board(): BoardTimer | undefined {
    const [first, ...rest] = this.info().timers;
    if (!first) return undefined;
    const screen = this.showOn === "none" ? undefined : this.showOn;
    const label = first.label?.toUpperCase().slice(0, 27);
    const pomodoro = first.kind === "work" ? { round: 1, rounds: 4 } : {};
    const t = this.list.find((x) => x.id === first.id)!;
    return { kind: first.kind, state: first.state, left_s: Math.round(this.leftOf(t) * 10) / 10, total_s: first.total_s, label, more: rest.filter((r) => r.state !== "ring").length, screen, ...pomodoro };
  },
  add(kind: TimerKind, secs: number, label?: string) {
    this.list.push({ id: this.next++, kind, label: label || undefined, total_s: secs, end: Date.now() + secs * 1000, left: secs, paused: false });
  },
  call(name: string, a: Record<string, unknown>): TimersInfo {
    const n = (k: string) => Number(a[k] ?? 0) || 0;
    if (name === "set_timer") this.add("timer", Math.round(n("hours") * 3600 + n("minutes") * 60 + n("seconds")), a.label as string | undefined);
    if (name === "set_reminder") this.add("reminder", Math.round(n("in_minutes") * 60) || 600, a.text as string);
    if (name === "pomodoro") {
      this.list = this.list.filter((t) => t.kind !== "work" && t.kind !== "break");
      if (a.action !== "stop") this.add("work", (n("work_minutes") || 25) * 60);
    }
    if (name === "control_timer") {
      const which = a.which as string | undefined;
      const ringing = this.list.filter((t) => t.rang);
      const picked = which === "all" ? this.list : which ? this.list.filter((t) => String(t.id) === which || t.label === which) : ringing.length ? ringing : this.list.slice(0, 1);
      for (const t of picked) {
        if (a.action === "pause" && !t.paused) {
          t.left = this.leftOf(t);
          t.paused = true;
        } else if (a.action === "resume" && t.paused) {
          t.end = Date.now() + t.left * 1000;
          t.paused = false;
        }
      }
      if (a.action === "cancel") this.list = this.list.filter((t) => !picked.includes(t));
    }
    return this.info();
  },
};

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
  setTimeout(() => bridge({ kind: "firmware", firmware: { state: "version", version: bundledVersion.trim(), idf: "v6.1", protocol: 2 } }), 1200);
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
