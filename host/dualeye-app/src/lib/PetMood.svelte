<!--
  The pet's mood as the board keeps it (pet.c, get_pet): a little face
  playing the scene that goes with it, the three levels behind it, what's
  moving it now, and what it reacted to last.
-->
<script lang="ts">
  import { SCENES, ScenePlayer } from "./scenes";
  import type { PetState } from "./monitor.svelte";
  import { MOODS, NOW, REACTIONS, ago, clock, duration, scenesEvery, voice } from "./petMood";

  let { pet }: { pet: PetState } = $props();

  const mood = $derived(MOODS[pet.mood] ?? MOODS.content);
  let canvas = $state<HTMLCanvasElement>();
  let player: ScenePlayer | null = null;

  $effect(() => {
    const scene = SCENES.find((s) => s.name === mood.scene) ?? SCENES[0];
    player = new ScenePlayer(scene);
  });

  $effect(() => {
    let frame = requestAnimationFrame(function draw(now) {
      const ctx = canvas?.getContext("2d");
      if (canvas && ctx && player) {
        const dpr = window.devicePixelRatio || 1;
        const w = Math.round(canvas.clientWidth * dpr);
        const h = Math.round(canvas.clientHeight * dpr);
        if (canvas.width !== w || canvas.height !== h) {
          canvas.width = w;
          canvas.height = h;
        }
        player.step(now);
        ctx.setTransform(1, 0, 0, 1, 0, 0);
        ctx.clearRect(0, 0, w, h);
        player.draw(ctx, h, w - 2 * h, now);
      }
      frame = requestAnimationFrame(draw);
    });
    return () => cancelAnimationFrame(frame);
  });

  const levels = $derived([
    ["Energy", pet.energy, "#ffd040", "Follows the time of day; talking and music lift it"],
    ["Happiness", pet.happiness, "#40e080", "Up with conversations, music, Claude finishing; down with errors and heat"],
    ["Affection", pet.affection, "#ff5aa8", "Grows as you talk and play with it; fades slowly when ignored"],
  ] as const);
</script>

<section class="mood" style:--c={mood.color}>
  <div class="top">
    <canvas bind:this={canvas} aria-hidden="true"></canvas>
    <div class="text">
      <h3>Mood</h3>
      <div class="label">{mood.label}</div>
      <p>{pet.away ? "You're away: it rests until you're back." : mood.says}</p>
    </div>
  </div>

  <div class="levels">
    {#each levels as [name, value, color, how] (name)}
      <div class="level" title={how}>
        <div class="lhead"><span class="name">{name}</span><span class="pct">{Math.round(value * 100)}</span></div>
        <span class="track"><span class="fill" style:width="{value * 100}%" style:background={color}></span></span>
      </div>
    {/each}
  </div>

  {#if pet.now.length}
    <div class="now" aria-label="Right now">
      {#each pet.now as n (n)}
        <span class="chip" style:--n={NOW[n]?.color ?? "#9a9ea8"}><span class="dot"></span>{NOW[n]?.label ?? n}</span>
      {/each}
    </div>
  {/if}

  <dl class="facts">
    <div><dt>Voice</dt><dd>{voice(pet.pitch)} <span class="dim">×{pet.pitch.toFixed(2)}</span></dd></div>
    <div><dt>Scenes</dt><dd>{pet.away || !pet.idle_scenes ? "paused" : scenesEvery(pet.pace)}</dd></div>
    <div><dt>Last played with</dt><dd>{ago(pet.lonely_s)}</dd></div>
    {#if pet.idle_s !== undefined}
      <div><dt>You</dt><dd>{pet.idle_s < 60 ? "at the computer" : `idle for ${duration(pet.idle_s)}`}</dd></div>
    {/if}
    {#if pet.minute !== undefined}
      <div><dt>Its clock</dt><dd>{clock(pet.minute)}</dd></div>
    {/if}
  </dl>

  <div class="recent">
    <h4>Recent reactions</h4>
    {#if pet.recent.length}
      <ul>
        {#each pet.recent as r, i (i)}
          <li>
            <span class="what">{REACTIONS[r.what] ?? r.what}</span>
            <span class="scene">{r.scene.replace(/_/g, " ")}</span>
            <span class="when">{ago(r.ago_s)}</span>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="none">None since the board started{pet.reactions ? "" : ": reactions are off"}.</p>
    {/if}
  </div>
</section>

<style>
  .mood {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 14px;
    border-radius: 16px;
    border: 1px solid var(--line);
    background:
      radial-gradient(100% 90% at 0% 0%, color-mix(in srgb, var(--c) 16%, transparent), transparent 60%),
      rgba(255, 255, 255, 0.025);
    transition: background 600ms;
  }
  .top {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 14px;
    align-items: center;
  }
  canvas {
    width: 96px;
    height: 45px;
  }
  h3,
  h4 {
    margin: 0;
    font: 600 9.5px/1 var(--mono);
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--faint);
  }
  .label {
    margin-top: 5px;
    font: 650 17px/1.1 var(--sans);
    letter-spacing: -0.02em;
    color: var(--c);
  }
  p {
    margin: 4px 0 0;
    font: 400 12px/1.45 var(--sans);
    color: var(--dim);
  }
  .levels {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 12px;
  }
  .level {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .lhead {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }
  .name {
    font: 600 9.5px/1 var(--mono);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--faint);
  }
  .pct {
    font: 600 11px/1 var(--mono);
    color: var(--text);
  }
  .track {
    height: 4px;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.07);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    border-radius: 4px;
    transition: width 800ms cubic-bezier(0.3, 0.8, 0.2, 1);
  }
  .now {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 22px;
    padding: 0 9px;
    border-radius: 999px;
    font: 550 11px/1 var(--sans);
    color: var(--text);
    background: color-mix(in srgb, var(--n) 13%, transparent);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--n);
  }
  .facts {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 10px 14px;
    margin: 0;
    padding-top: 12px;
    border-top: 1px solid var(--line);
  }
  .facts div {
    min-width: 0;
  }
  dt {
    font: 600 9.5px/1 var(--mono);
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--faint);
  }
  dd {
    margin: 5px 0 0;
    font: 500 12px/1.3 var(--sans);
    color: var(--text);
  }
  .dim {
    color: var(--faint);
    font-family: var(--mono);
    font-size: 10.5px;
  }
  .recent {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 12px;
    border-top: 1px solid var(--line);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  li {
    display: grid;
    grid-template-columns: 1fr auto auto;
    gap: 10px;
    align-items: baseline;
    font: 400 12px/1.3 var(--sans);
  }
  .what {
    color: var(--text);
  }
  .scene {
    font: 500 10.5px/1 var(--mono);
    color: var(--dim);
  }
  .when {
    min-width: 64px;
    text-align: right;
    font-size: 11px;
    color: var(--faint);
  }
  .none {
    margin: 0;
    font-size: 11.5px;
    color: var(--faint);
  }
</style>
