<!--
  The first-run setup: full window, one screen per step, a progress bar.
  Every step has Back and Skip; the voice and Claude steps use the same
  components as the settings, so both always give the same result.
-->
<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { i18n, LANGUAGES, type Lang } from "../i18n/index.svelte";
  import { monitor } from "../monitor.svelte";
  import "../settings/settings.css";
  import { settingsData } from "../settings/data.svelte";
  import Glyph from "../settings/Glyph.svelte";
  import { onboarding } from "./state.svelte";
  import StepClaude from "./StepClaude.svelte";
  import StepConnect from "./StepConnect.svelte";
  import StepFirmware from "./StepFirmware.svelte";
  import StepVoice from "./StepVoice.svelte";

  const STEPS = ["welcome", "connect", "firmware", "voice", "claude", "done"] as const;
  type Step = (typeof STEPS)[number];

  const t = $derived(i18n.t.onboarding);
  let index = $state(0);
  /** +1 going ahead, -1 back: the screens slide that way, and only going ahead skips a step that's already fine. */
  let dir = $state(1);
  const step = $derived<Step>(STEPS[index]);
  let heading = $state<HTMLElement>();

  function go(to: number) {
    dir = to > index ? 1 : -1;
    index = Math.max(0, Math.min(STEPS.length - 1, to));
  }
  const next = () => go(index + 1);
  const back = () => go(index - 1);

  // Each new step's title takes the focus, so a screen reader reads it.
  $effect(() => {
    void index;
    queueMicrotask(() => heading?.focus({ preventScroll: true }));
  });

  // What the summary shows.
  const fw = $derived(monitor.boardFirmware);
  const summary = $derived.by(() => {
    const v = settingsData.voice;
    const link = settingsData.link;
    const alerts = settingsData.alerts;
    const mcp = settingsData.mcp;
    return [
      { id: "language" as const, ok: true, note: LANGUAGES.find((l) => l.id === i18n.lang)?.name ?? "" },
      { id: "connect" as const, ok: monitor.link === "connected", note: monitor.port ?? "" },
      { id: "firmware" as const, ok: monitor.link === "connected" && fw?.state === "version" && !monitor.update, note: fw?.state === "version" ? `v${fw.version}` : "" },
      { id: "voice" as const, ok: !!v?.settings.enabled, note: "" },
      { id: "claude" as const, ok: !!(link?.connected && alerts?.hooks.connected && mcp?.registered), note: "" },
    ];
  });

  function finish(tour: boolean) {
    onboarding.finishWizard();
    if (tour) onboarding.startTour();
    else onboarding.declineTour();
  }
</script>

<div class="wizard settings-ui" role="dialog" aria-modal="true" aria-label={t.wizard.title} transition:fade={{ duration: 220 }}>
  <div class="wback" aria-hidden="true"></div>
  <header class="whead">
    <svg class="wlogo" viewBox="0 0 40 20" aria-hidden="true">
      <circle cx="9" cy="10" r="8" fill="none" stroke="var(--cpu)" stroke-width="2.4" stroke-dasharray="36 60" transform="rotate(-90 9 10)" stroke-linecap="round" />
      <circle cx="31" cy="10" r="8" fill="none" stroke="var(--gpu)" stroke-width="2.4" stroke-dasharray="24 60" transform="rotate(-90 31 10)" stroke-linecap="round" />
    </svg>
    <span class="wtitle">{t.wizard.title}</span>
    <span class="wcount">{t.wizard.stepOf(index + 1, STEPS.length)}</span>
    <button class="dclose" onclick={() => finish(false)} aria-label={t.wizard.exit} title={t.wizard.exitTitle}>
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" /></svg>
    </button>
  </header>

  <ol class="wprogress" aria-label={t.wizard.title}>
    {#each STEPS as s, i (s)}
      <li class:done={i < index} class:current={i === index} aria-current={i === index ? "step" : undefined}>
        <span class="bar"></span>
        <span class="wlabel">{t.wizard.steps[s]}</span>
      </li>
    {/each}
  </ol>

  <main class="wbody">
    {#key index}
      <div class="wstep" in:fly={{ x: 28 * dir, duration: 320, easing: cubicOut, delay: 60 }} out:fade={{ duration: 80 }}>
        {#if step === "welcome"}
          <div class="hello">
            <svg class="eyes" viewBox="0 0 120 60" aria-hidden="true">
              <circle cx="30" cy="30" r="26" class="lcd cpu" />
              <circle cx="90" cy="30" r="26" class="lcd gpu" />
              <circle cx="34" cy="28" r="7" class="pupil" />
              <circle cx="94" cy="28" r="7" class="pupil" />
            </svg>
            <h1 tabindex="-1" bind:this={heading}>{t.welcome.title}</h1>
            <p class="lead">{t.welcome.lead}</p>
            <p class="hint">{t.welcome.time}</p>
          </div>
          <div class="wfield">
            <span class="rlabel">
              <svg viewBox="0 0 16 16" aria-hidden="true" class="globe"
                ><circle cx="8" cy="8" r="6.3" /><path d="M1.8 8h12.4M8 1.7c-1.9 1.8-2.8 3.9-2.8 6.3s.9 4.5 2.8 6.3M8 1.7c1.9 1.8 2.8 3.9 2.8 6.3s-.9 4.5-2.8 6.3" /></svg
              >
              {t.welcome.language}
            </span>
            <div class="rots two" role="radiogroup" aria-label={t.welcome.language}>
              {#each LANGUAGES as l (l.id)}
                <button class="rot" class:checked={i18n.lang === l.id} role="radio" aria-checked={i18n.lang === l.id} onclick={() => i18n.set(l.id as Lang)}>{l.name}</button>
              {/each}
            </div>
          </div>
        {:else if step === "done"}
          <h1 tabindex="-1" bind:this={heading}>{t.done.title}</h1>
          <p class="lead">{t.done.lead}</p>
          <div class="statuscard summary">
            {#each summary as r (r.id)}
              <div class="srow">
                <Glyph state={r.ok ? "ok" : "off"} />
                <span class="sname">{t.done.rows[r.id]}</span>
                <span class="sstate">{r.ok ? r.note : t.done.skipped}</span>
              </div>
            {/each}
          </div>
          <div class="tourbox">
            <button class="btn primary big" onclick={() => finish(true)}>{t.done.tour}</button>
            <span class="hint">{t.done.tourNote}</span>
          </div>
        {:else}
          <h1 tabindex="-1" bind:this={heading}>{t[step].title}</h1>
          {#if step === "connect"}
            <StepConnect />
          {:else if step === "firmware"}
            <StepFirmware forward={dir > 0} onnext={next} />
          {:else if step === "voice"}
            <StepVoice />
          {:else if step === "claude"}
            <StepClaude />
          {/if}
        {/if}
      </div>
    {/key}
  </main>

  <footer class="wfoot">
    {#if index > 0}
      <button class="btn" onclick={back}>← {t.wizard.back}</button>
    {/if}
    <span class="grow"></span>
    {#if step === "welcome"}
      <button class="btn primary big" onclick={next}>{t.welcome.start} →</button>
    {:else if step === "done"}
      <button class="btn big" onclick={() => finish(false)}>{t.done.finish}</button>
    {:else}
      <button class="btn ghost" onclick={next}>{t.wizard.skip}</button>
      <button class="btn primary big" onclick={next}>{t.wizard.next} →</button>
    {/if}
  </footer>
</div>

<style>
  .wizard {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: grid;
    grid-template-rows: auto auto minmax(0, 1fr) auto;
    background: var(--bg);
    container-type: inline-size;
  }
  .wback {
    position: absolute;
    inset: 0;
    pointer-events: none;
    background:
      radial-gradient(50% 45% at 50% 30%, rgba(196, 240, 106, 0.05), transparent 70%),
      radial-gradient(40% 40% at 70% 80%, rgba(200, 108, 240, 0.05), transparent 70%);
  }
  .whead {
    position: relative;
    display: flex;
    align-items: center;
    gap: 10px;
    height: 52px;
    padding: 0 12px 0 18px;
  }
  .wlogo {
    width: 32px;
    height: 16px;
  }
  .wtitle {
    font: 650 13.5px/1 var(--sans);
  }
  .wcount {
    flex: 1;
    font: 500 11px/1 var(--mono);
    color: var(--faint);
    letter-spacing: 0.06em;
  }

  .wprogress {
    position: relative;
    display: grid;
    grid-template-columns: repeat(6, 1fr);
    gap: 6px;
    width: min(640px, calc(100% - 32px));
    margin: 4px auto 0;
    padding: 0;
    list-style: none;
  }
  .wprogress li {
    display: grid;
    gap: 7px;
    min-width: 0;
  }
  .wprogress .bar {
    height: 3px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.08);
    overflow: hidden;
    position: relative;
  }
  .wprogress .bar::after {
    content: "";
    position: absolute;
    inset: 0;
    background: var(--text);
    transform: scaleX(0);
    transform-origin: left;
    transition: transform 420ms cubic-bezier(0.3, 0.8, 0.2, 1);
  }
  .wprogress li.done .bar::after,
  .wprogress li.current .bar::after {
    transform: scaleX(1);
  }
  .wprogress li.current .bar::after {
    background: var(--cpu);
  }
  .wlabel {
    font: 550 10.5px/1.2 var(--sans);
    color: var(--faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .wprogress li.current .wlabel {
    color: var(--text);
  }
  @container (max-width: 520px) {
    .wlabel {
      display: none;
    }
  }

  .wbody {
    position: relative;
    overflow-y: auto;
    display: grid;
    justify-items: center;
    padding: 28px 16px 20px;
  }
  .wstep {
    width: min(560px, 100%);
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .wstep :global(h1) {
    margin: 0;
    font: 680 26px/1.15 var(--sans);
    letter-spacing: -0.02em;
    outline: none;
  }
  .wstep :global(.lead) {
    margin: 0;
    color: var(--dim);
    font: 450 14.5px/1.5 var(--sans);
  }
  .hello {
    display: grid;
    justify-items: center;
    text-align: center;
    gap: 12px;
    padding: 12px 0 8px;
  }
  .eyes {
    width: 150px;
    height: 75px;
    margin-bottom: 6px;
  }
  .eyes .lcd {
    fill: #0b0c0e;
    stroke-width: 2;
  }
  .eyes .lcd.cpu {
    stroke: var(--cpu);
  }
  .eyes .lcd.gpu {
    stroke: var(--gpu);
  }
  .eyes .pupil {
    fill: var(--text);
    animation: look 5s ease-in-out infinite;
  }
  @keyframes look {
    0%,
    40%,
    100% {
      transform: translateX(0);
    }
    50%,
    70% {
      transform: translateX(-8px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .eyes .pupil {
      animation: none;
    }
  }
  .wfield {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 14px;
    flex-wrap: wrap;
  }
  .wfield .rlabel {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .globe {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.3;
  }
  .wfield .rots {
    min-width: 220px;
  }

  .summary {
    margin-top: 4px;
  }
  .tourbox {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    margin-top: 6px;
  }
  .tourbox .hint {
    margin: 0;
  }

  .wfoot {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    width: min(560px, calc(100% - 32px));
    margin: 0 auto;
    padding: 14px 0 18px;
    border-top: 1px solid var(--line);
  }
  .grow {
    flex: 1;
  }
  .wizard :global(.btn.big) {
    height: 36px;
    padding: 0 18px;
    font-size: 13px;
  }
  .wizard :global(.btn.ghost) {
    background: none;
    border-color: transparent;
    color: var(--dim);
  }
</style>
