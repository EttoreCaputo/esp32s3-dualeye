<!--
  Setup, step 4 (optional): the voice assistant, on this computer (the
  models recommended for it, downloaded once) or online (an API key, checked
  as soon as it's pasted). It ends with Try it: the board says something.
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import { monitor } from "../monitor.svelte";
  import { settingsData } from "../settings/data.svelte";
  import Glyph from "../settings/Glyph.svelte";
  import KeyField from "../settings/KeyField.svelte";
  import VoiceStatus from "../settings/VoiceStatus.svelte";

  const t = $derived(i18n.t.onboarding.voice);
  const voice = $derived(settingsData.voice);

  // Often, for the download's progress.
  $effect(() => {
    const load = () => settingsData.loadVoice().catch(() => {});
    load();
    const id = setInterval(load, 700);
    return () => clearInterval(id);
  });

  /** The voice speaks the app's language. */
  const lang = $derived(i18n.lang === "it" ? "it" : "en");
  let mode = $state<"local" | "online" | null>(null);
  let busy = $state(false);
  let error = $state("");
  /** Turned on here: show where it stands and the test. */
  let chosen = $state(false);
  const on = $derived(!!voice?.settings.enabled);
  const showStatus = $derived(on && (chosen || mode === null));

  const plan = $derived(voice ? settingsData.localPlan(lang) : { models: [], engine: null });
  const bytes = $derived(plan.models.reduce((n, m) => n + m.bytes, 0));
  const size = (n: number) => (n >= 1e9 ? `${i18n.number(n / 1e9, { maximumFractionDigits: 1 })} GB` : `${Math.round(n / 1_000_000)} MB`);

  async function run(job: () => Promise<unknown>) {
    busy = true;
    error = "";
    try {
      await job();
      chosen = true;
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  const useLocal = () => run(() => settingsData.useLocal(lang));

  // Online: the first service with models for every part (Groq), its key checked as it's pasted.
  const provider = $derived(voice?.providers.find((p) => p.id === "groq") ?? voice?.providers[0] ?? null);
  const keyState = $derived(provider ? settingsData.keys[provider.id]?.state : undefined);
  let applied = false;
  $effect(() => {
    if (mode !== "online" || !provider || keyState !== "valid" || applied) return;
    applied = true;
    const id = provider.id;
    void run(() => settingsData.useOnline(id));
  });

  const downloading = $derived(voice?.download ? voice.models.find((m) => m.id === voice.download?.[0]) : null);
</script>

<p class="lead">{t.lead}</p>

{#if !voice}
  <p class="empty">{i18n.t.settings.common.loading}</p>
{:else if showStatus}
  <div class="ready">
    <Glyph state="ok" />
    <span>{t.ready}</span>
  </div>
  <section class="voice">
    <VoiceStatus inSetup />
  </section>
  <p class="hint">{monitor.link === "connected" ? t.tryTip : t.needsBoard}</p>
  <div class="actions">
    <button class="btn small" onclick={() => ((chosen = false), (mode = mode ?? "local"))}>{t.change}</button>
  </div>
{:else}
  <div class="choices" role="radiogroup" aria-label={t.title}>
    <button class="choice" class:checked={mode === "local"} role="radio" aria-checked={mode === "local"} onclick={() => (mode = "local")}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4.5" width="18" height="12" rx="2" /><path d="M8 20h8M12 16.5V20" /></svg>
      <b>{t.local}</b>
      <small>{t.localNote}</small>
    </button>
    <button class="choice" class:checked={mode === "online"} role="radio" aria-checked={mode === "online"} onclick={() => (mode = "online")}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 18.5h10.5a4 4 0 0 0 .6-8A6 6 0 0 0 6.6 9.2 4.7 4.7 0 0 0 7 18.5z" /></svg>
      <b>{t.online}</b>
      <small>{t.onlineNote}</small>
    </button>
  </div>

  {#if mode === "local"}
    {#if downloading && voice.download}
      <div class="progress"><span style:width="{voice.download[1]}%"></span></div>
      <p class="hint">{t.downloading(downloading.id, Math.round(voice.download[1]))}</p>
    {:else if voice.piper_install}
      <div class="progress indeterminate"><span style:width="30%"></span></div>
      <p class="hint">{t.installing}</p>
    {:else}
      <p class="hint">{voice.recommendation.why}</p>
      <div class="actions">
        <button class="btn primary big" disabled={busy} onclick={useLocal}>{bytes ? t.download(size(bytes)) : t.turnOn}</button>
        {#if !bytes && !plan.engine}<span class="hint">{t.nothingToDownload}</span>{/if}
      </div>
    {/if}
  {:else if mode === "online" && provider}
    <p class="hint">{t.keyFirst}</p>
    <section class="voice">
      <KeyField {provider} />
    </section>
    {#if keyState === "valid" && !busy && !on}
      <div class="actions">
        <button class="btn primary" onclick={() => run(() => settingsData.useOnline(provider.id))}>{t.use}</button>
      </div>
    {/if}
  {/if}
{/if}
{#if error}<p class="hint error">{error}</p>{/if}

<style>
  .choices {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  @container (max-width: 460px) {
    .choices {
      grid-template-columns: 1fr;
    }
  }
  .choice {
    display: grid;
    justify-items: start;
    align-content: start;
    gap: 6px;
    padding: 14px;
    border-radius: 12px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.03);
    color: var(--text);
    text-align: left;
    cursor: pointer;
    transition:
      border-color 160ms,
      background 160ms;
  }
  .choice:hover {
    background: rgba(255, 255, 255, 0.05);
  }
  .choice.checked {
    border-color: color-mix(in srgb, var(--cpu) 55%, transparent);
    background: color-mix(in srgb, var(--cpu) 6%, transparent);
  }
  .choice svg {
    width: 22px;
    height: 22px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
    color: var(--dim);
  }
  .choice b {
    font: 620 14px/1.2 var(--sans);
  }
  .choice small {
    font: 450 12px/1.45 var(--sans);
    color: var(--dim);
  }
  .ready {
    display: flex;
    align-items: center;
    gap: 10px;
    font: 550 14px/1.4 var(--sans);
  }
  .actions {
    align-items: center;
  }
</style>
