// The eyes' scenes (SKITS in main/ui_eyes.c), the same poses and springs, for
// the gallery: each one plays here over and over, and on the board when picked.
import { paintEye, type Geom } from "./eyes";

type Params = { w: number; h: number; x: number; y: number; lidIn: number; lidOut: number; happy: number };
type Pose = (eye: number, t: number, p: Params) => void;

export type Scene = {
  name: string;
  label: string;
  /** "rainbow": going round the hues. */
  color: string;
  ms: number;
  blinks: boolean;
  pose: Pose;
  /** [ms in, sound name] */
  cues: [number, string][];
  mood: Mood;
};

export type Mood = "joy" | "calm" | "sleepy" | "grumpy" | "silly" | "robot" | "life";

export const MOODS: [Mood, string][] = [
  ["joy", "Joy"],
  ["calm", "Calm"],
  ["sleepy", "Sleepy"],
  ["grumpy", "Grumpy"],
  ["silly", "Silly"],
  ["robot", "Robot"],
  ["life", "Reactions"],
];

const PI2 = Math.PI * 2;
const { sin, cos, abs, exp, min, max } = Math;
const fmod = (a: number, b: number) => a % b;
const smooth01 = (x: number) => {
  x = x < 0 ? 0 : x > 1 ? 1 : x;
  return x * x * (3 - 2 * x);
};
/** hash32() in ui_eyes.c. */
function hash32(n: number) {
  n = Math.imul(n >>> 0, 2654435761);
  n = (n ^ (n >>> 15)) >>> 0;
  n = Math.imul(n, 2246822519);
  n = (n ^ (n >>> 13)) >>> 0;
  return n;
}

const openPose = (p: Params) => {
  p.w = 110;
  p.h = 124;
};

const S = (
  name: string,
  label: string,
  mood: Mood,
  color: string,
  ms: number,
  blinks: boolean,
  cues: [number, string][],
  pose: Pose,
): Scene => ({ name, label, mood, color, ms, blinks, cues, pose });

export const SCENES: Scene[] = [
  S("look_around", "Look around", "calm", "#30d5f0", 4300, true, [[3000, "hmm"]], (eye, t, p) => {
    openPose(p);
    p.x = t < 0.8 ? 0 : t < 1.9 ? -36 : t < 3.0 ? 36 : t < 3.7 ? 10 : 0;
    p.y = t >= 3.0 && t < 3.7 ? -26 : t >= 0.8 && t < 3.0 ? 4 : 0;
  }),
  S("sleepy", "Sleepy", "sleepy", "#6a8cff", 6000, false, [[300, "yawn"], [3400, "startle"]], (eye, t, p) => {
    openPose(p);
    if (t < 3.4) {
      const d = min(t / 3.4, 1);
      p.h = 104 - 80 * d;
      p.lidIn = p.lidOut = 0.35 + 0.25 * d;
      p.y = 6 + 10 * d;
    } else if (t < 4.2) {
      p.w = 120;
      p.h = 146;
      p.y = -6;
    } else {
      p.h = 96;
      p.lidIn = p.lidOut = 0.4;
      p.y = 6;
    }
  }),
  S("suspicious", "Suspicious", "grumpy", "#30d5f0", 4200, false, [[500, "hmm"]], (eye, t, p) => {
    p.w = 114;
    p.h = eye === 0 ? 64 : 78;
    p.lidIn = p.lidOut = 0.2;
    p.x = t < 0.5 ? 0 : t < 1.8 ? -30 : t < 3.1 ? 30 : t < 3.6 ? -30 : 0;
  }),
  S("happy", "Happy", "joy", "#40e080", 3500, true, [[200, "happy"]], (eye, t, p) => {
    p.w = 118;
    p.h = 116;
    p.happy = 0.42;
    p.y = -6 - 10 * abs(sin(PI2 * 0.8 * t));
  }),
  S("surprised", "Surprised", "calm", "#30d5f0", 3300, false, [[450, "gasp"]], (eye, t, p) => {
    openPose(p);
    if (t >= 0.5 && t < 2.4) {
      p.w = 134;
      p.h = 170;
      p.y = -8;
    }
  }),
  S("wink", "Wink", "joy", "#30d5f0", 2800, false, [[850, "wink"]], (eye, t, p) => {
    openPose(p);
    const winking = t >= 0.9 && t < 1.8;
    if (winking) {
      p.happy = 0.3;
      if (eye === 1) {
        p.h = 8;
        p.happy = 0;
      }
    }
    p.y = winking ? -4 : 0;
  }),
  S("angry", "Angry", "grumpy", "#ff5a30", 3200, false, [[400, "grumble"]], (eye, t, p) => {
    p.w = 118;
    p.h = 104;
    p.lidIn = 0.46;
    p.y = 4;
    p.x = t > 0.5 && t < 2.0 ? 3 * sin(PI2 * 9 * t) : 0;
  }),
  S("sad", "Sad", "grumpy", "#4a7bff", 4200, true, [[500, "aww"]], (eye, t, p) => {
    p.w = 108;
    p.h = 100;
    p.lidOut = 0.44;
    p.y = t < 0.6 ? 4 : 18;
    p.x = t < 0.6 ? 0 : -8;
  }),
  S("dizzy", "Dizzy", "silly", "#c070ff", 4000, false, [[100, "boing"]], (eye, t, p) => {
    const r = 24 * min(1, t / 0.5) * min(1, max(0, (4 - t) / 1.2));
    const a = PI2 * 1.3 * t;
    p.w = 98;
    p.h = 98;
    p.x = r * cos(eye === 0 ? a : -a);
    p.y = r * sin(eye === 0 ? a : -a);
  }),
  S("cross_eyed", "Cross-eyed", "silly", "#30d5f0", 3000, false, [[450, "boop"]], (eye, t, p) => {
    p.w = 96;
    p.h = 110;
    const crossed = t >= 0.5 && t < 2.4;
    p.x = crossed ? (eye === 0 ? 32 : -32) : 0;
    p.y = crossed ? 10 : 0;
  }),
  S("eye_roll", "Eye roll", "grumpy", "#30d5f0", 3400, false, [[1800, "pfft"]], (eye, t, p) => {
    p.w = 112;
    p.h = 110;
    if (t >= 0.5 && t < 1.9) {
      const a = (Math.PI * (t - 0.5)) / 1.4;
      p.x = -32 * cos(a);
      p.y = -30 * sin(a) - 6;
    } else if (t >= 1.9) {
      p.h = 86;
      p.lidIn = p.lidOut = 0.3;
    }
  }),
  S("curious", "Curious", "calm", "#30d5f0", 4000, true, [[400, "hmm"], [2000, "chirp"]], (eye, t, p) => {
    const big = (eye === 0) === t < 2.0;
    p.w = big ? 122 : 100;
    p.h = big ? 150 : 96;
    p.lidIn = big ? 0 : 0.16;
    p.x = t < 0.4 ? 0 : t < 2.0 ? 26 : -26;
    p.y = t < 0.4 ? 0 : -10;
  }),
  S("love", "Love", "joy", "#ff5aa8", 4000, false, [[0, "heartbeat"], [1000, "heartbeat"], [2000, "heartbeat"], [3000, "heartbeat"]], (eye, t, p) => {
    const beat = fmod(t, 1);
    const pulse = exp(-40 * beat * beat) + 0.7 * exp(-40 * (beat - 0.22) * (beat - 0.22));
    p.w = 112 * (1 + 0.12 * pulse);
    p.h = 118 * (1 + 0.12 * pulse);
    p.happy = 0.36;
    p.y = -4;
  }),
  S("scan", "Scan", "robot", "#30f0b0", 3800, false, [[300, "scan"]], (eye, t, p) => {
    p.w = 128;
    p.h = t < 0.3 || t > 3.4 ? 110 : 30;
    p.x = t < 0.5 || t > 3.2 ? 0 : 40 * sin((PI2 * (t - 0.5)) / 1.35);
  }),
  S("shy", "Shy", "calm", "#ff8ab0", 3600, false, [[400, "eep"]], (eye, t, p) => {
    p.w = 106;
    p.h = 104;
    p.happy = 0.26;
    const peek = t >= 2.2 && t < 2.8;
    p.x = t < 0.4 ? 0 : peek ? -6 : -30;
    p.y = t < 0.4 ? 0 : peek ? 2 : 20;
  }),
  S("flutter", "Flutter", "silly", "#30d5f0", 2600, false, [[600, "flutter"]], (eye, t, p) => {
    openPose(p);
    const burst = t >= 0.6 && t < 1.8;
    if (burst && fmod(t - 0.6, 0.3) < 0.12) p.h = 10;
    p.y = burst ? 4 : 0;
  }),
  S("yawn", "Yawn", "sleepy", "#7a9cff", 4200, false, [[300, "yawn"]], (eye, t, p) => {
    openPose(p);
    if (t >= 0.3 && t < 1.5) {
      p.w = 132;
      p.h = 22;
      p.y = -10;
    } else if (t >= 1.5) {
      p.h = t >= 2.6 && t < 2.9 ? 10 : t < 2.6 ? 70 : 92;
      p.lidIn = p.lidOut = 0.3;
      p.y = 6;
    }
  }),
  S("sneeze", "Sneeze", "silly", "#30d5f0", 3400, false, [[500, "sneeze"]], (eye, t, p) => {
    openPose(p);
    if (t >= 0.5 && t < 0.74) {
      p.w = 118;
      p.h = 146;
      p.y = -10;
    } else if (t >= 0.74 && t < 0.84) {
      p.h = 128;
      p.y = -4;
    } else if (t >= 0.84 && t < 1.18) {
      p.w = 124;
      p.h = 160;
      p.y = -16;
    } else if (t >= 1.18 && t < 1.5) {
      p.w = 134;
      p.h = 8;
      p.y = 18;
    } else if (t >= 1.5 && t < 2.4) {
      p.x = 10 * sin(PI2 * 7 * t) * exp(-3 * (t - 1.5));
      p.h = 110;
      p.lidOut = 0.15;
    }
  }),
  S("giggle", "Giggle", "joy", "#40e080", 3200, false, [[300, "giggle"], [1700, "giggle"]], (eye, t, p) => {
    p.w = 118;
    p.h = 104;
    p.happy = 0.5;
    const shake = t > 0.3 ? abs(sin(PI2 * 4.5 * t)) : 0;
    p.y = -4 - 8 * shake;
  }),
  S("excited", "Excited", "joy", "#ffd040", 3400, false, [[200, "excited"], [1700, "excited"]], (eye, t, p) => {
    const pulse = sin(PI2 * 2.5 * t);
    p.w = 124 + 6 * pulse;
    p.h = 146 + 8 * pulse;
    p.y = -10 - 18 * abs(sin(PI2 * 1.25 * t));
    p.happy = 0.22;
  }),
  S("bored", "Bored", "sleepy", "#8aa4c0", 4600, true, [[1100, "sigh"]], (eye, t, p) => {
    p.w = 112;
    p.h = 72;
    p.lidIn = p.lidOut = 0.42;
    p.y = t < 1.2 ? 4 : 14;
    p.x = t < 2.5 ? 0 : -24;
  }),
  S("confused", "Confused", "silly", "#c0a0ff", 3800, true, [[400, "confused"]], (eye, t, p) => {
    const raised = (eye === 0) !== t >= 2.0;
    p.w = raised ? 116 : 104;
    p.h = raised ? 140 : 92;
    p.y = raised ? -14 : 8;
    p.lidIn = raised ? 0 : 0.28;
    p.x = t < 0.3 ? 0 : 10 * sin(PI2 * 0.5 * t);
  }),
  S("scared", "Scared", "grumpy", "#a0c8ff", 3800, false, [[300, "whimper"]], (eye, t, p) => {
    p.w = 84;
    p.h = 100;
    p.lidOut = 0.25;
    p.x = (t < 0.3 ? 0 : fmod(t, 1.2) < 0.6 ? -22 : 22) + 2.5 * sin(PI2 * 13 * t);
    p.y = 6;
  }),
  S("peekaboo", "Peekaboo", "silly", "#40e0c0", 3800, false, [[2000, "boo"], [2500, "giggle"]], (eye, t, p) => {
    if (t < 0.4) openPose(p);
    else if (t < 2.0) {
      p.w = 124;
      p.h = 6;
    } else {
      p.w = 128;
      p.h = 150;
      p.happy = t > 2.4 ? 0.4 : 0;
      p.y = -10 - 10 * abs(sin(PI2 * 1.5 * (t - 2.0)));
    }
  }),
  S("nod", "Nod", "joy", "#40e080", 2800, true, [[400, "yes"]], (eye, t, p) => {
    openPose(p);
    p.happy = 0.2;
    p.y = t >= 0.4 && t < 2.0 ? 18 * abs(sin(Math.PI * 1.875 * (t - 0.4))) : 0;
  }),
  S("shake", "Shake head", "grumpy", "#30d5f0", 2800, false, [[400, "nope"]], (eye, t, p) => {
    p.w = 110;
    p.h = 104;
    p.lidIn = p.lidOut = 0.14;
    p.x = t >= 0.4 && t < 2.0 ? 26 * sin(PI2 * 1.9 * (t - 0.4)) : 0;
  }),
  S("hiccup", "Hiccup", "silly", "#30d5f0", 4200, true, [[600, "hiccup"], [1800, "hiccup"], [3000, "hiccup"]], (eye, t, p) => {
    openPose(p);
    if (t >= 0.6 && fmod(t - 0.6, 1.2) < 0.14) {
      p.y = -20;
      p.w = 104;
      p.h = 150;
    }
  }),
  S("mischief", "Mischief", "silly", "#b060ff", 3800, false, [[600, "mischief"]], (eye, t, p) => {
    p.w = 118;
    p.h = 86;
    p.lidIn = 0.34;
    p.happy = 0.3;
    p.x = t < 0.5 ? 0 : t < 2.2 ? -22 : 22;
    p.y = 4;
  }),
  S("dance", "Dance", "joy", "rainbow", 4400, false, [[0, "beat"], [2000, "beat"]], (eye, t, p) => {
    p.w = 116;
    p.h = 120;
    p.happy = 0.3;
    p.y = 4 - 18 * exp(-8 * fmod(t, 0.5));
    p.x = 20 * sin(Math.PI * t);
  }),
  S("sing", "Sing", "joy", "#70e0ff", 3200, false, [[300, "sing"]], (eye, t, p) => {
    p.w = 114;
    p.h = 100;
    p.happy = 0.45;
    p.x = 12 * sin(PI2 * 0.6 * t);
    p.y = -4 - (t >= 0.3 && t < 1.6 ? 6 * abs(sin(Math.PI * 4.55 * (t - 0.3))) : 0);
  }),
  S("purr", "Purr", "calm", "#ffb070", 3600, false, [[400, "purr"], [2000, "purr"]], (eye, t, p) => {
    const breath = sin(PI2 * 0.6 * t);
    p.w = 120 + 3 * breath;
    p.h = 76;
    p.happy = 0.55;
    p.lidIn = p.lidOut = 0.1;
    p.y = 2 + 3 * breath;
  }),
  S("sigh", "Sigh", "sleepy", "#5a8ae0", 3600, false, [[500, "sigh"]], (eye, t, p) => {
    openPose(p);
    if (t >= 0.5 && t < 0.8) {
      p.w = 114;
      p.h = 136;
      p.y = -12;
    } else if (t >= 0.8) {
      const d = smooth01((t - 0.8) / 0.9);
      p.w = 112;
      p.h = 136 - 50 * d;
      p.lidIn = p.lidOut = 0.32 * d;
      p.y = -12 + 24 * d;
    }
  }),
  S("focus", "Focus", "robot", "#40ffd0", 3200, false, [[500, "lock"]], (eye, t, p) => {
    if (t < 0.4) {
      openPose(p);
      return;
    }
    const zoom = t >= 0.5 && t < 0.9;
    p.w = zoom ? 96 : 106;
    p.h = zoom ? 54 : 68;
    p.lidIn = 0.12;
    p.lidOut = 0.06;
    p.x = eye === 0 ? 8 : -8;
  }),
  S("snore", "Snore", "sleepy", "#34507a", 6500, false, [[1500, "snore"], [4000, "snore"]], (eye, t, p) => {
    if (t < 1.2) {
      const d = smooth01(t / 1.2);
      p.w = 110 + 14 * d;
      p.h = 124 - 114 * d;
      p.lidIn = p.lidOut = 0.3 * min(1, 2 * d);
      p.y = 16 * d;
      return;
    }
    const b = fmod(t - 1.5 + 2.5, 2.5);
    p.w = 124;
    p.h = 8 + (b < 0.9 ? 8 * sin((Math.PI * b) / 0.9) : 0);
    p.y = 16;
  }),
  S("glitch", "Glitch", "robot", "#30f0b0", 2800, false, [[300, "glitch"], [1400, "glitch"]], (eye, t, p) => {
    openPose(p);
    if (t > 0.3 && t < 2.2) {
      const h = hash32(Math.floor(t * 14) + eye * 7);
      if (h & 1) {
        p.x = ((h >>> 1) % 60) - 30;
        p.y = ((h >>> 7) % 40) - 20;
        p.w = 60 + ((h >>> 13) % 90);
        p.h = 20 + ((h >>> 19) % 140);
      }
    }
  }),
  S("proud", "Proud", "joy", "#ffd040", 3200, true, [[300, "tada"]], (eye, t, p) => {
    p.w = 116;
    p.h = 96;
    p.lidIn = p.lidOut = 0.22;
    p.happy = 0.3;
    p.y = t < 0.3 ? 0 : -16;
  }),
  S("hot", "Too hot", "life", "#ff4a30", 4200, false, [[300, "pant"], [2000, "pant"]], (eye, t, p) => {
    const pant = abs(sin(PI2 * 2.2 * t));
    p.w = 120 + 4 * pant;
    p.h = 70 - 6 * pant;
    p.lidOut = 0.3;
    p.lidIn = 0.1;
    p.y = 8 + 6 * pant;
  }),
  S("relieved", "Relieved", "life", "#60e0ff", 3200, true, [[400, "phew"]], (eye, t, p) => {
    if (t < 0.3) openPose(p);
    else if (t < 0.6) {
      p.w = 116;
      p.h = 136;
      p.y = -8;
    } else {
      const d = smooth01((t - 0.6) / 0.8);
      p.w = 116 + 4 * d;
      p.h = 136 - 36 * d;
      p.happy = 0.35 * d;
      p.lidIn = p.lidOut = 0.15 * d;
      p.y = -8 + 14 * d;
    }
  }),
  S("tired", "Low battery", "life", "#ffa030", 4200, false, [[300, "drain"]], (eye, t, p) => {
    const d = smooth01((t - 0.3) / 2.6);
    p.w = 110 - 10 * d;
    p.h = 110 - 64 * d;
    p.lidIn = p.lidOut = 0.4 * d;
    p.y = 22 * d;
  }),
  S("charged", "Charging", "life", "#40e080", 3200, false, [[200, "charge"]], (eye, t, p) => {
    const d = smooth01((t - 0.2) / 1.4);
    p.w = 112;
    p.h = 40 + 94 * d;
    p.y = 30 - 36 * d;
    p.happy = t > 1.8 ? 0.32 : 0;
  }),
];

const SHAPE_K = 220;
const SHAPE_ZETA = 0.5;
const GAZE_K = 650;
const GAZE_ZETA = 0.85;
const SPRING_STEP_S = 0.01;
const BLINK_MS = 180;
const RADIUS_SHARE = 0.3;
const LCD = 240;
/** Between two runs of a scene in the gallery: shut, then opening. */
const PAUSE_MS = 900;
const KEYS = ["w", "h", "x", "y", "lidIn", "lidOut", "happy"] as const;
type Spring = { v: number; vel: number };

const hex = (c: string) => [1, 3, 5].map((i) => parseInt(c.slice(i, i + 2), 16));

/** rainbow_color() in ui_eyes.c. */
export function rainbow(now: number) {
  const h = ((now / 3000) % 1) * 6;
  const x = 1 - abs((h % 2) - 1);
  const pick = [
    [0, 1, -1],
    [1, 0, -1],
    [-1, 0, 1],
    [-1, 1, 0],
    [1, -1, 0],
    [0, -1, 1],
  ][Math.floor(h) % 6];
  const rgb = pick.map((w) => Math.floor(60 + 195 * (w === 0 ? 1 : w === 1 ? x : 0)));
  return `rgb(${rgb.join(",")})`;
}

const shut = (): Params => ({ w: 124, h: 6, x: 0, y: 0, lidIn: 0, lidOut: 0, happy: 0 });

/** One scene on a loop, both eyes. */
export class ScenePlayer {
  scene: Scene;
  #start = 0;
  #last = 0;
  #springs: Record<(typeof KEYS)[number], Spring>[];
  #nextBlink = 0;

  constructor(scene: Scene, now = performance.now()) {
    this.scene = scene;
    const p = shut();
    this.#springs = [0, 1].map(() => Object.fromEntries(KEYS.map((k) => [k, { v: p[k], vel: 0 }])) as Record<(typeof KEYS)[number], Spring>);
    this.restart(now);
  }

  restart(now: number) {
    this.#start = now;
    this.#last = now;
    this.#nextBlink = now + 900 + Math.random() * 1300;
  }

  /** Seconds into the scene; past its end while it's shut between runs. */
  t(now: number) {
    return (now - this.#start) / 1000;
  }

  get progress() {
    return min(1, (this.#last - this.#start) / this.scene.ms);
  }

  step(now: number) {
    const dt = min(0.08, max(0.001, (now - this.#last) / 1000));
    this.#last = now;
    const elapsed = now - this.#start;
    if (elapsed > this.scene.ms + PAUSE_MS) {
      this.restart(now);
    }
    const t = (now - this.#start) / 1000;
    const running = now - this.#start <= this.scene.ms;
    this.#springs.forEach((s, eye) => {
      const target = shut();
      if (running) {
        const p: Params = { w: 0, h: 0, x: 0, y: 0, lidIn: 0, lidOut: 0, happy: 0 };
        this.scene.pose(eye, t, p);
        Object.assign(target, p);
      }
      for (const k of KEYS) {
        const gaze = k === "x" || k === "y";
        const kk = gaze ? GAZE_K : SHAPE_K;
        const c = 2 * (gaze ? GAZE_ZETA : SHAPE_ZETA) * Math.sqrt(kk);
        const n = Math.ceil(dt / SPRING_STEP_S);
        for (let i = 0; i < n; i++) {
          s[k].vel += (kk * (target[k] - s[k].v) - c * s[k].vel) * (dt / n);
          s[k].v += s[k].vel * (dt / n);
        }
      }
    });
    if (this.scene.blinks && running && now >= this.#nextBlink + BLINK_MS) {
      this.#nextBlink = now + 2200 + Math.random() * 3300;
    }
  }

  #blink(now: number) {
    if (!this.scene.blinks) return 1;
    const t = now - this.#nextBlink;
    if (t < 0 || t > BLINK_MS) return 1;
    return 0.07 + 0.93 * abs(1 - (2 * t) / BLINK_MS);
  }

  color(now: number) {
    return this.scene.color === "rainbow" ? rainbow(now) : `rgb(${hex(this.scene.color).join(",")})`;
  }

  geom(eye: number, now: number): Geom {
    const p = this.#springs[eye];
    const w = max(p.w.v, 4);
    const h = max(p.h.v * this.#blink(now), 3);
    const cx = LCD / 2 + p.x.v;
    const cy = LCD / 2 + p.y.v;
    const lidIn = max(p.lidIn.v, 0) * h;
    const lidOut = max(p.lidOut.v, 0) * h;
    return {
      x0: cx - w / 2,
      x1: cx + w / 2,
      y0: cy - h / 2,
      y1: cy + h / 2,
      radius: min(min(w, h) * RADIUS_SHARE, min(w, h) / 2),
      lidLeft: eye === 0 ? lidOut : lidIn,
      lidRight: eye === 0 ? lidIn : lidOut,
      happyTop: p.happy.v > 0.02 ? cy + h / 2 - p.happy.v * h * 0.8 : 0,
    };
  }

  /** Both eyes side by side on `ctx`, each a round screen `size` px across. */
  draw(ctx: CanvasRenderingContext2D, size: number, gap: number, now: number) {
    const color = this.color(now);
    for (const eye of [0, 1]) {
      ctx.save();
      ctx.translate(eye * (size + gap), 0);
      ctx.beginPath();
      ctx.arc(size / 2, size / 2, size / 2, 0, PI2);
      ctx.clip();
      paintEye(ctx, this.geom(eye, now), color, size);
      ctx.restore();
    }
  }
}
