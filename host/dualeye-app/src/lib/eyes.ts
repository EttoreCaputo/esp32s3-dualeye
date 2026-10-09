// The board's voice eyes (main/ui_eyes.c), the same springs and expressions,
// for the mirror. One engine for both screens, so they blink together.
import type { VoiceState } from "./monitor.svelte";

const COLORS: Record<VoiceState, string> = {
  idle: "#30d5f0",
  listening: "#30d5f0",
  thinking: "#ffb020",
  speaking: "#40e080",
  error: "#ff4040",
};

const SHAPE_K = 220;
const SHAPE_ZETA = 0.5;
const GAZE_K = 650;
const GAZE_ZETA = 0.85;
const SPRING_STEP_S = 0.01;
const COLOR_RATE = 0.25;
const FRAME_MS = 40;
const CLOSED_H = 12;
const CLOSE_MIN_MS = 260;
const BLINK_DOWN_MS = 70;
const BLINK_HOLD_MS = 30;
const BLINK_UP_MS = 80;
const BLINK_SHUT = 0.07;
const RADIUS_SHARE = 0.3;
const SMILE_SHARE = 1.6;
const LCD = 240;

type Params = { w: number; h: number; x: number; y: number; lidIn: number; lidOut: number; happy: number };
type Spring = { v: number; vel: number };
const KEYS = ["w", "h", "x", "y", "lidIn", "lidOut", "happy"] as const;

export type Geom = {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
  radius: number;
  lidLeft: number;
  lidRight: number;
  happyTop: number;
};

const rand = (min: number, max: number) => min + Math.floor(Math.random() * (max - min + 1));
const hex = (c: string) => [1, 3, 5].map((i) => parseInt(c.slice(i, i + 2), 16));

class Eyes {
  open = false;
  #state: VoiceState = "idle";
  #stateMs = 0;
  #lastMs = 0;
  #levelIn = 0;
  #level = 0;
  #rgb = hex(COLORS.listening);
  #nextBlink = 0;
  #blinkAt = 0;
  #blinking = false;
  #blinkAgain = false;
  #blink = 1;
  #nextGlance = 0;
  #gazeX = 0;
  #gazeY = 0;
  // Last: shut eyes are worked out from the fields above.
  #springs: Record<(typeof KEYS)[number], Spring>[] = [0, 1].map(() => this.#shut());

  #shut() {
    const p = this.#expression("idle", 0, 0);
    return Object.fromEntries(KEYS.map((k) => [k, { v: p[k], vel: 0 }])) as Record<(typeof KEYS)[number], Spring>;
  }

  #expression(state: VoiceState, eye: number, t: number): Params {
    const level = this.#level;
    const p: Params = { w: 0, h: 0, x: 0, y: 0, lidIn: 0, lidOut: 0, happy: 0 };
    switch (state) {
      case "listening":
        p.w = 112 * (1 + 0.1 * level);
        p.h = 136 * (1 + 0.1 * level);
        p.x = this.#gazeX;
        p.y = this.#gazeY;
        break;
      case "thinking":
        p.w = 108;
        p.h = eye === 0 ? 92 : 104;
        p.x = 26 * Math.sin((Math.PI * 2 * t) / 3.2);
        p.y = -24;
        p.lidIn = p.lidOut = eye === 0 ? 0.18 : 0;
        break;
      case "speaking":
        p.w = 118;
        p.h = 118 * (1 + 0.1 * level);
        p.x = this.#gazeX * 0.5;
        p.y = -6 - 12 * level;
        p.happy = 0.28 + 0.18 * level;
        break;
      case "error":
        p.w = 110;
        p.h = 104;
        p.x = 12 * Math.sin(Math.PI * 2 * 6 * t) * Math.exp(-2.5 * t);
        p.y = 6;
        p.lidIn = 0.05;
        p.lidOut = 0.38;
        break;
      default:
        p.w = 124;
        p.h = 6;
    }
    return p;
  }

  show(state: VoiceState, now = performance.now()) {
    if (state === this.#state) return;
    const was = this.#state;
    this.#state = state;
    this.#stateMs = now;
    if (state === "idle") return;
    if (!this.open) {
      this.#rgb = hex(COLORS[state]);
      this.#springs = [0, 1].map(() => this.#shut());
      this.#level = this.#levelIn = 0;
      this.#gazeX = this.#gazeY = 0;
      this.#lastMs = now;
      this.open = true;
    } else if (state === "listening" && was !== "idle") {
      for (const s of this.#springs) s.h.vel += 320;
    }
    this.#blinking = this.#blinkAgain = false;
    this.#blink = 1;
    this.#nextBlink = now + rand(900, 2200);
    this.#nextGlance = now + rand(700, 1500);
  }

  /** The mirror has no audio level: the board keeps it. */
  setLevel(level: number) {
    this.#levelIn = Math.min(1, Math.max(0, level));
  }

  /** Advance to `now`, a frame at a time like the board. */
  step(now: number) {
    if (!this.open || now - this.#lastMs < FRAME_MS) return;
    const dt = Math.min(0.08, Math.max(0.001, (now - this.#lastMs) / 1000));
    this.#lastMs = now;
    const t = (now - this.#stateMs) / 1000;
    this.#level += (this.#levelIn - this.#level) * Math.min(1, dt * 12);
    this.#blinkStep(now);
    this.#glanceStep(now);
    const to = hex(COLORS[this.#state]);
    this.#rgb = this.#rgb.map((c, i) => c + (to[i] - c) * COLOR_RATE);
    let shut = true;
    this.#springs.forEach((s, eye) => {
      const target = this.#expression(this.#state, eye, t);
      for (const k of KEYS) {
        const gaze = k === "x" || k === "y";
        const kk = gaze ? GAZE_K : SHAPE_K;
        const c = 2 * (gaze ? GAZE_ZETA : SHAPE_ZETA) * Math.sqrt(kk);
        // In small steps, or a late frame throws a stiff spring off to infinity.
        const n = Math.ceil(dt / SPRING_STEP_S);
        for (let i = 0; i < n; i++) {
          s[k].vel += (kk * (target[k] - s[k].v) - c * s[k].vel) * (dt / n);
          s[k].v += s[k].vel * (dt / n);
        }
      }
      shut &&= s.h.v < CLOSED_H;
    });
    if (this.#state === "idle" && shut && now - this.#stateMs >= CLOSE_MIN_MS) this.open = false;
  }

  #blinkStep(now: number) {
    if (this.#state === "idle" || this.#state === "error") {
      this.#blinking = false;
      this.#blink = 1;
      return;
    }
    if (!this.#blinking && now >= this.#nextBlink) {
      this.#blinking = true;
      this.#blinkAt = now;
    }
    if (!this.#blinking) {
      this.#blink = 1;
      return;
    }
    const t = now - this.#blinkAt;
    if (t < BLINK_DOWN_MS) this.#blink = 1 - ((1 - BLINK_SHUT) * t) / BLINK_DOWN_MS;
    else if (t < BLINK_DOWN_MS + BLINK_HOLD_MS) this.#blink = BLINK_SHUT;
    else if (t < BLINK_DOWN_MS + BLINK_HOLD_MS + BLINK_UP_MS)
      this.#blink = BLINK_SHUT + ((1 - BLINK_SHUT) * (t - BLINK_DOWN_MS - BLINK_HOLD_MS)) / BLINK_UP_MS;
    else {
      this.#blink = 1;
      this.#blinking = false;
      if (this.#blinkAgain) {
        this.#blinkAgain = false;
        this.#nextBlink = now + 90;
      } else {
        this.#nextBlink = now + (this.#state === "thinking" ? rand(3500, 7000) : rand(2200, 5500));
        this.#blinkAgain = rand(0, 4) === 0;
      }
    }
  }

  #glanceStep(now: number) {
    if (this.#state !== "listening" && this.#state !== "speaking") {
      this.#gazeX = this.#gazeY = 0;
      return;
    }
    if (now < this.#nextGlance) return;
    if (rand(0, 9) < 3) this.#gazeX = this.#gazeY = 0;
    else {
      this.#gazeX = rand(0, 24) - 12;
      this.#gazeY = rand(0, 16) - 8;
    }
    this.#nextGlance = now + rand(900, 2800);
  }

  get color() {
    return `rgb(${this.#rgb.map((c) => Math.round(c)).join(",")})`;
  }

  /** Eye 0 is the left (CPU) screen, in 240 px panel coordinates. */
  geom(eye: number): Geom {
    const p = this.#springs[eye];
    const w = Math.max(p.w.v, 4);
    const h = Math.max(p.h.v * this.#blink, 3);
    const cx = LCD / 2 + p.x.v;
    const cy = LCD / 2 + p.y.v;
    const lidIn = Math.max(p.lidIn.v, 0) * h;
    const lidOut = Math.max(p.lidOut.v, 0) * h;
    return {
      x0: cx - w / 2,
      x1: cx + w / 2,
      y0: cy - h / 2,
      y1: cy + h / 2,
      radius: Math.min(Math.min(w, h) * RADIUS_SHARE, Math.min(w, h) / 2),
      lidLeft: eye === 0 ? lidOut : lidIn,
      lidRight: eye === 0 ? lidIn : lidOut,
      happyTop: p.happy.v > 0.02 ? cy + h / 2 - p.happy.v * h * 0.8 : 0,
    };
  }
}

export const eyes = new Eyes();

/** One frame of eye `eye` on a canvas `size` px across. */
export function drawEye(ctx: CanvasRenderingContext2D, eye: number, size: number) {
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, size, size);
  if (!eyes.open) return;
  paintEye(ctx, eyes.geom(eye), eyes.color, size);
}

/** An eye `g` in `color` on a black screen `size` px across, where `ctx` is now. */
export function paintEye(ctx: CanvasRenderingContext2D, g: Geom, color: string, size: number) {
  ctx.save();
  ctx.scale(size / LCD, size / LCD);
  ctx.fillStyle = "#000";
  ctx.fillRect(0, 0, LCD, LCD);
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.roundRect(g.x0, g.y0, g.x1 - g.x0, g.y1 - g.y0, g.radius);
  ctx.fill();
  ctx.fillStyle = "#000";
  if (g.lidLeft > 0 || g.lidRight > 0) {
    ctx.beginPath();
    ctx.moveTo(g.x0 - 2, g.y0 - 2);
    ctx.lineTo(g.x1 + 2, g.y0 - 2);
    ctx.lineTo(g.x1 + 2, g.y0 + g.lidRight);
    ctx.lineTo(g.x0 - 2, g.y0 + g.lidLeft);
    ctx.fill();
  }
  if (g.happyTop > 0) {
    const r = ((g.x1 - g.x0) * SMILE_SHARE) / 2;
    ctx.beginPath();
    ctx.arc((g.x0 + g.x1) / 2, g.happyTop + r, r, 0, Math.PI * 2);
    ctx.fill();
  }
  ctx.restore();
}
