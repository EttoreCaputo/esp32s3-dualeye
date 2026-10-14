<!--
  An online service's API key, the same for every service: hidden with a
  "show" button, "paste", "Get a key" (the service's page), and a check as
  soon as it's pasted. Its state is always in view: ✓ works, ✗ refused (why
  behind the ⓘ), — none, … being checked.
-->
<script lang="ts">
  import InfoTip from "../InfoTip.svelte";
  import { i18n } from "../i18n/index.svelte";
  import { monitor, type ProviderInfo } from "../monitor.svelte";
  import { settingsData } from "./data.svelte";
  import Glyph, { type GlyphState } from "./Glyph.svelte";

  let { provider: p }: { provider: ProviderInfo } = $props();
  const tk = $derived(i18n.t.settings.keys);
  const k = $derived(settingsData.keys[p.id] ?? { state: p.key ? "checking" : "missing", error: null });
  const GLYPH: Record<string, GlyphState> = { valid: "ok", invalid: "error", unknown: "warn", checking: "busy", missing: "off" };

  let draft = $state("");
  let shown = $state(false);
  let input = $state<HTMLInputElement>();

  async function save() {
    const key = draft.trim();
    if (!key) return;
    await settingsData.saveKey(p.id, key);
    if (settingsData.keys[p.id]?.state === "valid") {
      draft = "";
      shown = false;
    }
  }

  async function paste() {
    try {
      draft = (await navigator.clipboard.readText()).trim();
      if (draft) await save();
    } catch {
      // No clipboard access: paste by hand.
      input?.focus();
    }
  }
</script>

<div class="keyfield" data-anchor="key-{p.id}">
  <div class="khead">
    <span class="kname">{p.name}&nbsp;<InfoTip text={tk.tips[p.id] ?? p.note} /></span>
    <span class="kstate {k.state}">
      <Glyph state={GLYPH[k.state]} />
      {tk.state[k.state]}
      {#if k.error && (k.state === "invalid" || k.state === "unknown")}<InfoTip text={k.error} label={tk.why} />{/if}
    </span>
  </div>
  {#if p.key === "env"}
    <p class="hint small">{tk.env(p.key_env)}</p>
  {:else}
    <div class="krow">
      <input
        bind:this={input}
        class="kinput"
        type={shown ? "text" : "password"}
        autocomplete="off"
        spellcheck="false"
        placeholder={p.key ? tk.saved : tk.placeholder}
        aria-label={tk.aria(p.name)}
        bind:value={draft}
        onpaste={() => setTimeout(save)}
        onkeydown={(e) => e.key === "Enter" && save()}
      />
      <button class="kicon" type="button" title={shown ? tk.hide : tk.show} aria-label={shown ? tk.hide : tk.show} aria-pressed={shown} onclick={() => (shown = !shown)}>
        <svg viewBox="0 0 16 16" aria-hidden="true"
          ><path d="M1.5 8s2.4-4.5 6.5-4.5S14.5 8 14.5 8 12.1 12.5 8 12.5 1.5 8 1.5 8z" /><circle cx="8" cy="8" r="2" />{#if shown}<path d="M2.5 2.5l11 11" />{/if}</svg
        >
      </button>
      <button class="kicon" type="button" title={tk.paste} aria-label={tk.paste} disabled={k.state === "checking"} onclick={paste}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><rect x="3.5" y="3" width="9" height="11" rx="1.5" /><path d="M6 3V2h4v1M6 7h4M6 10h3" /></svg>
      </button>
    </div>
    <div class="kactions">
      <button class="btn small" type="button" onclick={() => monitor.openKeyPage(p.id)}>{tk.get} ↗</button>
      {#if draft.trim()}
        <button class="btn small primary" type="button" disabled={k.state === "checking"} onclick={save}>{tk.check}</button>
      {/if}
      {#if p.key === "saved"}
        <button class="btn small" type="button" disabled={k.state === "checking"} onclick={() => settingsData.saveKey(p.id, null)}>{tk.forget}</button>
      {/if}
    </div>
  {/if}
</div>
