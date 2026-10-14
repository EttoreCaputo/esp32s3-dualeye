<!--
  An ⓘ next to a short label: the longer explanation shows while it's hovered
  or focused, and a click keeps it open. Fixed on the page, so a scrolling
  panel doesn't clip it.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { i18n } from "./i18n/index.svelte";

  let { text, children, label }: { text?: string; children?: Snippet; label?: string } = $props();

  const id = `tip-${Math.random().toString(36).slice(2, 8)}`;
  const W = 280;
  let button = $state<HTMLButtonElement>();
  let hovered = $state(false);
  let pinned = $state(false);
  let pos = $state({ x: 0, y: 0, above: false });
  const open = $derived(hovered || pinned);

  function place() {
    if (!button) return;
    const r = button.getBoundingClientRect();
    const above = r.bottom + 180 > innerHeight && r.top > 180;
    pos = { x: Math.min(Math.max(r.left + r.width / 2, W / 2 + 10), innerWidth - W / 2 - 10), y: above ? r.top - 8 : r.bottom + 8, above };
  }
  const show = () => (place(), (hovered = true));
  const hide = () => (hovered = false);
  function toggle(e: MouseEvent) {
    // Inside a <label>, the click mustn't flip its switch.
    e.preventDefault();
    e.stopPropagation();
    place();
    pinned = !pinned;
  }
  // It stays where it opened: any scroll (the panel's too) closes it.
  $effect(() => {
    if (!open) return;
    const close = () => (hovered = pinned = false);
    document.addEventListener("scroll", close, true);
    return () => document.removeEventListener("scroll", close, true);
  });
  /** On <body>: a panel with a backdrop-filter would otherwise be what `fixed` is relative to. */
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }
  function key(e: KeyboardEvent) {
    if (e.key === "Escape" && open) {
      e.stopPropagation();
      hovered = pinned = false;
    }
  }
</script>

<span class="info">
  <button
    bind:this={button}
    type="button"
    class:open
    aria-label={label ?? i18n.t.settings.common.moreInfo}
    aria-describedby={open ? id : undefined}
    aria-expanded={pinned}
    onmouseenter={show}
    onmouseleave={hide}
    onfocus={show}
    onblur={() => (hovered = pinned = false)}
    onclick={toggle}
    onkeydown={key}
  >
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <circle cx="8" cy="8" r="6.4" fill="none" stroke="currentColor" stroke-width="1.3" />
      <path d="M8 7.2v4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      <circle cx="8" cy="4.9" r="0.95" fill="currentColor" />
    </svg>
  </button>
  {#if open}
    <span use:portal role="tooltip" {id} class="pop" class:above={pos.above} style:left="{pos.x}px" style:top="{pos.y}px" style:--w="{W}px">
      {#if children}{@render children()}{:else}{text}{/if}
    </span>
  {/if}
</span>

<style>
  .info {
    display: inline-flex;
    vertical-align: middle;
  }
  button {
    display: inline-grid;
    place-items: center;
    width: 18px;
    height: 18px;
    margin: -3px 0 -3px 2px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: none;
    color: var(--faint);
    cursor: help;
    transition: color 150ms;
  }
  button:hover,
  button.open,
  button:focus-visible {
    color: var(--text);
  }
  button:focus-visible {
    outline: 1.5px solid color-mix(in srgb, var(--cpu) 70%, transparent);
  }
  svg {
    width: 14px;
    height: 14px;
  }
  .pop {
    position: fixed;
    z-index: 100;
    width: max-content;
    max-width: var(--w);
    padding: 10px 12px;
    transform: translateX(-50%);
    border-radius: 10px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    background: rgba(28, 29, 33, 0.97);
    box-shadow: 0 16px 40px -10px rgba(0, 0, 0, 0.7);
    color: var(--text);
    font: 400 12.5px/1.5 var(--sans);
    letter-spacing: 0;
    text-transform: none;
    text-align: left;
    white-space: normal;
    pointer-events: none;
    animation: in 140ms ease-out;
  }
  .pop.above {
    transform: translate(-50%, -100%);
  }
  .pop :global(code) {
    font: 500 11.5px var(--mono);
    color: var(--text);
  }
  @keyframes in {
    from {
      opacity: 0;
    }
  }
</style>
