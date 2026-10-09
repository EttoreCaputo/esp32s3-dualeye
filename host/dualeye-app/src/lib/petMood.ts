// How the app shows the pet's mood (main/pet.c), wherever it does.
import type { PetMood, PetNow } from "./monitor.svelte";

export const MOODS: Record<PetMood, { label: string; says: string; scene: string; color: string }> = {
  content: { label: "Content", says: "All is well. Just looking around.", scene: "look_around", color: "#30d5f0" },
  happy: { label: "Happy", says: "In a good mood: smiles and songs.", scene: "happy", color: "#40e080" },
  excited: { label: "Excited", says: "Full of beans, ready to dance.", scene: "excited", color: "#ffd040" },
  loving: { label: "Fond of you", says: "You've been spending time together.", scene: "love", color: "#ff5aa8" },
  bored: { label: "Bored", says: "Nobody has played with it in a while.", scene: "bored", color: "#8aa4c0" },
  grumpy: { label: "Grumpy", says: "Things went wrong. Some attention would help.", scene: "suspicious", color: "#ff7a50" },
  sad: { label: "Sad", says: "A bad patch: talk to it, play some music.", scene: "sad", color: "#4a7bff" },
  sleepy: { label: "Sleepy", says: "Low on energy: it's late, or it's been a long day.", scene: "yawn", color: "#7a9cff" },
  hot: { label: "Too hot", says: "The computer is running hot.", scene: "hot", color: "#ff4a30" },
};

export const NOW: Record<PetNow, { label: string; color: string }> = {
  hot: { label: "Running hot", color: "#ff4a30" },
  music: { label: "Music playing", color: "#40e080" },
  claude: { label: "Claude working", color: "#d97757" },
  battery_low: { label: "Battery low", color: "#ffa030" },
  charging: { label: "Plugged in", color: "#40e080" },
  night: { label: "Night", color: "#7a9cff" },
  away: { label: "You're away", color: "#8aa4c0" },
  bored: { label: "Nothing to do", color: "#8aa4c0" },
};

/** What a reaction was to, as get_pet names it. */
export const REACTIONS: Record<string, string> = {
  hot: "It got too hot",
  cool: "Cooled down",
  music: "Music came on",
  claude_start: "Claude started",
  claude_done: "Claude finished",
  battery_low: "Battery low",
  charging: "Plugged in",
  goodnight: "Bedtime",
  greeting: "You came back",
  errors: "Things went wrong",
};

export function ago(s: number) {
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} d ago`;
}

export function duration(s: number) {
  if (s < 60) return `${Math.round(s)} s`;
  if (s < 3600) return `${Math.round(s / 60)} min`;
  return `${Math.floor(s / 3600)} h ${Math.round((s % 3600) / 60)} min`;
}

/** How the pitch sounds, 1 as written. */
export function voice(pitch: number) {
  if (pitch >= 1.08) return "bright and high";
  if (pitch >= 1.02) return "a little high";
  if (pitch > 0.98) return "as usual";
  if (pitch > 0.92) return "a little low";
  return "low and slow";
}

/** The idle scenes' gap at `pace` (ui_eyes.c: 30-120 s times it). */
export function scenesEvery(pace: number) {
  return `every ${Math.round(30 * pace)}–${Math.round(120 * pace)} s`;
}

export function clock(minute: number) {
  return `${String(Math.floor(minute / 60)).padStart(2, "0")}:${String(minute % 60).padStart(2, "0")}`;
}
