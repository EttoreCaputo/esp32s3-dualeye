// Constants and formatting copied from main/ui_watch.c, so the mirror shows the
// same pixels the board does. Keep in sync when the firmware UI changes.

import type { Battery, BoardMusic, BoardTimer, ClaudeMetrics, ClaudeState, Disk, Gaze, Metrics, Net } from "./monitor.svelte";

export const LCD = 240;
export const USAGE_ARC_SIZE = 216;
export const RING_GAP = 32;
export const ARC_WIDTH = 13;
export const TEMP_WARM_C = 80;
export const TEMP_HOT_C = 90;
export const MEM_HIGH_PCT = 90;
export const TEMP_MAX_C = 100;
export const CLAUDE_WARM_PCT = 80;
export const CLAUDE_HOT_PCT = 95;
export const CLAUDE_BLOCK_MIN = 300;

export const COLOR = {
  text: "#FFFFFF",
  textDim: "#9A9A9C",
  cyan: "#3AE7ED",
  tempTrack: "#0B2C30",
  mem: "#5E8BFF",
  memTrack: "#141D3A",
  warm: "#F8A639",
  hot: "#F05354",
  stale: "#FFD60A",
  claude: "#D97757",
  claudeDim: "#6E3B2B",
  claudeTrack: "#35190F",
  week: "#E9C4A6",
  weekTrack: "#2B2019",
  green: "#40E080",
  greenTrack: "#0F2A18",
  timerTrack: "#33230D",
  tomato: "#FF6347",
  tomatoTrack: "#3A1512",
  divider: "#3A3A3C",
  error: "#FF453A",
  music: "#FF4F7B",
  musicTrack: "#3A1420",
  artist: "#C8C8CC",
  disc: "#141416",
  groove: "#26262A",
} as const;
/** The net rings are logarithmic: log10 of the speed, 100 B/s (empty) to 1 GB/s (full). */
export const NET_LOG_MIN = 2;
export const NET_LOG_MAX = 9;
export const TIMER_BLINK_MS = 500;
export const DISK_FULL_PCT = 90;
export const BATTERY_LOW_PCT = 20;
export const BATTERY_EMPTY_PCT = 10;
/** The music face's position ring round the very edge, and the record shown without a cover. */
export const MUSIC_RING_SIZE = 234;
export const MUSIC_RING_WIDTH = 6;
export const MUSIC_DISC_SIZE = 124;
/** The eyes face (main/ui_eyes.c): its colours, how far the pointer moves it, when it dozes. */
export const AMBIENT_COLOR = "#E6F2FF";
export const ASLEEP_COLOR = "#34507A";
export const LOOK_X = 36;
export const LOOK_Y = 26;
export const LOOK_Y_BIAS = -8;
export const POINTER_FRESH_MS = 6000;
export const DOZE_AFTER_MS = 60000;
export const DOZE_MS = 20000;

export const DEVICES = {
  cpu: { title: "CPU", memTitle: "RAM", accent: "#C4F06A", track: "#163012" },
  gpu: { title: "GPU", memTitle: "VRAM", accent: "#C86CF0", track: "#2A1238" },
} as const;
export type DeviceId = keyof typeof DEVICES;

/** `metrics_face_t`; the names are what goes on the wire. */
export type Face = "classic" | "rings" | "plus" | "bar" | "claude" | "clawd" | "net" | "disk" | "battery" | "image" | "timer" | "music" | "eyes";
/** `metrics_source_t`: whose metrics classic, rings, plus and bar show. */
export type Source = DeviceId;
/** Faces by screen (`cpu` is the left one, `gpu` the right one, as on the wire) and each screen's source. */
export type Faces = { cpu: Face; gpu: Face; src: Record<DeviceId, Source> };
export const DEFAULT_SOURCES: Record<DeviceId, Source> = { cpu: "cpu", gpu: "gpu" };
export const DEFAULT_FACES: Faces = { cpu: "classic", gpu: "classic", src: { ...DEFAULT_SOURCES } };
export const FACES: { id: Face; name: string; blurb: string }[] = [
  { id: "classic", name: "Classic", blurb: "Temperature, clock, power, fan" },
  { id: "rings", name: "Rings", blurb: "Load, temperature and memory rings" },
  { id: "plus", name: "Plus", blurb: "Classic with a RAM or VRAM bar" },
  { id: "bar", name: "Bar", blurb: "Classic with a slim memory bar, no numbers" },
  { id: "claude", name: "Claude", blurb: "Claude Code's 5-hour and weekly limits, with Clawd" },
  { id: "clawd", name: "Clawd", blurb: "Clawd shows whether Claude Code is working" },
  { id: "net", name: "Network", blurb: "Download and upload speed" },
  { id: "disk", name: "Disk", blurb: "System disk space, reads and writes" },
  { id: "battery", name: "Battery", blurb: "The laptop's charge and time left" },
  { id: "image", name: "Image", blurb: "A picture or GIF of your own" },
  { id: "timer", name: "Timer", blurb: "Timers, reminders and the pomodoro counting down" },
  { id: "music", name: "Music", blurb: "What's playing, with its cover; Alexa pauses and skips it" },
  { id: "eyes", name: "Eyes", blurb: "A pair of eyes that follow your mouse pointer" },
];
/** Faces that show the CPU's or the GPU's metrics, as the screen's source says. */
export const hasSource = (face: Face) => face === "classic" || face === "rings" || face === "plus" || face === "bar";
/** Extra clockwise turn of a screen, in degrees; `rot` on the wire. */
export type Rotation = 0 | 90 | 180 | 270;
export type Rotations = Record<DeviceId, Rotation>;
export const DEFAULT_ROTATIONS: Rotations = { cpu: 0, gpu: 0 };
export const ROTATIONS: Rotation[] = [0, 90, 180, 270];
export const isClaudeFace = (face: Face): face is "claude" | "clawd" => face === "claude" || face === "clawd";

/** `ui_classic_layout_t` for the classic-based faces; no bar when `barW` is 0. */
export const CLASSIC_LAYOUT: Record<"classic" | "plus" | "bar", { y: number; titleGap: number; barW: number; barH: number; memText: boolean }> = {
  classic: { y: 2, titleGap: 10, barW: 0, barH: 0, memText: false },
  plus: { y: -10, titleGap: 8, barW: 96, barH: 6, memText: true },
  bar: { y: -3, titleGap: 10, barW: 72, barH: 4, memText: false },
};

/** The text and colours one round screen shows, as the `update_*` functions in ui_watch.c compute them. */
export type Screen = {
  face: Face;
  /** Whose accent the metrics faces wear: the screen's source. */
  source: DeviceId;
  placeholder: boolean;
  title: string;
  value: string;
  clock: string;
  watts: string;
  usage: string;
  rpm: string;
  mem: string;
  memName: string;
  memValue: string;
  usagePct: number;
  tempPct: number;
  memPct: number;
  tempRing: string;
  labelColor: string;
  valueColor: string;
  usageColor: string;
  memColor: string;
  /** The memory bar and the plus face's RAM/VRAM name: orange when nearly full. */
  barColor: string;
  warn: boolean;
  /** Set on the claude and clawd faces. */
  claude?: ClaudeView;
  net?: NetView;
  disk?: DiskView;
  battery?: BatteryView;
  /** The image face: a data URL of the picture, or null without one. */
  image?: string | null;
  /** The timer face; `null` without a timer (the hint). */
  timer?: TimerView | null;
  /** The music face; `null` with nothing playing (the record and the hint). */
  music?: MusicView | null;
  /** The eyes face. */
  eyes?: EyesView;
};

/** `update_music()` and `music_tick()`. */
export type MusicView = {
  /** The cover as a data URL, once the one for this track is in; else the record. */
  cover: string | null;
  playing: boolean;
  title: string;
  titleColor: string;
  artist: string;
  time: string;
  /** The position ring, 0–100. */
  pct: number;
};
/** `ambient_pose()`: where the eye looks, in panel pixels, and how far it has dozed off (0–1). */
export type EyesView = { x: number; y: number; doze: number; color: string };

/** `update_net()`: download outside and in large, upload inside and below a divider. */
export type NetView = { rxPct: number; txPct: number; unit: string; tx: string };
/** `update_disk()` */
export type DiskView = { pct: number; color: string; space: string; read: string | null; write: string | null };
/** `update_timer()` and `timer_tick()`: the ring empties as the time runs out. */
export type TimerView = {
  pct: number;
  color: string;
  track: string;
  title: string;
  titleColor: string;
  value: string;
  /** Over an hour the time is in the bold 32 font. */
  small: boolean;
  valueColor: string;
  dim: boolean;
  label: string;
  info: string;
  infoColor: string;
  more: string;
};
/** `update_battery()` */
export type BatteryView = { pct: number; color: string; status: string; statusColor: string; time: string };

/** What the screens show besides one device's metrics. */
export type Extras = {
  net?: Net;
  disk?: Disk;
  bat?: Battery;
  image?: string | null;
  claude?: ClaudeMetrics;
  fan?: number;
  timer?: BoardTimer;
  /** How long ago the snapshot with `timer` came, in ms: the board counts down meanwhile. */
  timerAgeMs?: number;
  /** For the blink while it rings. */
  nowMs?: number;
  music?: BoardMusic;
  cover?: { id: number; url: string } | null;
  /** How long ago the snapshot with `music` came, in ms: the board counts the position on meanwhile. */
  musicAgeMs?: number;
  /** The mouse pointer, for the eyes face. */
  gaze?: Gaze | null;
};

/** What `update_claude()` and `update_clawd_face()` put on screen. */
export type ClaudeView = {
  sessionPct: number;
  sessionColor: string;
  showWeek: boolean;
  weekPct: number;
  weekColor: string;
  value: string;
  valueColor: string;
  reset: string;
  weekName: string;
  week: string;
  model: string;
  status: string;
  statusColor: string;
  tokens: string;
  mascot: ClaudeState;
  mascotColor: string;
};

const cInt = (v: number) => Math.trunc(v + 0.5); // (int) (v + 0.5f)
/** `clamp_pct()` */
const pct = (v: number, max: number) => Math.min(100, Math.max(0, cInt((v / max) * 100)));
const pctText = (p: number | undefined) => (p === undefined ? "--%" : `${p}%`);

/** One screen: `face`, with `source`'s metrics `m` for the faces that have a source. */
export function screenFor(
  source: DeviceId,
  face: Face,
  m: Metrics | undefined,
  stale: boolean,
  waiting: boolean,
  extras: Extras = {},
): Screen {
  const screen = sensorScreen(source, face, m, stale, waiting, extras.fan);
  if (isClaudeFace(face)) return { ...screen, claude: claudeView(extras.claude, stale, waiting) };
  if (face === "timer") return { ...screen, timer: timerView(extras, stale) };
  if (face === "net") return netScreen(screen, extras.net, stale, waiting);
  if (face === "disk") return diskScreen(screen, extras.disk, stale, waiting);
  if (face === "battery") return batteryScreen(screen, extras.bat, stale, waiting);
  if (face === "image") return { ...screen, image: extras.image ?? null };
  if (face === "music") return { ...screen, music: waiting ? null : musicView(extras, stale), title: waiting ? "WAITING" : "NOTHING PLAYING" };
  if (face === "eyes") return { ...screen, eyes: eyesView(extras) };
  return screen;
}

/** `format_clock()`: "3:07", "1:02:45". */
function clock(secs: number): string {
  const s = Math.trunc(secs);
  const two = (n: number) => String(n).padStart(2, "0");
  return s >= 3600 ? `${Math.trunc(s / 3600)}:${two(Math.trunc((s % 3600) / 60))}:${two(s % 60)}` : `${Math.trunc(s / 60)}:${two(s % 60)}`;
}

function musicView(x: Extras, stale: boolean): MusicView | null {
  const m = x.music;
  if (!m) return null;
  const playing = m.state === "play";
  const dur = m.dur_s ?? 0;
  let pos = (m.pos_s ?? 0) + (playing ? (x.musicAgeMs ?? 0) / 1000 : 0);
  if (dur > 0) pos = Math.min(pos, dur);
  const time = m.pos_s === undefined ? "" : dur > 0 ? `${clock(pos)} / ${clock(dur + 0.5)}` : clock(pos);
  return {
    cover: m.art && x.cover?.id === m.art ? x.cover.url : null,
    playing,
    title: m.title,
    titleColor: stale ? COLOR.textDim : COLOR.text,
    artist: m.artist ?? "",
    time,
    pct: m.pos_s !== undefined && dur > 0 ? (pos / dur) * 100 : 0,
  };
}

const mix = (a: string, b: string, t: number) =>
  "#" + [1, 3, 5].map((i) => Math.round(parseInt(a.slice(i, i + 2), 16) * (1 - t) + parseInt(b.slice(i, i + 2), 16) * t).toString(16).padStart(2, "0")).join("");

/** `ambient_step()` and `ambient_pose()` without the glances: at the pointer, or ahead. */
function eyesView(x: Extras): EyesView {
  const now = x.nowMs ?? Date.now();
  const g = x.gaze;
  const fresh = g && now - g.movedAt < POINTER_FRESH_MS;
  const still = g ? now - g.movedAt : 0;
  const doze = g && still > DOZE_AFTER_MS ? Math.min(1, (still - DOZE_AFTER_MS) / DOZE_MS) : 0;
  const lx = fresh ? LOOK_X * g.x : 0;
  const ly = fresh ? LOOK_Y_BIAS + LOOK_Y * g.y : LOOK_Y_BIAS / 2;
  return { x: lx * (1 - doze), y: ly * (1 - doze) + 16 * doze, doze, color: mix(AMBIENT_COLOR, ASLEEP_COLOR, doze) };
}

/** `format_rate()`: three figures at most, the number and its unit. */
export function formatRate(bps: number): [string, string] {
  const units = ["B/s", "KB/s", "MB/s", "GB/s"];
  let u = 0;
  bps = Math.max(0, bps);
  while (bps >= 999.5 && u < 3) {
    bps /= 1000;
    u++;
  }
  return [bps < 99.95 && u > 0 ? bps.toFixed(1) : String(cInt(bps)), units[u]];
}
const rate = (bps: number) => formatRate(bps).join(" ");

/** `net_ring_pct()` */
const netPct = (bps: number) => (bps <= 1 ? 0 : pct(Math.log10(bps) - NET_LOG_MIN, NET_LOG_MAX - NET_LOG_MIN));

/** `format_span()`: "10 min", "1 h 30 min", "45 s". */
function formatSpan(secs: number): string {
  const h = Math.trunc(secs / 3600), m = Math.trunc((secs % 3600) / 60), s = secs % 60;
  if (h > 0) return m > 0 ? `${h} h ${m} min` : `${h} h`;
  if (m > 0) return s > 0 ? `${m} min ${s} s` : `${m} min`;
  return `${s} s`;
}

const TIMER_COLORS: Record<BoardTimer["kind"], [string, string]> = {
  timer: [COLOR.warm, COLOR.timerTrack],
  work: [COLOR.tomato, COLOR.tomatoTrack],
  break: [COLOR.green, COLOR.greenTrack],
  reminder: [COLOR.cyan, COLOR.tempTrack],
};

function timerTitle(t: BoardTimer, ringing: boolean): string {
  switch (t.kind) {
    case "work":
      return ringing ? "BREAK TIME" : "FOCUS";
    case "break":
      return ringing ? "BACK TO WORK" : "BREAK";
    case "reminder":
      return "REMINDER";
    default:
      return ringing ? "TIME'S UP" : "TIMER";
  }
}

function timerView(x: Extras, stale: boolean): TimerView | null {
  const t = x.timer;
  if (!t || t.total_s <= 0) return null;
  const ringing = t.state === "ring";
  const paused = t.state === "pause";
  const left = ringing ? 0 : paused ? t.left_s : Math.max(0, t.left_s - (x.timerAgeMs ?? 0) / 1000);
  const secs = Math.max(0, Math.ceil(left - 0.05));
  const two = (n: number) => String(n).padStart(2, "0");
  const value = secs >= 3600 ? `${Math.trunc(secs / 3600)}:${two(Math.trunc((secs % 3600) / 60))}:${two(secs % 60)}` : `${Math.trunc(secs / 60)}:${two(secs % 60)}`;
  const [color, track] = TIMER_COLORS[t.kind] ?? TIMER_COLORS.timer;
  const on = !ringing || Math.trunc((x.nowMs ?? 0) / TIMER_BLINK_MS) % 2 === 0;
  const info = paused ? "paused" : t.rounds ? `round ${t.round ?? 1} of ${t.rounds}` : `of ${formatSpan(Math.trunc(t.total_s + 0.5))}`;
  return {
    pct: ringing ? 100 : Math.min(100, (left / t.total_s) * 100),
    color: on ? color : COLOR.hot,
    track,
    title: timerTitle(t, ringing),
    titleColor: stale ? COLOR.stale : color,
    value,
    small: secs >= 3600,
    valueColor: paused ? COLOR.textDim : COLOR.text,
    dim: !on,
    label: t.label ?? "",
    info,
    infoColor: paused ? COLOR.stale : COLOR.textDim,
    more: t.more ? `+${t.more} MORE` : "",
  };
}

function placeholderScreen(screen: Screen, title: string, waiting: boolean): Screen {
  return {
    ...screen,
    placeholder: true,
    title,
    value: "—",
    labelColor: COLOR.textDim,
    valueColor: COLOR.textDim,
    warn: false,
  };
}

function netScreen(screen: Screen, net: Net | undefined, stale: boolean, waiting: boolean): Screen {
  if (waiting || !net) {
    return { ...placeholderScreen(screen, "NET", waiting), net: { rxPct: 0, txPct: 0, unit: "--", tx: "--" } };
  }
  const [value, unit] = formatRate(net.rx_bps);
  return {
    ...screen,
    placeholder: false,
    title: "NET",
    value,
    labelColor: stale ? COLOR.stale : COLOR.cyan,
    valueColor: stale ? COLOR.textDim : COLOR.text,
    warn: false,
    net: { rxPct: netPct(net.rx_bps), txPct: netPct(net.tx_bps), unit, tx: rate(net.tx_bps) },
  };
}

function diskScreen(screen: Screen, disk: Disk | undefined, stale: boolean, waiting: boolean): Screen {
  if (waiting || !disk || disk.total_gb <= 0) {
    return { ...placeholderScreen(screen, "DISK", waiting), disk: { pct: 0, color: COLOR.mem, space: "-- GB", read: null, write: null } };
  }
  const p = pct(disk.used_gb, disk.total_gb);
  const full = p >= DISK_FULL_PCT;
  const space =
    disk.total_gb >= 1000
      ? `${(disk.used_gb / 1000).toFixed(1)} / ${(disk.total_gb / 1000).toFixed(1)} TB`
      : `${disk.used_gb.toFixed(0)} / ${disk.total_gb.toFixed(0)} GB`;
  const io = disk.read_bps !== undefined || disk.write_bps !== undefined;
  return {
    ...screen,
    placeholder: false,
    title: "DISK",
    value: `${p}%`,
    labelColor: stale ? COLOR.stale : full ? COLOR.warm : COLOR.mem,
    valueColor: full ? COLOR.warm : COLOR.text,
    warn: full,
    disk: { pct: p, color: full ? COLOR.warm : COLOR.mem, space, read: io ? rate(disk.read_bps ?? 0) : null, write: io ? rate(disk.write_bps ?? 0) : null },
  };
}

function batteryScreen(screen: Screen, bat: Battery | undefined, stale: boolean, waiting: boolean): Screen {
  if (waiting || !bat) {
    return {
      ...placeholderScreen(screen, "BATTERY", waiting),
      battery: { pct: 0, color: COLOR.green, status: waiting ? "WAITING" : "NO BATTERY", statusColor: COLOR.textDim, time: "" },
    };
  }
  const p = pct(bat.pct, 100);
  const color = !bat.plugged && p <= BATTERY_EMPTY_PCT ? COLOR.hot : !bat.plugged && p <= BATTERY_LOW_PCT ? COLOR.warm : COLOR.green;
  const status = bat.charging ? "CHARGING" : bat.plugged ? "PLUGGED IN" : "ON BATTERY";
  let time = "";
  if (bat.mins !== undefined && bat.mins > 0 && (bat.charging || !bat.plugged)) {
    const what = bat.charging ? "to full" : "left";
    time = bat.mins >= 60 ? `${Math.trunc(bat.mins / 60)}h ${String(bat.mins % 60).padStart(2, "0")}m ${what}` : `${bat.mins}m ${what}`;
  }
  return {
    ...screen,
    placeholder: false,
    title: "BATTERY",
    value: `${p}%`,
    labelColor: stale ? COLOR.stale : color,
    valueColor: color === COLOR.green ? COLOR.text : color,
    warn: color === COLOR.hot,
    battery: { pct: p, color, status, statusColor: bat.charging || bat.plugged ? COLOR.green : COLOR.textDim, time },
  };
}

/** `format_tokens()`: "1.2M", "845K", "9.4K", "512". */
export function formatTokens(t: number): string {
  if (t < 1000) return String(Math.trunc(t));
  if (t < 9950) return `${(t / 1e3).toFixed(1)}K`;
  if (t < 999500) return `${(t / 1e3).toFixed(0)}K`;
  if (t < 99950000) return `${(t / 1e6).toFixed(1)}M`;
  return `${(t / 1e6).toFixed(0)}M`;
}

const limitColor = (p: number, normal: string) => (p >= CLAUDE_HOT_PCT ? COLOR.hot : p >= CLAUDE_WARM_PCT ? COLOR.warm : normal);

function claudeView(c: ClaudeMetrics | undefined, stale: boolean, waiting: boolean): ClaudeView {
  const live = !waiting && c !== undefined;
  const asleep = !live || c.state === "sleep";
  const base = {
    mascot: live ? c.state : ("idle" as ClaudeState),
    mascotColor: asleep ? COLOR.claudeDim : COLOR.claude,
    weekColor: COLOR.week,
  };
  if (!live) {
    return {
      ...base,
      sessionPct: 0,
      sessionColor: COLOR.claude,
      showWeek: true,
      weekPct: 0,
      value: "—",
      valueColor: COLOR.textDim,
      reset: "--",
      weekName: "WK",
      week: "--",
      model: "CLAUDE",
      status: waiting ? "WAITING" : "NO DATA",
      statusColor: COLOR.textDim,
      tokens: "--",
    };
  }
  // update_session_arc(): the 5-hour limit, else how far into the window.
  const session = c.s_pct !== undefined ? pct(c.s_pct, 100) : undefined;
  const sessionPct = session ?? (c.left_min !== undefined ? pct(CLAUDE_BLOCK_MIN - c.left_min, CLAUDE_BLOCK_MIN) : 0);
  const week = c.w_pct !== undefined ? pct(c.w_pct, 100) : undefined;
  const left = c.left_min;
  const reset = left === undefined ? "--" : left >= 60 ? `${Math.trunc(left / 60)}h ${String(left % 60).padStart(2, "0")}m` : `${left}m`;
  const status = { work: "WORKING", idle: "IDLE", sleep: "ASLEEP" }[c.state];
  return {
    ...base,
    sessionPct,
    sessionColor: session !== undefined ? limitColor(session, COLOR.claude) : COLOR.claude,
    showWeek: week !== undefined,
    weekPct: week ?? 0,
    weekColor: week !== undefined ? limitColor(week, COLOR.week) : COLOR.week,
    value: session !== undefined ? `${session}%` : formatTokens(c.tok),
    valueColor: stale ? COLOR.textDim : session !== undefined ? limitColor(session, COLOR.text) : COLOR.text,
    reset,
    weekName: week !== undefined ? "WK" : "DAY",
    week: week !== undefined ? `${week}%` : formatTokens(c.today),
    model: c.model || "CLAUDE",
    status,
    statusColor: c.state === "work" ? COLOR.claude : COLOR.textDim,
    tokens: formatTokens(c.tok),
  };
}

/** Clawd on its 16 × 5 grid, as `clawd_pose()` and `clawd_animate()` lay it out. */
export const CLAWD_COLS = 16;
export const CLAWD_ROWS = 5;
export const CLAWD_TICK_MS = 150;
export type Rect = { x: number; y: number; w: number; h: number };
export function clawdPose(px: number, state: ClaudeState, tick: number) {
  const slit = Math.max(1, Math.trunc(px / 4));
  const lift = Math.max(1, Math.trunc(px / 4));
  let bob = 0;
  let liftA = false;
  let liftB = false;
  let eyeH = px;
  let low = false;
  let zzz = "";
  if (state === "work") {
    const phase = tick % 4;
    bob = phase % 2 ? lift : 0;
    liftA = phase < 2;
    liftB = phase >= 2;
  } else if (state === "idle") {
    if (tick % 24 === 0) eyeH = slit;
  } else {
    eyeH = slit;
    low = true;
    zzz = ["z", "z Z", "z Z z", ""][Math.trunc(tick / 5) % 4];
  }
  const body: Rect[] = [
    { x: 2 * px, y: -bob, w: 12 * px, h: 4 * px },
    { x: 0, y: 2 * px - bob, w: CLAWD_COLS * px, h: px },
    ...[3, 5, 10, 12].map((col, i) => ({ x: col * px, y: 4 * px, w: px, h: (i % 2 === 0 ? liftA : liftB) ? Math.trunc(px / 2) : px })),
  ];
  const eyes: Rect[] = [4, 11].map((col) => ({
    x: col * px,
    y: px - bob + (low ? px - eyeH : Math.trunc((px - eyeH) / 2)),
    w: px,
    h: eyeH,
  }));
  return { body, eyes, zzz };
}

function sensorScreen(
  id: DeviceId,
  face: Face,
  m: Metrics | undefined,
  stale: boolean,
  waiting: boolean,
  fan?: number,
): Screen {
  const dev = DEVICES[id];
  const accent = dev.accent;
  const mem = m?.mem && m.mem.total_mb > 0 ? m.mem : undefined;
  const memPct = mem ? pct(mem.used_mb, mem.total_mb) : undefined;
  const base = {
    face,
    source: id,
    title: dev.title,
    memName: dev.memTitle,
    clock: "-- GHz",
    watts: "-- W",
    usage: "--%",
    rpm: "--",
    mem: "--%",
    memValue: "-- GB",
    usagePct: 0,
    tempPct: 0,
    memPct: 0,
    tempRing: COLOR.cyan,
    memColor: COLOR.mem,
  };
  if (waiting || m?.temp_c === undefined) {
    return {
      ...base,
      placeholder: true,
      value: "—",
      labelColor: COLOR.textDim,
      valueColor: COLOR.textDim,
      usageColor: COLOR.textDim,
      memColor: COLOR.textDim,
      barColor: COLOR.textDim,
      warn: false,
    };
  }

  // metrics_parser.c leaves absent fields at 0.
  const temp = m!.temp_c!;
  const usage = m!.load_pct ?? 0;
  let labelColor: string = accent;
  let valueColor: string = COLOR.text;
  let warn = false;
  if (temp >= TEMP_HOT_C) {
    labelColor = valueColor = COLOR.hot;
    warn = true;
  } else if (temp >= TEMP_WARM_C) {
    labelColor = valueColor = COLOR.warm;
    warn = true;
  } else if (stale) {
    labelColor = COLOR.stale;
    valueColor = COLOR.textDim;
  }
  const usagePct = pct(usage, 100);
  return {
    ...base,
    placeholder: false,
    value: `${cInt(temp)}°`,
    clock: `${((m!.clock_mhz ?? 0) / 1000).toFixed(1)} GHz`,
    watts: `${(m!.power_w ?? 0).toFixed(0)} W`,
    // The classic-based faces print the load with "%.0f", rings the clamped ring value.
    usage: face === "rings" ? pctText(usagePct) : `${usage.toFixed(0)}%`,
    rpm: fan === undefined ? "--" : String(fan),
    mem: pctText(memPct),
    memValue: mem ? `${(mem.used_mb / 1024).toFixed(1)}/${(mem.total_mb / 1024).toFixed(0)} GB` : "-- GB",
    usagePct,
    tempPct: pct(temp, TEMP_MAX_C),
    memPct: memPct ?? 0,
    tempRing: temp >= TEMP_HOT_C ? COLOR.hot : temp >= TEMP_WARM_C ? COLOR.warm : COLOR.cyan,
    labelColor,
    valueColor,
    usageColor: accent,
    memColor: memPct === undefined ? COLOR.textDim : COLOR.mem,
    barColor: memPct === undefined ? COLOR.textDim : memPct >= MEM_HIGH_PCT ? COLOR.warm : COLOR.mem,
    warn,
  };
}

/** Colour a temperature the way the board would: accent, then warm, then hot. */
export function heatColor(id: DeviceId, temp: number | undefined): string {
  if (temp !== undefined && temp >= TEMP_HOT_C) return COLOR.hot;
  if (temp !== undefined && temp >= TEMP_WARM_C) return COLOR.warm;
  return DEVICES[id].accent;
}

// Font Awesome Free (CC BY 4.0): "fan" U+F863 and LV_SYMBOL_WARNING.
export const FAN_PATH =
  "M160 144c0-79.5 64.5-144 144-144 8.8 0 16 7.2 16 16l0 152.2c15-5.3 31.2-8.2 48-8.2 79.5 0 144 64.5 144 144 0 8.8-7.2 16-16 16l-152.2 0c5.3 15 8.2 31.2 8.2 48 0 79.5-64.5 144-144 144-8.8 0-16-7.2-16-16l0-152.2c-15 5.3-31.2 8.2-48 8.2-79.5 0-144-64.5-144-144 0-8.8 7.2-16 16-16l152.2 0c-5.3-15-8.2-31.2-8.2-48zm96 144a32 32 0 1 0 0-64 32 32 0 1 0 0 64z";
export const WARN_PATH =
  "M256 0c14.7 0 28.2 8.1 35.2 21l216 400c6.7 12.4 6.4 27.4-.8 39.5S486.1 480 472 480L40 480c-14.1 0-27.2-7.4-34.4-19.5s-7.5-27.1-.8-39.5l216-400c7-12.9 20.5-21 35.2-21zm0 352a32 32 0 1 0 0 64 32 32 0 1 0 0-64zm0-192c-18.2 0-32.7 15.5-31.4 33.7l7.4 104c.9 12.5 11.4 22.3 23.9 22.3 12.6 0 23-9.7 23.9-22.3l7.4-104c1.3-18.2-13.1-33.7-31.4-33.7z";
