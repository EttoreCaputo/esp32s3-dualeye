<!--
  The tour's invitation, for someone past the setup who hasn't seen this
  version of it: after an upgrade, or when a new version adds steps.
  "Not now" holds until the next start; ✕ declines this version.
-->
<script lang="ts">
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { i18n } from "../i18n/index.svelte";
  import { onboarding } from "./state.svelte";

  const t = $derived(i18n.t.onboarding.invite);
  const tc = $derived(i18n.t.settings.common);
</script>

{#if onboarding.invite}
  <div class="invite" role="status" transition:fly={{ y: 16, duration: 320, easing: cubicOut }}>
    <svg class="spark" viewBox="0 0 16 16" aria-hidden="true"><path d="M8 1.8l1.5 3.7 3.7 1.5-3.7 1.5L8 12.2 6.5 8.5 2.8 7l3.7-1.5z" /></svg>
    <span class="text">{onboarding.tourVersion > 0 ? t.textNew : t.text}</span>
    <button class="ibtn" onclick={() => (onboarding.later = true)}>{t.later}</button>
    <button class="ibtn primary" onclick={() => onboarding.startTour(true)}>{t.go}</button>
    <button class="ibtn close" onclick={() => onboarding.declineTour()} aria-label={tc.close} title={tc.close}>
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7" /></svg>
    </button>
  </div>
{/if}

<style>
  .invite {
    position: fixed;
    z-index: 30;
    left: 50%;
    bottom: 18px;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 8px;
    max-width: calc(100vw - 32px);
    padding: 8px 8px 8px 14px;
    border-radius: 14px;
    background: rgba(24, 25, 29, 0.95);
    backdrop-filter: blur(18px);
    border: 1px solid rgba(255, 255, 255, 0.1);
    box-shadow: 0 18px 40px -12px rgba(0, 0, 0, 0.8);
  }
  .spark {
    flex: none;
    width: 15px;
    height: 15px;
    fill: var(--cpu);
  }
  .text {
    font: 500 12.5px/1.35 var(--sans);
    margin-right: 4px;
  }
  .ibtn {
    flex: none;
    height: 28px;
    padding: 0 11px;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    background: rgba(255, 255, 255, 0.05);
    color: var(--dim);
    font: 550 11.5px/1 var(--sans);
    cursor: pointer;
  }
  .ibtn:hover {
    color: var(--text);
  }
  .ibtn.primary {
    background: var(--text);
    color: #0b0c0e;
    border-color: transparent;
  }
  .ibtn.close {
    display: grid;
    place-items: center;
    width: 28px;
    padding: 0;
    border-color: transparent;
    background: none;
  }
  .ibtn.close svg {
    width: 13px;
    height: 13px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
  }
</style>
