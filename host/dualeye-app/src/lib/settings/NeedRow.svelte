<!-- "⚠ Needs …" under a setting that depends on another, with the fix right here. -->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import Glyph from "./Glyph.svelte";
  import { settingsNav } from "./nav.svelte";
  import type { Need } from "./requirements";

  let { need }: { need: Need | null } = $props();
  const tc = $derived(i18n.t.settings.common);
  let busy = $state(false);
  let error = $state("");

  async function fix() {
    if (!need) return;
    const f = need.fix;
    if (f.kind === "configure") return settingsNav.go(f.to);
    busy = true;
    error = "";
    try {
      await f.run();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

{#if need}
  <div class="need" role="status">
    <Glyph state="warn" />
    <span class="needtext">{need.text}</span>
    <button class="btn small" class:primary={need.fix.kind === "enable"} disabled={busy} onclick={fix}>
      {need.fix.kind === "enable" ? tc.enable : `${tc.configure} →`}
    </button>
  </div>
{/if}
{#if error}<p class="hint error">{error}</p>{/if}
