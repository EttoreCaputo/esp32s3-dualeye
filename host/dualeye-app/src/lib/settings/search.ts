// What the settings search finds: every setting by its label and its ⓘ text,
// in the app's language, and where it is.

import type { Messages } from "../i18n/index.svelte";
import type { Anchor } from "./nav.svelte";

type S = Messages["settings"];
export type Entry = Anchor & { label: (t: S) => string; tip?: (t: S) => string };

export const ENTRIES: Entry[] = [
  { tab: "voice", id: "assistant", label: (t) => t.voice.assistant, tip: (t) => t.voice.assistantTip },
  { tab: "voice", id: "status", label: (t) => t.voice.statusCard, tip: (t) => `${t.voice.hearing} ${t.voice.understanding} ${t.voice.speaking} ${t.common.tryIt}` },
  { tab: "voice", id: "volume", label: (t) => t.voice.volume, tip: (t) => t.voice.volumeAria },
  { tab: "voice", id: "language", label: (t) => t.voice.language, tip: (t) => t.voice.languageTip },
  { tab: "voice", id: "speak", label: (t) => t.voice.speak, tip: (t) => t.voice.speakTip },
  { tab: "voice", id: "llm", label: (t) => t.voice.llm, tip: (t) => t.voice.llmTip },
  { tab: "voice", id: "personality", label: (t) => t.voice.personality, tip: (t) => `${t.voice.personalityTip} ${Object.values(t.voice.personalities).flat().join(" ")}` },
  { tab: "voice", id: "services", label: (t) => t.voice.services, tip: (t) => t.voice.servicesTip },
  { tab: "voice", id: "key-groq", label: (t) => t.keys.apiKey("Groq"), tip: (t) => t.keys.tips.groq },
  { tab: "voice", id: "key-elevenlabs", label: (t) => t.keys.apiKey("ElevenLabs"), tip: (t) => t.keys.tips.elevenlabs },
  { tab: "voice", id: "eleven", label: (t) => t.voice.eleven, tip: (t) => t.voice.elevenTip },
  { tab: "voice", id: "models", label: (t) => t.voice.models, tip: (t) => t.voice.modelsSearch },
  { tab: "voice", id: "model-hearing", label: (t) => `${t.voice.models} · ${t.voice.hearing}`, tip: (t) => t.voice.hearingWhat },
  { tab: "voice", id: "model-llm", label: (t) => `${t.voice.models} · ${t.voice.understanding}`, tip: (t) => t.voice.understandingWhat },
  { tab: "voice", id: "voices", label: (t) => `${t.voice.models} · ${t.voice.speaking}`, tip: (t) => t.voice.voiceWhat },
  { tab: "voice", id: "follow-up", advanced: true, label: (t) => t.voice.followUp, tip: (t) => t.voice.followUpTip },
  { tab: "voice", id: "pause-music", advanced: true, label: (t) => t.voice.pauseMusic },
  { tab: "voice", id: "keep-warm", advanced: true, label: (t) => t.voice.keepWarm, tip: (t) => t.voice.keepWarmTip },
  { tab: "voice", id: "wake-sound", advanced: true, label: (t) => t.voice.wakeSound },
  { tab: "voice", id: "eyes", advanced: true, label: (t) => t.voice.eyes, tip: (t) => t.voice.eyesTip },
  { tab: "voice", id: "downloads", advanced: true, label: (t) => t.voice.downloads, tip: (t) => `${t.common.delete} Piper whisper llama` },
  { tab: "voice", id: "recordings", advanced: true, label: (t) => t.voice.keepRecordings },
  { tab: "voice", id: "transcripts", label: (t) => t.voice.transcripts },

  { tab: "pet", id: "pet-behaviour", label: (t) => t.pet.behaviour, tip: (t) => t.pet.behaviourTip },
  { tab: "pet", id: "idle-eyes", label: (t) => t.pet.idleEyes },
  { tab: "pet", id: "reactions", label: (t) => t.pet.reactions, tip: (t) => t.pet.reactionsTip },
  { tab: "pet", id: "pet-sounds", label: (t) => t.pet.sounds, tip: (t) => t.pet.soundsTip },
  { tab: "pet", id: "hearing", label: (t) => t.pet.hearing, tip: (t) => t.pet.hearingTip },
  { tab: "pet", id: "quips", label: (t) => t.pet.quips, tip: (t) => t.pet.quipsTip },

  { tab: "timers", id: "running", label: (t) => t.timers.running, tip: (t) => t.timers.tip },
  { tab: "timers", id: "new-timer", label: (t) => t.timers.new },
  { tab: "timers", id: "reminder", label: (t) => t.timers.reminder },
  { tab: "timers", id: "pomodoro", label: (t) => t.timers.pomodoro },
  { tab: "timers", id: "takes-over", label: (t) => t.timers.takesOver, tip: (t) => t.timers.takesOverTip },

  { tab: "claude", id: "connect", label: (t) => t.claude.connect, tip: (t) => `${t.claude.codeTip} ${t.claude.pieces.statusline} ${t.claude.pieces.hooks} ${t.claude.pieces.mcp} MCP` },
  { tab: "claude", id: "alerts", label: (t) => t.claude.alerts, tip: (t) => t.claude.alertsTip },
  { tab: "claude", id: "alert-needs", label: (t) => t.claude.needsYou },
  { tab: "claude", id: "alert-done", label: (t) => t.claude.done },
  { tab: "claude", id: "alert-usage", label: (t) => t.claude.usageAlert },
  { tab: "claude", id: "alert-speak", label: (t) => t.claude.speak },
  { tab: "claude", id: "alert-long", label: (t) => t.claude.longTask, tip: (t) => t.claude.longTaskTitle },
  { tab: "claude", id: "alert-language", label: (t) => t.claude.languageAria },
  { tab: "claude", id: "mcp", advanced: true, label: (t) => t.claude.mcp, tip: (t) => `${t.claude.mcpTip} Claude Desktop` },

  { tab: "device", id: "general", label: (t) => `${t.general.redo} · ${t.general.tour}`, tip: (t) => `${t.general.title} ${t.general.redoTip} ${t.general.tourTip}` },
  { tab: "device", id: "connection", label: (t) => t.device.connection, tip: (t) => `${t.device.connectionTip} USB WiFi` },
  { tab: "device", id: "wifi", label: (t) => t.device.wifi, tip: (t) => t.device.setupTip },
  { tab: "device", id: "firmware", label: (t) => t.device.firmware, tip: (t) => `${t.device.flashFirmware} ${t.device.flashNow}` },
  { tab: "device", id: "serial", advanced: true, label: (t) => t.device.serial, tip: (t) => t.device.serialTip },
  { tab: "device", id: "board", advanced: true, label: (t) => t.device.board, tip: (t) => `${t.device.boardTip} ${t.device.identify} esptool MAC` },

  { tab: "diagnostics", id: "sensors", label: (t) => t.diag.allReadings, tip: (t) => `${t.diag.sensors} ${t.diag.sensorsTip}` },
  { tab: "diagnostics", id: "power", label: (t) => t.diag.power, tip: (t) => t.diag.powerTip },
  { tab: "diagnostics", id: "console", label: (t) => t.diag.console },
  { tab: "diagnostics", id: "stream", label: (t) => t.diag.stream },
];

/** Lower case, no accents: "velocità" finds "velocita". */
const fold = (s: string) => s.toLowerCase().normalize("NFD").replace(/\p{M}/gu, "");

/** The entries with every word of `query`, those matching on the label first. */
export function search(query: string, t: S): Entry[] {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  const scored: [number, Entry][] = [];
  for (const e of ENTRIES) {
    const label = fold(e.label(t));
    const hay = `${label} ${fold(e.tip?.(t) ?? "")} ${fold(t.tabs[e.tab])}`;
    if (!words.every((w) => hay.includes(w))) continue;
    scored.push([words.filter((w) => label.includes(w)).length + (label.startsWith(words[0]) ? 1 : 0), e]);
  }
  return scored.sort((a, b) => b[0] - a[0]).map(([, e]) => e);
}
