<!--
  The Waveshare ESP32-S3-DualEye-LCD-1.28, drawn: two round modules joined by
  the small bridge PCB with its FPC connectors. Screens are live mirrors.
-->
<script lang="ts">
  import Eye from "./Eye.svelte";
  import VoiceEyes from "./VoiceEyes.svelte";
  import type { Rotations, Screen } from "./firmware";
  import type { BoardState, VoiceState } from "./monitor.svelte";

  let { cpu, gpu, board, size, cpuGlow, gpuGlow, pixels = false, rotation = { cpu: 0, gpu: 0 }, voice = "idle", eyes = true }: {
    cpu: Screen;
    gpu: Screen;
    /** How the board has turned each screen: shown as a badge, the mirror stays upright to stay readable. */
    rotation?: Rotations;
    board: BoardState;
    size: number;
    cpuGlow: { color: string; level: number };
    gpuGlow: { color: string; level: number };
    pixels?: boolean;
    /** The board's voice overlay, as the firmware draws it (ui_voice.c). */
    voice?: VoiceState;
    /** The board draws eyes over the whole screens instead of the ring (ui_eyes.c). */
    eyes?: boolean;
  } = $props();

  let tiltX = $state(0);
  let tiltY = $state(0);
  let tracking = $state(false);

  function move(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement;
    const r = el.getBoundingClientRect();
    tiltX = ((e.clientX - r.left) / r.width - 0.5) * 2;
    tiltY = ((e.clientY - r.top) / r.height - 0.5) * 2;
    tracking = true;
  }
  function leave() {
    tiltX = 0;
    tiltY = 0;
    tracking = false;
  }

  // The 1.28" panel fills most of the module; the black bezel is a thin ring.
  const lcd = $derived(Math.round(size * 0.915));

  // Pixel loupe: a second, magnified copy of the screen under the pointer.
  const ZOOM = 4;
  const LOUPE = 156;
  let loupe = $state<{ id: "cpu" | "gpu"; x: number; y: number } | null>(null);
  function inspect(id: "cpu" | "gpu", e: PointerEvent) {
    if (!pixels) return;
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    loupe = { id, x: ((e.clientX - r.left) / r.width) * lcd, y: ((e.clientY - r.top) / r.height) * lcd };
  }
  const lit = $derived(board !== "off");
</script>

{#snippet turned(deg: number)}
  {#if deg}
    <span class="turned" title="The board draws this screen turned {deg}° clockwise">
      <svg viewBox="0 0 16 16" aria-hidden="true">
        <path d="M13 8a5 5 0 1 1-1.6-3.7" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
        <path d="M13.6 2.2v3.6H10z" fill="currentColor" />
      </svg>
      {deg}°
    </span>
  {/if}
{/snippet}

<div
  class="stage"
  class:tracking
  role="presentation"
  onpointermove={move}
  onpointerleave={leave}
  style:--d="{size}px"
  style:--tx={tiltX}
  style:--ty={tiltY}
>
  <div class="glow left" style:--c={cpuGlow.color} style:--i={lit ? cpuGlow.level : 0}></div>
  <div class="glow right" style:--c={gpuGlow.color} style:--i={lit ? gpuGlow.level : 0}></div>

  <div class="board">
    <div class="module">
      <div class="bezel">
        <div class="well" class:inspect={pixels} role="presentation" onpointermove={(e) => inspect("cpu", e)} onpointerleave={() => (loupe = null)}>
          <Eye id="cpu" screen={cpu} {board} size={lcd} />
          {#if lit && eyes}<VoiceEyes eye={0} size={lcd} {voice} />{:else if lit && voice !== "idle"}<div class="voice {voice}" aria-hidden="true"></div>{/if}
          <div class="glass"></div>
          {#if pixels && loupe?.id === "cpu"}
            <div class="loupe" style:left="{loupe.x}px" style:top="{loupe.y}px" style:--l="{LOUPE}px">
              <div class="lens" style:transform="translate({LOUPE / 2 - loupe.x * ZOOM}px, {LOUPE / 2 - loupe.y * ZOOM}px)">
                <Eye id="cpu" screen={cpu} {board} size={lcd * ZOOM} pixels />
              </div>
            </div>
          {/if}
        </div>
      </div>
      {@render turned(rotation.cpu)}
      <div class="fpc inner-right"></div>
    </div>

    <div class="bridge" aria-hidden="true">
      <svg viewBox="0 0 120 100" preserveAspectRatio="none">
        <defs>
          <linearGradient id="pcb" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stop-color="#26292e" />
            <stop offset="0.5" stop-color="#17191c" />
            <stop offset="1" stop-color="#0e0f11" />
          </linearGradient>
          <linearGradient id="pad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stop-color="#e9ecef" />
            <stop offset="1" stop-color="#8b9096" />
          </linearGradient>
        </defs>
        <rect x="0" y="0" width="120" height="100" rx="5" fill="url(#pcb)" />
        <g stroke="#2d3137" stroke-width="0.8" fill="none" opacity="0.9">
          <path d="M8 22 H44 L52 30 H70 L78 22 H112" />
          <path d="M8 78 H40 L48 70 H72 L80 78 H112" />
          <path d="M20 40 H100" />
          <path d="M20 60 H56 L60 56 H100" />
          <path d="M60 12 V30" />
          <path d="M60 70 V88" />
        </g>
        <g fill="url(#pad)" opacity="0.55">
          <rect x="39" y="34" width="4" height="9" rx="0.8" />
          <rect x="39" y="57" width="4" height="9" rx="0.8" />
          <rect x="77" y="34" width="4" height="9" rx="0.8" />
          <rect x="77" y="57" width="4" height="9" rx="0.8" />
        </g>
        <g fill="#0a0b0c">
          <circle cx="54" cy="50" r="2.2" />
          <circle cx="66" cy="50" r="2.2" />
        </g>
        <g fill="#3a3025">
          <rect x="24" y="46" width="5" height="3" rx="0.5" />
          <rect x="91" y="51" width="5" height="3" rx="0.5" />
          <rect x="57" y="16" width="6" height="3" rx="0.5" />
        </g>
        <rect x="0.4" y="0.4" width="119.2" height="99.2" rx="5" fill="none" stroke="rgba(255,255,255,0.06)" stroke-width="0.8" />
      </svg>
    </div>

    <div class="module">
      <div class="bezel">
        <div class="well" class:inspect={pixels} role="presentation" onpointermove={(e) => inspect("gpu", e)} onpointerleave={() => (loupe = null)}>
          <Eye id="gpu" screen={gpu} {board} size={lcd} />
          {#if lit && eyes}<VoiceEyes eye={1} size={lcd} {voice} />{:else if lit && voice !== "idle"}<div class="voice {voice}" aria-hidden="true"></div>{/if}
          <div class="glass"></div>
          {#if pixels && loupe?.id === "gpu"}
            <div class="loupe" style:left="{loupe.x}px" style:top="{loupe.y}px" style:--l="{LOUPE}px">
              <div class="lens" style:transform="translate({LOUPE / 2 - loupe.x * ZOOM}px, {LOUPE / 2 - loupe.y * ZOOM}px)">
                <Eye id="gpu" screen={gpu} {board} size={lcd * ZOOM} pixels />
              </div>
            </div>
          {/if}
        </div>
      </div>
      {@render turned(rotation.gpu)}
      <div class="fpc inner-left"></div>
    </div>
  </div>
  <div class="shadow"></div>
</div>

<style>
  .stage {
    --gap: calc(var(--d) * 0.3);
    position: relative;
    display: grid;
    place-items: center;
    padding: calc(var(--d) * 0.12) calc(var(--d) * 0.2) calc(var(--d) * 0.28);
    perspective: 1600px;
  }

  .board {
    position: relative;
    display: flex;
    align-items: center;
    transform-style: preserve-3d;
    transform: rotateX(calc(var(--ty) * -7deg)) rotateY(calc(var(--tx) * 9deg));
    transition: transform 900ms cubic-bezier(0.2, 0.8, 0.2, 1);
    -webkit-box-reflect: below calc(var(--d) * 0.06) linear-gradient(transparent 62%, rgba(255, 255, 255, 0.13));
    z-index: 1;
  }
  .stage.tracking .board {
    transition: transform 180ms ease-out;
  }

  .module {
    position: relative;
    width: var(--d);
    height: var(--d);
    z-index: 2;
  }

  /* Above the module, clear of the reflection below the board. */
  .turned {
    position: absolute;
    left: 50%;
    bottom: calc(100% + var(--d) * 0.035);
    translate: -50% 0;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 3px 7px 3px 6px;
    border-radius: 999px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.04);
    color: var(--dim);
    font: 550 10px/1 var(--mono);
    white-space: nowrap;
    animation: turned-in 300ms ease both;
  }
  .turned svg {
    width: 10px;
    height: 10px;
  }
  @keyframes turned-in {
    from {
      opacity: 0;
      translate: -50% 4px;
    }
  }

  /* Glossy black bezel with the thin bright rim visible in product shots. */
  .bezel {
    position: absolute;
    inset: 0;
    border-radius: 50%;
    background:
      radial-gradient(circle at 50% 50%, #000 0 60%, transparent 70%),
      conic-gradient(from 210deg, #1b1d20, #07080a 18%, #202328 34%, #08090b 52%, #16181b 70%, #050607 86%, #1b1d20);
    box-shadow:
      0 0 0 1px rgba(255, 255, 255, 0.16),
      0 0 0 2.5px #0a0b0c,
      0 0 0 3.5px rgba(255, 255, 255, 0.07),
      0 calc(var(--d) * 0.08) calc(var(--d) * 0.16) rgba(0, 0, 0, 0.7),
      0 calc(var(--d) * 0.02) calc(var(--d) * 0.04) rgba(0, 0, 0, 0.6),
      inset 0 1px 1px rgba(255, 255, 255, 0.18),
      inset 0 -1px 2px rgba(0, 0, 0, 0.9);
    display: grid;
    place-items: center;
  }
  .well {
    position: relative;
    border-radius: 50%;
    box-shadow:
      0 0 0 1px #000,
      0 0 0 2px rgba(255, 255, 255, 0.05),
      inset 0 0 calc(var(--d) * 0.02) rgba(0, 0, 0, 0.9);
    line-height: 0;
  }

  .well.inspect {
    cursor: none;
  }
  .loupe {
    position: absolute;
    z-index: 5;
    width: var(--l);
    height: var(--l);
    margin: calc(var(--l) / -2) 0 0 calc(var(--l) / -2);
    border-radius: 50%;
    overflow: hidden;
    background: #000;
    pointer-events: none;
    box-shadow:
      0 0 0 1.5px rgba(255, 255, 255, 0.55),
      0 0 0 6px rgba(10, 11, 12, 0.85),
      0 0 0 7px rgba(255, 255, 255, 0.12),
      0 24px 60px rgba(0, 0, 0, 0.8);
    animation: loupe-in 180ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  .loupe::after {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: 50%;
    background: radial-gradient(circle at 35% 25%, rgba(255, 255, 255, 0.14), transparent 45%);
  }
  .lens {
    position: absolute;
    left: 0;
    top: 0;
    line-height: 0;
  }
  @keyframes loupe-in {
    from {
      opacity: 0;
      transform: scale(0.6);
    }
  }

  /* Cover glass: a specular highlight that drifts against the tilt, plus a soft streak. */
  /* 8 px of the 240 px panel, colours from main/ui_voice.c. */
  .voice {
    position: absolute;
    inset: 0;
    border-radius: 50%;
    pointer-events: none;
    --w: calc(var(--d) * 0.915 * 8 / 240);
    mask: radial-gradient(closest-side, transparent calc(100% - var(--w)), #000 calc(100% - var(--w) + 0.5px));
  }
  .voice.listening {
    background: #30d5f0;
  }
  .voice.speaking {
    background: #40e080;
  }
  .voice.error {
    background: #ff4040;
  }
  .voice.thinking {
    background: conic-gradient(#ffb020 0deg 70deg, transparent 70deg);
    animation: voice-spin 1.2s linear infinite;
  }
  @keyframes voice-spin {
    to {
      transform: rotate(360deg);
    }
  }

  .glass {
    position: absolute;
    inset: 0;
    border-radius: 50%;
    pointer-events: none;
    background:
      radial-gradient(
        ellipse 55% 38% at calc(32% - var(--tx) * 10%) calc(16% - var(--ty) * 8%),
        rgba(255, 255, 255, 0.13),
        rgba(255, 255, 255, 0.03) 55%,
        transparent 72%
      ),
      linear-gradient(
        calc(118deg + var(--tx) * 8deg),
        transparent 44%,
        rgba(255, 255, 255, 0.035) 50%,
        transparent 57%
      ),
      radial-gradient(circle at 50% 50%, transparent 66%, rgba(255, 255, 255, 0.025) 71%, transparent 72%);
    box-shadow:
      inset 0 0 0 1px rgba(255, 255, 255, 0.05),
      inset 0 calc(var(--d) * 0.01) calc(var(--d) * 0.03) rgba(255, 255, 255, 0.03);
    transition: background-position 400ms;
  }

  .bridge {
    width: calc(var(--d) * 0.42);
    height: calc(var(--d) * 0.34);
    margin: 0 calc(var(--d) * -0.06);
    z-index: 1;
    filter: drop-shadow(0 calc(var(--d) * 0.03) calc(var(--d) * 0.04) rgba(0, 0, 0, 0.6));
  }
  .bridge svg {
    width: 100%;
    height: 100%;
    display: block;
  }

  /* FPC connectors: translucent blue film over gold contacts. */
  .fpc {
    position: absolute;
    top: 50%;
    width: calc(var(--d) * 0.055);
    height: calc(var(--d) * 0.22);
    transform: translateY(-50%);
    border-radius: 2px;
    background:
      repeating-linear-gradient(180deg, rgba(210, 190, 120, 0.55) 0 1px, transparent 1px 3px),
      linear-gradient(90deg, #11306e, #2f63c9 45%, #173a86);
    box-shadow:
      inset 0 0 0 1px rgba(150, 190, 255, 0.35),
      0 1px 3px rgba(0, 0, 0, 0.6);
    z-index: 3;
  }
  .fpc.inner-right {
    right: calc(var(--d) * -0.02);
  }
  .fpc.inner-left {
    left: calc(var(--d) * -0.02);
  }

  /* Coloured light each screen throws on its surroundings. */
  .glow {
    position: absolute;
    top: 50%;
    width: calc(var(--d) * 1.9);
    height: calc(var(--d) * 1.9);
    border-radius: 50%;
    background: radial-gradient(circle, var(--c) 0%, transparent 62%);
    opacity: calc(0.08 + var(--i) * 0.42);
    filter: blur(calc(var(--d) * 0.12));
    transform: translate(-50%, -54%);
    transition:
      opacity 1.2s ease,
      background 1.2s ease;
    pointer-events: none;
    z-index: 0;
  }
  .glow.left {
    left: calc(50% - var(--d) * 0.67);
  }
  .glow.right {
    left: calc(50% + var(--d) * 0.67);
  }

  .shadow {
    position: absolute;
    left: 50%;
    bottom: calc(var(--d) * 0.1);
    width: calc(var(--d) * 2.2);
    height: calc(var(--d) * 0.12);
    transform: translateX(calc(-50% + var(--tx) * -8px));
    background: radial-gradient(ellipse at center, rgba(0, 0, 0, 0.75), transparent 70%);
    filter: blur(8px);
    z-index: 0;
    transition: transform 900ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
</style>
