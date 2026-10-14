<!--
  The voice assistant's status card: hearing, understanding, speaking, each
  with ✓/⚠ and a test. The Voice tab and the first-run setup show the same
  one; in the setup (`inSetup`) it doesn't send you to the settings.
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import { monitor, type SttStatus } from "../monitor.svelte";
  import { settingsData } from "./data.svelte";
  import Glyph, { type GlyphState } from "./Glyph.svelte";
  import { settingsNav } from "./nav.svelte";
  import { need, type Need } from "./requirements";

  let { inSetup = false }: { inSetup?: boolean } = $props();

  const tx = $derived(i18n.t.settings);
  const tv = $derived(tx.voice);
  const tc = $derived(tx.common);
  const voice = $derived(settingsData.voice);
  const connected = $derived(monitor.link === "connected");

  const GLYPH: Record<SttStatus, GlyphState> = { ready: "ok", starting: "busy", error: "error", off: "off" };
  /** The board says something in the language it hears, or the app's. */
  const testLang = $derived(voice && voice.settings.language !== "auto" ? voice.settings.language : i18n.lang === "it" ? "it" : "en");

  let testing = $state(false);
  let error = $state("");
  async function testVoice() {
    testing = true;
    error = "";
    try {
      await monitor.testVoice(testLang);
    } catch (e) {
      error = String(e);
    } finally {
      testing = false;
    }
  }

  /** "Say Alexa, then something": what the next utterance proves. */
  let listen = $state<{ part: "hearing" | "understanding"; since: number; result: { ok: boolean; text: string } | null } | null>(null);
  function startListen(part: "hearing" | "understanding") {
    const test = { part, since: Date.now(), result: null };
    listen = test;
    setTimeout(() => listen?.since === test.since && !listen.result && (listen.result = { ok: false, text: tv.tests.nothing }), 25_000);
  }
  $effect(() => {
    if (!listen || listen.result) return;
    const since = listen.since;
    const entry = [...monitor.transcripts].reverse().find((e) => e.at >= since);
    if (!entry) return;
    if (!entry.transcript) listen.result = { ok: false, text: tv.noWords };
    else if (listen.part === "hearing") listen.result = { ok: true, text: tv.tests.heard(entry.transcript.text) };
    else if (entry.reply) listen.result = entry.reply.by === "llm" ? { ok: true, text: tv.tests.answered(entry.reply.text) } : { ok: false, text: tv.tests.rules };
  });

  const ICONS = {
    hearing: "M4.8 6.3a3.4 3.4 0 1 1 6.8 0c0 1.8-1.3 2.4-1.9 3.3-.5.8-.6 2.6-2.2 3-1 .3-1.9-.3-2.2-1.1M7.2 6.4a1.1 1.1 0 0 1 2.1.4",
    understanding: "M8 2.5l1.3 3.2 3.2 1.3-3.2 1.3L8 11.5 6.7 8.3 3.5 7l3.2-1.3zM12.5 11l.6 1.4 1.4.6-1.4.6-.6 1.4-.6-1.4-1.4-.6 1.4-.6z",
    speaking: "M3 6.2h2.2L8.5 3.5v9L5.2 9.8H3zM11 5.5a3.5 3.5 0 0 1 0 5M12.8 3.8a6 6 0 0 1 0 8.4",
  };
</script>

<!-- The fix a status row offers when its part is off. -->
{#snippet fixButton(n: Need)}
  {#if n.fix.kind === "enable"}
    <button class="btn small primary" onclick={n.fix.run}>{tc.enable}</button>
  {:else if !inSetup}
    {@const to = n.fix.to}
    <button class="btn small" onclick={() => settingsNav.go(to)}>{tc.configure} →</button>
  {/if}
{/snippet}

{#if voice}
  {@const v = voice}
  {@const s = v.settings}
  {@const rows = [
    { id: "hearing" as const, name: tv.hearing, state: v.stt, fix: null },
    { id: "understanding" as const, name: tv.understanding, state: s.llm ? v.llm : ("off" as SttStatus), fix: need("llm") },
    { id: "speaking" as const, name: tv.speaking, state: s.speak ? v.tts : ("off" as SttStatus), fix: need("speak") },
  ]}
  <div class="statuscard" data-anchor="status">
    {#each rows as r (r.id)}
      <div class="srow">
        <svg class="sicon" viewBox="0 0 16 16" aria-hidden="true"><path d={ICONS[r.id]} /></svg>
        <span class="sname">{r.name}</span>
        <span class="sstate">{tv.status[r.state]}</span>
        <Glyph state={GLYPH[r.state]} label={tv.status[r.state]} />
        {#if r.state === "off" && r.fix}
          {@render fixButton(r.fix)}
        {:else if r.state === "error"}
          {#if !inSetup}
            {@const to = { tab: "voice" as const, id: r.id === "hearing" ? "model-hearing" : r.id === "understanding" ? "model-llm" : "voices" }}
            <button class="btn small" onclick={() => settingsNav.go(to)}>{tc.configure} →</button>
          {/if}
        {:else if r.id === "speaking"}
          <button class="btn small" class:primary={inSetup} disabled={v.tts !== "ready" || testing || !connected} title={connected ? tv.testTitle : tx.need.board} onclick={testVoice}
            >{testing ? tv.testing : tc.tryIt}</button
          >
        {:else}
          {@const part = r.id}
          <button class="btn small" disabled={r.state !== "ready" || !connected || (!!listen && !listen.result)} title={connected ? tv.tests.title : tx.need.board} onclick={() => startListen(part)}
            >{tc.tryIt}</button
          >
        {/if}
      </div>
    {/each}
    {#if listen}
      <div class="listen" class:ok={listen.result?.ok} class:bad={listen.result && !listen.result.ok} role="status">
        {#if listen.result}
          <Glyph state={listen.result.ok ? "ok" : "warn"} />
          <span>{listen.result.text}</span>
        {:else}
          <Glyph state="busy" />
          <span>{listen.part === "hearing" ? tv.tests.sayHearing : tv.tests.sayUnderstanding}</span>
        {/if}
        <button class="btn small" onclick={() => (listen = null)}>{tc.closeBtn}</button>
      </div>
    {/if}
    <span class="sboard" title={tv.boardStateTitle}>{tv.board(connected ? monitor.voice : tc.offline)}</span>
  </div>
  {#each [v.stt === "error" && v.stt_error, v.tts === "error" && v.tts_error, s.llm && v.llm === "error" && v.llm_error && tv.llmFallback(v.llm_error), error] as err, i (i)}
    {#if err}<p class="hint error">{err}</p>{/if}
  {/each}
  {#if v.tts_fallback}<p class="hint">{tv.ttsFallback(v.tts_fallback)}</p>{/if}
{/if}
