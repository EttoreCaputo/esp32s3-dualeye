<!--
  The eyes' scenes and the board's sounds, all of them: each scene plays on a
  loop in its card, the picked one big on the stage. Picking one plays it on
  the board (with its sounds), or here, with the sounds on this computer.
-->
<script lang="ts">
  import { MOODS, SCENES, ScenePlayer, type Mood, type Scene } from "./scenes";
  import { SOUND_LIST, playHere } from "./sounds";
  import { monitor } from "./monitor.svelte";

  let { connected, petSounds, oldFirmware }: { connected: boolean; petSounds: boolean; oldFirmware: boolean } = $props();

  let target = $state<"board" | "here">("board");
  const onBoard = $derived(target === "board" && connected);
  let mood = $state<Mood | "all">("all");
  let picked = $state.raw<Scene>(SCENES[0]);
  let error = $state("");
  let busy = $state<string | null>(null);

  const shown = $derived(mood === "all" ? SCENES : SCENES.filter((s) => s.mood === mood));
  const moodColor: Record<Mood, string> = {
    joy: "#40e080",
    calm: "#30d5f0",
    sleepy: "#7a9cff",
    grumpy: "#ff7a50",
    silly: "#c070ff",
    robot: "#30f0b0",
    life: "#ffa030",
  };
  const pets = SOUND_LIST.filter((s) => s.pet);
  const chimes = SOUND_LIST.filter((s) => !s.pet);

  // Every card's own loop, and the stage's, on one animation frame.
  const players = new Map<string, ScenePlayer>(SCENES.map((s) => [s.name, new ScenePlayer(s)]));
  let stage = $state(new ScenePlayer(SCENES[0]));
  const canvases = new Map<string, HTMLCanvasElement>();
  let stageCanvas = $state<HTMLCanvasElement>();
  let progress = $state(0);
  let cueTimers: ReturnType<typeof setTimeout>[] = [];

  function fit(canvas: HTMLCanvasElement) {
    const dpr = window.devicePixelRatio || 1;
    const w = Math.round(canvas.clientWidth * dpr);
    const h = Math.round(canvas.clientHeight * dpr);
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
    }
  }

  function paint(canvas: HTMLCanvasElement, player: ScenePlayer, now: number) {
    fit(canvas);
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    player.step(now);
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    // Two round screens with a gap of a sixth of one between.
    const size = canvas.height;
    const gap = canvas.width - 2 * size;
    player.draw(ctx, size, gap, now);
  }

  $effect(() => {
    let frame = requestAnimationFrame(function draw(now) {
      for (const [name, canvas] of canvases) {
        const player = players.get(name);
        if (player && canvas.isConnected) paint(canvas, player, now);
      }
      if (stageCanvas) {
        paint(stageCanvas, stage, now);
        progress = stage.progress;
      }
      frame = requestAnimationFrame(draw);
    });
    return () => {
      cancelAnimationFrame(frame);
      cueTimers.forEach(clearTimeout);
    };
  });

  function card(node: HTMLCanvasElement, name: string) {
    canvases.set(name, node);
    return { destroy: () => canvases.delete(name) };
  }

  async function play(scene: Scene) {
    picked = scene;
    stage = new ScenePlayer(scene);
    cueTimers.forEach(clearTimeout);
    cueTimers = [];
    error = "";
    if (onBoard) {
      busy = scene.name;
      try {
        await monitor.playScene(scene.name);
      } catch (e) {
        error = String(e);
      } finally {
        busy = null;
      }
    } else if (petSounds || !connected) {
      cueTimers = scene.cues.map(([ms, sound]) => setTimeout(() => playHere(sound), ms));
    }
  }

  async function sound(name: string) {
    error = "";
    if (!onBoard) {
      playHere(name);
      return;
    }
    busy = name;
    try {
      await monitor.playSound(name);
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
  }

  const seconds = (ms: number) => `${(ms / 1000).toFixed(1)} s`;
</script>

<section class="pet">
  <div class="head">
    <h3>Animations</h3>
    <div class="seg" role="radiogroup" aria-label="Play on">
      <button role="radio" aria-checked={onBoard} class:on={onBoard} disabled={!connected} onclick={() => (target = "board")} title={connected ? "" : "Connect the board to play on it"}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="5" cy="8" r="3.2" /><circle cx="11" cy="8" r="3.2" /></svg>
        Board
      </button>
      <button role="radio" aria-checked={!onBoard} class:on={!onBoard} onclick={() => (target = "here")}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><rect x="2" y="3" width="12" height="8" rx="1.5" /><path d="M6 13.5h4" /></svg>
        Here
      </button>
    </div>
  </div>

  <div class="stage" style:--c={picked.color === "rainbow" ? "#ff5aa8" : picked.color}>
    <canvas bind:this={stageCanvas} class="big" aria-label="{picked.label}, playing"></canvas>
    <div class="about">
      <div class="title">
        <span class="name">{picked.label}</span>
        <span class="mood" style:--m={moodColor[picked.mood]}>{MOODS.find(([m]) => m === picked.mood)?.[1]}</span>
      </div>
      <div class="meta">
        <span class="mono">{picked.name}</span>
        <span>·</span>
        <span>{seconds(picked.ms)}</span>
        {#if picked.cues.length}
          <span>·</span>
          <span class="says">
            {#each [...new Set(picked.cues.map(([, s]) => s))] as s (s)}<span class="cue">♪ {s.replace(/_/g, " ")}</span>{/each}
          </span>
        {/if}
      </div>
      <div class="bar"><span style:width="{progress * 100}%"></span></div>
      <button class="replay" onclick={() => play(picked)} disabled={busy !== null}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M5 3.5v9l7.5-4.5z" /></svg>
        {onBoard ? "Play on the board" : "Play here"}
      </button>
    </div>
  </div>

  {#if oldFirmware && connected}
    <p class="hint">The board's firmware only knows the first 16 scenes, with no sounds or moods: update it from the Device tab.</p>
  {/if}
  {#if error}<p class="hint error">{error}</p>{/if}

  <div class="moods" role="tablist" aria-label="Mood">
    <button role="tab" aria-selected={mood === "all"} class:on={mood === "all"} onclick={() => (mood = "all")}>All <span class="n">{SCENES.length}</span></button>
    {#each MOODS as [key, label] (key)}
      <button role="tab" aria-selected={mood === key} class:on={mood === key} style:--m={moodColor[key]} onclick={() => (mood = key)}>
        <span class="dot"></span>{label}
      </button>
    {/each}
  </div>

  <div class="grid">
    {#each shown as scene (scene.name)}
      <button class="card" class:picked={picked.name === scene.name} class:busy={busy === scene.name} onclick={() => play(scene)} title="{scene.label} · {seconds(scene.ms)}">
        <canvas use:card={scene.name} aria-hidden="true"></canvas>
        <span class="label"><span class="dot" style:--m={moodColor[scene.mood]}></span>{scene.label}</span>
      </button>
    {/each}
  </div>
</section>

<section class="pet">
  <h3>Sounds</h3>
  {#if !petSounds && onBoard}
    <p class="hint">Pet sounds are off on the board: turn them on above to hear these there.</p>
  {/if}
  <div class="sounds">
    {#each pets as s (s.name)}
      <button class="snd" class:busy={busy === s.name} onclick={() => sound(s.name)} title="{s.name} · {seconds(s.ms)}">
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 6h2.5L9 3v10l-3.5-3H3z" /><path class="wave" d="M11 5.5c1 .8 1 4.2 0 5M12.8 4c1.8 1.6 1.8 6.4 0 8" /></svg>
        {s.label}
      </button>
    {/each}
  </div>
  <h4>Chimes</h4>
  <div class="sounds">
    {#each chimes as s (s.name)}
      <button class="snd chime" class:busy={busy === s.name} onclick={() => sound(s.name)} title="{s.name} · {seconds(s.ms)}">
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 2.5c-2.4 0-4 1.8-4 4.2V10l-1.2 1.8h10.4L12 10V6.7c0-2.4-1.6-4.2-4-4.2zM6.6 13.4a1.5 1.5 0 0 0 2.8 0" /></svg>
        {s.label}
      </button>
    {/each}
  </div>
</section>

<style>
  .pet {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  h3 {
    margin: 0;
    font: 600 10px/1 var(--mono);
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--faint);
  }
  h4 {
    margin: 4px 0 0;
    font: 600 10px/1 var(--mono);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--faint);
    opacity: 0.8;
  }
  .hint {
    margin: 0;
    font: 400 12.5px/1.5 var(--sans);
    color: var(--dim);
  }
  .hint.error {
    color: var(--hot);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .seg {
    display: flex;
    padding: 2px;
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--line);
  }
  .seg button {
    display: flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 10px;
    border: 0;
    border-radius: 7px;
    background: none;
    color: var(--dim);
    font: 550 11px/1 var(--sans);
    cursor: pointer;
    transition:
      background 160ms,
      color 160ms;
  }
  .seg button.on {
    background: rgba(255, 255, 255, 0.09);
    color: var(--text);
  }
  .seg button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .seg svg {
    width: 13px;
    height: 13px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
  }

  .stage {
    /* Stays in view while the cards scroll under it. */
    position: sticky;
    top: 0;
    z-index: 2;
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: 16px;
    padding: 14px;
    border-radius: 16px;
    border: 1px solid var(--line);
    background:
      radial-gradient(120% 140% at 0% 50%, color-mix(in srgb, var(--c) 14%, transparent), transparent 60%),
      #16171b;
    box-shadow: 0 10px 24px -12px rgba(0, 0, 0, 0.8);
  }
  canvas.big {
    width: 172px;
    height: 80px;
    filter: drop-shadow(0 0 18px color-mix(in srgb, var(--c) 30%, transparent));
  }
  .about {
    display: flex;
    flex-direction: column;
    gap: 7px;
    min-width: 0;
  }
  .title {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .name {
    font: 650 17px/1.1 var(--sans);
    letter-spacing: -0.02em;
  }
  .mood {
    padding: 3px 7px;
    border-radius: 999px;
    font: 600 10px/1 var(--sans);
    color: var(--m);
    background: color-mix(in srgb, var(--m) 13%, transparent);
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 5px;
    color: var(--faint);
    font: 400 11px/1.4 var(--sans);
  }
  .mono {
    font-family: var(--mono);
    font-size: 10.5px;
  }
  .says {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .cue {
    color: var(--dim);
  }
  .bar {
    height: 2px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.06);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--c);
    opacity: 0.8;
  }
  .replay {
    align-self: flex-start;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 12px 0 10px;
    border-radius: 8px;
    border: 1px solid color-mix(in srgb, var(--c) 40%, transparent);
    background: color-mix(in srgb, var(--c) 14%, transparent);
    color: var(--text);
    font: 600 11.5px/1 var(--sans);
    cursor: pointer;
    transition: background 160ms;
  }
  .replay:hover:not(:disabled) {
    background: color-mix(in srgb, var(--c) 24%, transparent);
  }
  .replay:disabled {
    opacity: 0.5;
  }
  .replay svg {
    width: 12px;
    height: 12px;
    fill: currentColor;
  }

  .moods {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }
  .moods button {
    display: flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 8px;
    border-radius: 999px;
    border: 1px solid var(--line);
    background: none;
    color: var(--dim);
    font: 550 11px/1 var(--sans);
    cursor: pointer;
    transition:
      background 160ms,
      color 160ms,
      border-color 160ms;
  }
  .moods button:hover {
    color: var(--text);
  }
  .moods button.on {
    color: var(--text);
    background: rgba(255, 255, 255, 0.07);
    border-color: rgba(255, 255, 255, 0.14);
  }
  .moods .n {
    color: var(--faint);
    font-family: var(--mono);
    font-size: 10px;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--m);
    flex: none;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(112px, 1fr));
    gap: 8px;
  }
  .card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 7px;
    padding: 10px 6px 9px;
    border-radius: 12px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.02);
    color: var(--dim);
    cursor: pointer;
    transition:
      background 160ms,
      border-color 160ms,
      transform 160ms,
      color 160ms;
  }
  .card:hover {
    background: rgba(255, 255, 255, 0.05);
    color: var(--text);
    transform: translateY(-1px);
  }
  .card.picked {
    border-color: rgba(255, 255, 255, 0.22);
    background: rgba(255, 255, 255, 0.06);
    color: var(--text);
  }
  .card.busy {
    animation: pulse 700ms ease-in-out infinite alternate;
  }
  .card canvas {
    width: 92px;
    height: 43px;
  }
  .label {
    display: flex;
    align-items: center;
    gap: 5px;
    font: 550 11px/1 var(--sans);
    white-space: nowrap;
  }

  .sounds {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .snd {
    display: flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 10px 0 8px;
    border-radius: 8px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.03);
    color: var(--dim);
    font: 550 11px/1 var(--sans);
    cursor: pointer;
    transition:
      background 140ms,
      color 140ms;
  }
  .snd:hover {
    background: rgba(255, 255, 255, 0.07);
    color: var(--text);
  }
  .snd:active {
    transform: scale(0.96);
  }
  .snd.busy {
    color: var(--text);
  }
  .snd svg {
    width: 13px;
    height: 13px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.3;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .snd:hover .wave {
    stroke: var(--cpu);
  }
  .chime:hover svg {
    stroke: var(--warm);
  }
  @keyframes pulse {
    to {
      border-color: rgba(255, 255, 255, 0.35);
    }
  }
</style>
