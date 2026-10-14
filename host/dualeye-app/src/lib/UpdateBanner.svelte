<script lang="ts">
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { monitor } from "./monitor.svelte";
  import { i18n } from "./i18n/index.svelte";

  let { onUpdate }: { onUpdate: () => void } = $props();

  // "Later" holds until the next app start, for the version it was offered.
  let dismissed = $state<string | null>(null);
  let expanded = $state(false);

  const update = $derived(monitor.update);
  const visible = $derived(!!update && dismissed !== update.to && monitor.job === "idle");
  const t = $derived(i18n.t.main.update);
  const from = $derived(update?.from ? `v${update.from}` : t.old);
</script>

<!-- Always present, so the app's grid rows don't shift when the banner comes and goes. -->
<div class="slot">
  {#if visible && update}
    <div class="banner" transition:slide={{ duration: 320, easing: cubicOut }} role="status">
      <div class="row">
        <span class="dot" aria-hidden="true"></span>
        <p>
          <b>{t.available(update.to)}</b>
          <span class="dim">{t.runs(from)}</span>
        </p>
        {#if update.changes.length}
          <button class="link" aria-expanded={expanded} onclick={() => (expanded = !expanded)}>
            {expanded ? t.hide : t.whatsNew}
          </button>
        {/if}
        <span class="grow"></span>
        <button class="btn" onclick={() => (dismissed = update.to)}>{t.later}</button>
        <button class="btn primary" onclick={onUpdate}>{t.update}</button>
      </div>
      {#if expanded}
        <div class="changes" transition:slide={{ duration: 240, easing: cubicOut }}>
          {#each update.changes as release (release.version)}
            <div class="release">
              <span class="ver">v{release.version}</span>
              <ul>
                {#each release.notes as note, i (i)}<li>{note}</li>{/each}
              </ul>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .slot {
    padding: 0 18px;
  }
  .banner {
    position: relative;
    z-index: 2;
    margin-top: 4px;
    border-radius: 12px;
    background: color-mix(in srgb, var(--cpu) 6%, #0d0e10);
    border: 1px solid color-mix(in srgb, var(--cpu) 22%, transparent);
    overflow: hidden;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 8px 8px 14px;
  }
  .dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--cpu);
    box-shadow: 0 0 8px var(--cpu);
  }
  p {
    margin: 0;
    font: 450 12.5px/1.4 var(--sans);
    color: var(--text);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  b {
    font-weight: 600;
  }
  .dim {
    color: var(--dim);
    margin-left: 4px;
  }
  .grow {
    flex: 1;
  }
  .link {
    flex: none;
    border: 0;
    background: none;
    padding: 0;
    color: var(--text);
    font: 450 12.5px/1 var(--sans);
    text-decoration: underline;
    text-decoration-color: rgba(255, 255, 255, 0.25);
    text-underline-offset: 3px;
    cursor: pointer;
  }
  .btn {
    flex: none;
    height: 28px;
    padding: 0 13px;
    border-radius: 8px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.05);
    color: var(--text);
    font: 550 12px/1 var(--sans);
    cursor: pointer;
    transition: background 200ms;
  }
  .btn:hover {
    background: rgba(255, 255, 255, 0.09);
  }
  .btn.primary {
    border-color: color-mix(in srgb, var(--cpu) 45%, transparent);
    background: color-mix(in srgb, var(--cpu) 14%, transparent);
  }
  .btn.primary:hover {
    background: color-mix(in srgb, var(--cpu) 22%, transparent);
  }
  .changes {
    border-top: 1px solid var(--line);
    padding: 10px 14px 12px 32px;
    display: grid;
    gap: 10px;
  }
  .release {
    display: grid;
    grid-template-columns: 52px 1fr;
    gap: 10px;
  }
  .ver {
    font: 500 11px/1.6 var(--mono);
    color: var(--cpu);
  }
  ul {
    margin: 0;
    padding-left: 16px;
    font: 400 12.5px/1.55 var(--sans);
    color: var(--dim);
  }
</style>
