// What the settings show, read in the background even with the panel closed:
// the tabs' status dots and the dependencies between settings need it all the
// time. An open tab reads its own part faster.

import {
  monitor,
  type ClaudeAlertSettings,
  type ClaudeAlertsInfo,
  type ClaudeLink,
  type McpInfo,
  type ModelInfo,
  type QuipsInfo,
  type TtsEngine,
  type VoiceInfo,
  type VoiceSettings,
} from "../monitor.svelte";

/** A service's key: `missing` without one, `checking` while the service is asked. */
export type KeyState = { state: "missing" | "checking" | "valid" | "invalid" | "unknown"; error: string | null };

const EVERY_MS = 5000;
/** Claude's status reads Claude Code's config, which can be a few MB: less often in the background. */
const CLAUDE_EVERY_MS = 30_000;

class SettingsData {
  voice = $state<VoiceInfo | null>(null);
  alerts = $state<ClaudeAlertsInfo | null>(null);
  link = $state<ClaudeLink | null>(null);
  mcp = $state<McpInfo | null>(null);
  quips = $state<QuipsInfo | null>(null);
  /** By provider id. */
  keys = $state<Record<string, KeyState>>({});
  /** Where each provider's key came from when it was last checked: a change checks it again. */
  #checked: Record<string, string | null> = {};
  #started = false;

  start() {
    if (this.#started) return;
    this.#started = true;
    const often = () => Promise.all([this.loadVoice(), this.loadQuips()]).catch(() => {});
    const claude = () => this.loadClaude().catch(() => {});
    often();
    claude();
    setInterval(often, EVERY_MS);
    setInterval(claude, CLAUDE_EVERY_MS);
  }

  async loadVoice() {
    this.#setVoice(await monitor.voiceInfo());
  }

  async loadClaude() {
    const [link, alerts, mcp] = await Promise.all([monitor.claudeLink(), monitor.claudeAlerts(), monitor.mcpInfo()]);
    this.link = link;
    this.alerts = alerts;
    this.mcp = mcp;
  }

  async loadQuips() {
    this.quips = await monitor.quipsInfo();
  }

  async setVoice(change: Partial<VoiceSettings>) {
    if (!this.voice) return;
    this.#setVoice(await monitor.setVoice({ ...this.voice.settings, ...change }));
  }

  async setAlerts(change: Partial<ClaudeAlertSettings>) {
    if (!this.alerts) return;
    this.alerts = await monitor.setClaudeAlerts({ ...this.alerts.settings, ...change });
  }

  /** Download a model; a cancel isn't an error. */
  async download(id: string) {
    try {
      await monitor.downloadModel(id);
    } catch (e) {
      if (!/cancel/i.test(String(e))) throw e;
    } finally {
      await this.loadVoice().catch(() => {});
    }
  }

  async installEngine(engine: TtsEngine) {
    try {
      await monitor.installEngine(engine);
    } finally {
      await this.loadVoice().catch(() => {});
    }
  }

  /** The downloads "use the recommended models" needs, with their size: the models, a voice for `lang`, and its engine. */
  localPlan(lang: string): { models: ModelInfo[]; engine: TtsEngine | null } {
    const v = this.voice;
    if (!v) return { models: [], engine: null };
    const r = v.recommendation;
    const voiceId = v.settings.voices[lang];
    const voice = v.models.find((m) => m.id === voiceId && !m.provider) ?? v.models.find((m) => m.kind === "voice" && !m.provider && m.language === lang);
    const ids = [r.whisper, r.llm, voice?.id];
    const models = v.models.filter((m) => ids.includes(m.id) && !m.installed);
    const engine = voice?.engine && !v[voice.engine] ? voice.engine : null;
    return { models, engine };
  }

  /** The assistant on this computer: the recommended models, downloaded where missing, and a voice for `lang`. */
  async useLocal(lang: string) {
    const v = this.voice;
    if (!v) return;
    const r = v.recommendation;
    const voiceId = v.settings.voices[lang];
    const voice = v.models.find((m) => m.id === voiceId && !m.provider) ?? v.models.find((m) => m.kind === "voice" && !m.provider && m.language === lang);
    const { models, engine } = this.localPlan(lang);
    await this.setVoice({
      enabled: true,
      speak: true,
      model: r.whisper,
      llm: !!r.llm,
      ...(r.llm ? { llm_model: r.llm } : {}),
      ...(voice ? { voices: { ...v.settings.voices, [lang]: voice.id } } : {}),
    });
    for (const m of models) await this.download(m.id);
    if (engine) await this.installEngine(engine);
  }

  /** The assistant on `provider`'s models, where it has them: hearing, understanding, a voice per language. */
  async useOnline(provider: string) {
    const v = this.voice;
    if (!v) return;
    const of = (kind: ModelInfo["kind"], lang?: string) => v.models.find((m) => m.provider === provider && m.kind === kind && (!lang || m.language === lang || m.language === null))?.id;
    const voices = { ...v.settings.voices };
    for (const lang of Object.keys(voices)) voices[lang] = of("voice", lang) ?? voices[lang];
    const hearing = of("whisper");
    const llm = of("llm");
    await this.setVoice({ enabled: true, speak: true, voices, ...(hearing ? { model: hearing } : {}), ...(llm ? { llm: true, llm_model: llm } : {}) });
  }

  /** Save a key (null forgets it): the backend checks it right away. */
  async saveKey(provider: string, key: string | null) {
    this.keys[provider] = { state: key ? "checking" : "missing", error: null };
    try {
      this.#setVoice(await monitor.setApiKey(provider, key), false);
      this.keys[provider] = { state: key ? "valid" : "missing", error: null };
    } catch (e) {
      this.keys[provider] = { state: "invalid", error: String(e) };
      // It's saved even when refused: read where it stands.
      this.#setVoice(await monitor.voiceInfo(), false);
    }
    this.#checked[provider] = this.voice?.providers.find((p) => p.id === provider)?.key ?? null;
  }

  async checkKey(provider: string) {
    this.keys[provider] = { state: "checking", error: null };
    try {
      const r = await monitor.checkApiKey(provider);
      this.keys[provider] = { state: r.state, error: r.error };
    } catch (e) {
      this.keys[provider] = { state: "unknown", error: String(e) };
    }
  }

  /** Check the keys that are new or changed since they were last checked. */
  #setVoice(v: VoiceInfo, check = true) {
    this.voice = v;
    if (!check) return;
    for (const p of v.providers) {
      if (p.id in this.#checked && this.#checked[p.id] === p.key) continue;
      this.#checked[p.id] = p.key;
      if (p.key) this.checkKey(p.id);
      else this.keys[p.id] = { state: "missing", error: null };
    }
  }
}

export const settingsData = new SettingsData();
