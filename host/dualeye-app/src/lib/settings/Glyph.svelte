<!-- ✓ works, ⚠ something's missing, ✗ fails, — off, … being checked. -->
<script module lang="ts">
  export type GlyphState = "ok" | "warn" | "error" | "off" | "busy";
</script>

<script lang="ts">
  let { state, label }: { state: GlyphState; label?: string } = $props();
</script>

<span class="glyph {state}" role="img" aria-label={label} title={label}>
  {#if state === "ok"}
    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.2l3 3 6-6.6" /></svg>
  {:else if state === "warn"}
    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 2.2 14.2 13H1.8z" /><path d="M8 6.5v3M8 11.4v.1" /></svg>
  {:else if state === "error"}
    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7" /></svg>
  {:else if state === "busy"}
    <svg class="spin" viewBox="0 0 16 16" aria-hidden="true"><path d="M8 2.5a5.5 5.5 0 1 1-5.5 5.5" /></svg>
  {:else}
    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 8h7" /></svg>
  {/if}
</span>
