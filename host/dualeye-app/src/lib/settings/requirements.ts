// What a setting needs from another one, in one place: each tab shows an unmet
// need as "⚠ Needs …" with a button that fixes it where you are ("Turn on"),
// or takes you to the setting when it takes more than a click ("Set up").
// The status dots on the tabs come from here too.

import { i18n } from "../i18n/index.svelte";
import { isClaudeFace } from "../firmware";
import { monitor, type VoiceInfo } from "../monitor.svelte";
import { settingsData as d } from "./data.svelte";
import type { Anchor, Tab } from "./nav.svelte";

export type Fix = { kind: "enable"; run: () => Promise<unknown> } | { kind: "configure"; to: Anchor };
export type Need = { text: string; fix: Fix };
export type Requirement = "speak" | "llm" | "statusline" | "hooks" | "board" | `key:${string}`;

const providerName = (id: string) => d.voice?.providers.find((p) => p.id === id)?.name ?? id;
const keyWorks = (provider: string) => ["valid", "unknown", "checking"].includes(d.keys[provider]?.state ?? "missing");

/** A model that can run now: downloaded with its engine, or online with a key that works. */
export function usable(v: VoiceInfo, id: string): boolean {
  const m = v.models.find((x) => x.id === id);
  // The ElevenLabs account's own voices aren't in the catalog.
  const provider = m ? m.provider : id.startsWith("elevenlabs:") ? "elevenlabs" : null;
  if (provider) return v.providers.some((p) => p.id === provider && p.key) && keyWorks(provider);
  return !!m?.installed && (!m.engine || !!v[m.engine]);
}

const voiceUsable = (v: VoiceInfo) => Object.values(v.settings.voices).some((id) => id && usable(v, id));

/** What `r` still needs, or null when it's met (or not known yet). */
export function need(r: Requirement): Need | null {
  const t = i18n.t.settings.need;
  const v = d.voice;
  if (r.startsWith("key:")) {
    const id = r.slice(4);
    return keyWorks(id) ? null : { text: t.key(providerName(id)), fix: { kind: "configure", to: { tab: "voice", id: `key-${id}` } } };
  }
  switch (r) {
    case "speak": {
      if (!v) return null;
      const s = v.settings;
      if (s.enabled && s.speak && v.tts !== "error") return null;
      return {
        text: t.speak,
        fix:
          voiceUsable(v) && v.tts !== "error"
            ? { kind: "enable", run: () => d.setVoice({ enabled: true, speak: true }) }
            : { kind: "configure", to: { tab: "voice", id: "voices" } },
      };
    }
    case "llm": {
      if (!v || (v.settings.llm && v.llm !== "error")) return null;
      return {
        text: t.llm,
        fix:
          usable(v, v.settings.llm_model) && v.llm !== "error"
            ? { kind: "enable", run: () => d.setVoice({ llm: true }) }
            : { kind: "configure", to: { tab: "voice", id: "model-llm" } },
      };
    }
    case "statusline":
      if (!d.link || d.link.connected) return null;
      return { text: t.statusline, fix: { kind: "enable", run: () => monitor.claudeConnect(true).then(() => d.loadClaude()) } };
    case "hooks":
      if (!d.alerts || d.alerts.hooks.connected) return null;
      return { text: t.hooks, fix: { kind: "enable", run: () => monitor.claudeHooks(true).then(() => d.loadClaude()) } };
    case "board":
      if (monitor.link === "connected") return null;
      return { text: t.board, fix: { kind: "configure", to: { tab: "device", id: "connection" } } };
  }
  return null;
}

/** Something on a tab that needs a look: `warn` when a feature that's on is missing something, `error` when something fails. */
export type Issue = { level: "warn" | "error"; text: string; to: Anchor };

export function issues(tab: Tab): Issue[] {
  const t = i18n.t.settings.issues;
  const out: Issue[] = [];
  const add = (level: Issue["level"], text: string, to: Anchor) => out.push({ level, text, to });
  const v = d.voice;
  switch (tab) {
    case "voice": {
      if (!v) break;
      for (const p of v.providers) if (d.keys[p.id]?.state === "invalid") add("error", t.keyRefused(p.name), { tab, id: `key-${p.id}` });
      const s = v.settings;
      if (!s.enabled) break;
      const parts = i18n.t.settings.voice;
      if (v.stt === "error") add("error", t.notWorking(parts.hearing), { tab, id: "status" });
      if (s.llm && v.llm === "error") add("error", t.notWorking(parts.understanding), { tab, id: "status" });
      if (s.speak && v.tts === "error") add("error", t.notWorking(parts.speaking), { tab, id: "status" });
      if (!usable(v, s.model)) add("warn", t.missing(parts.hearing), { tab, id: "model-hearing" });
      if (s.llm && !usable(v, s.llm_model)) add("warn", t.missing(parts.understanding), { tab, id: "model-llm" });
      if (s.speak && !voiceUsable(v)) add("warn", t.missing(parts.speaking), { tab, id: "voices" });
      break;
    }
    case "pet":
      if (d.quips && d.quips.level !== "off" && need("speak")) add("warn", t.quipsSilent, { tab, id: "quips" });
      break;
    case "claude": {
      const a = d.alerts;
      if (d.mcp?.hub_error) add("error", t.hub, { tab, id: "connect" });
      if (!a || !d.link || !d.mcp) break;
      const pieces = [d.link.connected, a.hooks.connected, d.mcp.registered];
      const usesClaude = isClaudeFace(monitor.faces.cpu) || isClaudeFace(monitor.faces.gpu);
      if ((usesClaude || pieces.some(Boolean)) && !pieces.every(Boolean)) add("warn", t.claudePartly, { tab, id: "connect" });
      if (a.hooks.connected && a.settings.speak && need("speak")) add("warn", t.alertsSilent, { tab, id: "alert-speak" });
      break;
    }
    case "device":
      // Not while it's still being looked for, at start.
      if (monitor.link === "offline") add("error", t.offline, { tab, id: "connection" });
      else if (monitor.update) add("error", t.firmware(monitor.update.to), { tab, id: "firmware" });
      break;
  }
  return out;
}

/** The worst of a tab's issues. */
export const level = (list: Issue[]): Issue["level"] | null => (list.some((i) => i.level === "error") ? "error" : list.length ? "warn" : null);
