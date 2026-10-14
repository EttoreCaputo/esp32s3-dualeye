// How the app shows the pet's mood (main/pet.c), wherever it does. The words
// for each are in i18n's `pet`.
import type { PetMood, PetNow } from "./monitor.svelte";

export const MOODS: Record<PetMood, { scene: string; color: string }> = {
  content: { scene: "look_around", color: "#30d5f0" },
  happy: { scene: "happy", color: "#40e080" },
  excited: { scene: "excited", color: "#ffd040" },
  loving: { scene: "love", color: "#ff5aa8" },
  bored: { scene: "bored", color: "#8aa4c0" },
  grumpy: { scene: "suspicious", color: "#ff7a50" },
  sad: { scene: "sad", color: "#4a7bff" },
  sleepy: { scene: "yawn", color: "#7a9cff" },
  hot: { scene: "hot", color: "#ff4a30" },
};

export const NOW_COLORS: Record<PetNow, string> = {
  hot: "#ff4a30",
  music: "#40e080",
  claude: "#d97757",
  battery_low: "#ffa030",
  charging: "#40e080",
  night: "#7a9cff",
  away: "#8aa4c0",
  bored: "#8aa4c0",
  quiet: "#7a9cff",
  chatter: "#30d5f0",
};

/** The idle scenes' gap at `pace` (ui_eyes.c: 30-120 s times it). */
export const scenesEvery = (pace: number): [number, number] => [Math.round(30 * pace), Math.round(120 * pace)];

export function clock(minute: number) {
  return `${String(Math.floor(minute / 60)).padStart(2, "0")}:${String(minute % 60).padStart(2, "0")}`;
}
