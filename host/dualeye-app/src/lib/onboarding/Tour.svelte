<!--
  The tour: a light on one part of the real window at a time, a bubble with
  a sentence or two, Next / End tour, ← → and Esc. The steps are in tour.ts;
  one whose element isn't on screen (no pet yet) is left out.
-->
<script lang="ts">
  import { untrack } from "svelte";
  import { fade } from "svelte/transition";
  import { i18n } from "../i18n/index.svelte";
  import { onboarding } from "./state.svelte";
  import type { TourStep } from "./tour";

  let { steps: all }: { steps: TourStep[] } = $props();
  const t = $derived(i18n.t.onboarding.tour);

  const find = (s: TourStep) => (s.target ? document.querySelector<HTMLElement>(`[data-tour="${s.target}"]`) : null);
  // Read once: the list doesn't change under the person's feet.
  const steps = untrack(() => all.filter((s) => !s.target || find(s)));
  let index = $state(0);
  const step = $derived(steps[index]);
  const last = $derived(index === steps.length - 1);

  /** The lit part, with some room around it; null for a step in the middle of the window. */
  let rect = $state<{ x: number; y: number; w: number; h: number } | null>(null);
  let bubble = $state<HTMLElement>();
  let bubbleH = $state(0);
  let winW = $state(innerWidth);
  let winH = $state(innerHeight);
  const PAD = 8;

  function measure() {
    winW = innerWidth;
    winH = innerHeight;
    const el = step && find(step);
    if (!el) return (rect = null);
    const r = el.getBoundingClientRect();
    const x = Math.max(4, r.left - PAD);
    const y = Math.max(4, r.top - PAD);
    rect = { x, y, w: Math.min(winW - 4, r.right + PAD) - x, h: Math.min(winH - 4, r.bottom + PAD) - y };
  }

  // Follow the element: layouts move (the window resizes, the board's caption changes).
  $effect(() => {
    void index;
    measure();
    const id = setInterval(measure, 400);
    return () => clearInterval(id);
  });

  // The bubble takes the focus at each step, for the keyboard and screen readers.
  $effect(() => {
    void index;
    queueMicrotask(() => bubble?.focus({ preventScroll: true }));
  });

  /** Below the lit part if it fits, else above, else in the middle. */
  const W = 300;
  const place = $derived.by(() => {
    if (!rect) return { left: (winW - W) / 2, top: Math.max(16, (winH - bubbleH) / 2) };
    const left = Math.min(Math.max(12, rect.x + rect.w / 2 - W / 2), winW - W - 12);
    if (rect.y + rect.h + 14 + bubbleH < winH - 8) return { left, top: rect.y + rect.h + 14 };
    if (rect.y - 14 - bubbleH > 8) return { left, top: rect.y - 14 - bubbleH };
    return { left, top: Math.max(16, winH - bubbleH - 16) };
  });

  // Nothing to show (it shouldn't happen): done, rather than stuck.
  $effect(() => {
    if (!steps.length) onboarding.endTour();
  });

  const next = () => (last ? onboarding.endTour() : index++);
  const back = () => index > 0 && index--;

  function onKey(e: KeyboardEvent) {
    if (e.key === "Tab" && bubble) {
      // Tab goes round the bubble's buttons, not the window underneath.
      const buttons = [...bubble.querySelectorAll<HTMLElement>("button")];
      const at = buttons.indexOf(document.activeElement as HTMLElement);
      const to = at < 0 ? (e.shiftKey ? buttons.length - 1 : 0) : (at + (e.shiftKey ? -1 : 1) + buttons.length) % buttons.length;
      buttons[to]?.focus();
    } else if (e.key === "ArrowRight") next();
    else if (e.key === "ArrowLeft") back();
    else if (e.key === "Escape") onboarding.endTour();
    else return;
    e.preventDefault();
    e.stopImmediatePropagation();
  }
</script>

<svelte:window onkeydown={onKey} onresize={measure} />

{#if step}
  <div class="tour" transition:fade={{ duration: 200 }}>
    <!-- Catches the clicks: the window underneath is only to look at. -->
    <div class="shield" class:dim={!rect}></div>
    {#if rect}
      <div class="spot" style:left="{rect.x}px" style:top="{rect.y}px" style:width="{rect.w}px" style:height="{rect.h}px"></div>
    {/if}
    <div
      class="bubble"
      role="dialog"
      aria-modal="true"
      aria-labelledby="tour-title"
      aria-describedby="tour-text"
      tabindex="-1"
      bind:this={bubble}
      bind:clientHeight={bubbleH}
      style:left="{place.left}px"
      style:top="{place.top}px"
      style:width="{W}px"
    >
      <div class="bhead">
        <h2 id="tour-title">{t.steps[step.id].title}</h2>
        <span class="count">{t.count(index + 1, steps.length)}</span>
      </div>
      <p id="tour-text" aria-live="polite">{t.steps[step.id].text}</p>
      <div class="bfoot">
        <button class="tbtn ghost" onclick={() => onboarding.endTour()}>{t.end}</button>
        <span class="grow"></span>
        {#if index > 0}<button class="tbtn" onclick={back} aria-label={t.back}>←</button>{/if}
        <button class="tbtn primary" onclick={next}>{last ? t.finish : `${t.next} →`}</button>
      </div>
      <span class="keys">{t.keys}</span>
    </div>
  </div>
{/if}

<style>
  .tour {
    position: fixed;
    inset: 0;
    z-index: 70;
  }
  .shield {
    position: absolute;
    inset: 0;
    transition: background 300ms;
  }
  .shield.dim {
    background: rgba(0, 0, 0, 0.62);
  }
  .spot {
    position: absolute;
    border-radius: 16px;
    box-shadow:
      0 0 0 1.5px rgba(255, 255, 255, 0.35),
      0 0 0 9999px rgba(0, 0, 0, 0.62);
    pointer-events: none;
    transition:
      left 380ms cubic-bezier(0.3, 0.8, 0.2, 1),
      top 380ms cubic-bezier(0.3, 0.8, 0.2, 1),
      width 380ms cubic-bezier(0.3, 0.8, 0.2, 1),
      height 380ms cubic-bezier(0.3, 0.8, 0.2, 1);
  }
  .bubble {
    position: absolute;
    display: grid;
    gap: 10px;
    padding: 16px 16px 12px;
    border-radius: 16px;
    background: rgba(24, 25, 29, 0.97);
    border: 1px solid rgba(255, 255, 255, 0.1);
    box-shadow: 0 24px 60px -12px rgba(0, 0, 0, 0.8);
    outline: none;
    transition:
      left 380ms cubic-bezier(0.3, 0.8, 0.2, 1),
      top 380ms cubic-bezier(0.3, 0.8, 0.2, 1);
  }
  .bubble:focus-visible {
    box-shadow:
      0 0 0 2px rgba(255, 255, 255, 0.3),
      0 24px 60px -12px rgba(0, 0, 0, 0.8);
  }
  @media (prefers-reduced-motion: reduce) {
    .spot,
    .bubble {
      transition: none;
    }
  }
  .bhead {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  h2 {
    flex: 1;
    margin: 0;
    font: 650 15px/1.25 var(--sans);
  }
  .count {
    font: 500 10.5px/1 var(--mono);
    color: var(--faint);
  }
  p {
    margin: 0;
    font: 450 13px/1.5 var(--sans);
    color: var(--dim);
  }
  .bfoot {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 2px;
  }
  .grow {
    flex: 1;
  }
  .tbtn {
    height: 30px;
    padding: 0 12px;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    background: rgba(255, 255, 255, 0.06);
    color: var(--text);
    font: 550 12px/1 var(--sans);
    cursor: pointer;
  }
  .tbtn:hover {
    background: rgba(255, 255, 255, 0.1);
  }
  .tbtn.primary {
    background: var(--text);
    color: #0b0c0e;
    border-color: transparent;
  }
  .tbtn.primary:hover {
    background: #fff;
  }
  .tbtn.ghost {
    background: none;
    border-color: transparent;
    color: var(--dim);
    padding: 0 6px;
  }
  .tbtn:focus-visible {
    outline: 2px solid rgba(255, 255, 255, 0.5);
    outline-offset: 2px;
  }
  .keys {
    font: 450 10.5px/1 var(--sans);
    color: var(--faint);
  }
</style>
