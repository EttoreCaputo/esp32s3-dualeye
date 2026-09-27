<script lang="ts">
  import Board from "./lib/Board.svelte";
  import Drawer from "./lib/Drawer.svelte";
  import Telemetry from "./lib/Telemetry.svelte";
  import TitleBar from "./lib/TitleBar.svelte";
  import UpdateBanner from "./lib/UpdateBanner.svelte";
  import { heatColor, screenFor, type DeviceId } from "./lib/firmware";
  import { fanRpm, monitor } from "./lib/monitor.svelte";

  monitor.start();

  let settings = $state(false);
  let drawerTab = $state<"connection" | "display" | "device" | "sensors" | "console">("connection");
  let pixels = $state(false);
  let heroW = $state(0);
  let winH = $state(0);

  // A compact mirror: the board keeps its proportions but leaves the room to the charts.
  const size = $derived(Math.round(Math.max(130, Math.min(210, heroW / 3.4, winH * 0.25))));
  const board = $derived(monitor.boardState);
  const shown = $derived(monitor.shown);

  // The face the board is on: from the last line it got, classic until the first one.
  const screen = (id: DeviceId) =>
    screenFor(id, shown?.face?.[id] ?? "classic", shown?.[id], board === "stale", board === "waiting", fanRpm(shown, id), shown?.claude);
  // A line without `rot` turns both screens upright. Before the first line the
  // board keeps its last rotation, which normally is the one picked here.
  const rotation = $derived(shown ? { cpu: shown.rot?.cpu ?? 0, gpu: shown.rot?.gpu ?? 0 } : monitor.rotation);
  const glow = (id: DeviceId) => {
    const m = shown?.[id];
    const active = (board === "live" || board === "stale") && m?.temp_c !== undefined;
    return { color: heatColor(id, m?.temp_c), level: active ? 0.25 + ((m?.load_pct ?? 0) / 100) * 0.75 : 0.12 };
  };

  const caption = $derived.by(() => {
    switch (board) {
      case "live":
        return "Mirroring the board in real time";
      case "stale":
        return "The board keeps the last frame and marks it stale";
      case "waiting":
        return "The board is up and waiting for its first frame";
      case "boot":
        return "Opening the port reset the board, it is booting";
      default:
        return monitor.link === "offline" ? "Lost the board" : "Plug the DualEye into a USB port";
    }
  });
</script>

<svelte:window bind:innerHeight={winH} />

<div class="app">
  <div class="backdrop" aria-hidden="true"></div>
  <TitleBar onSettings={() => (settings = true)} />
  <UpdateBanner
    onUpdate={() => {
      drawerTab = "device";
      settings = true;
    }}
  />

  <section class="hero" bind:clientWidth={heroW}>
    <div class="board" class:off={board === "off"}>
      <Board cpu={screen("cpu")} gpu={screen("gpu")} {board} {size} cpuGlow={glow("cpu")} gpuGlow={glow("gpu")} {pixels} {rotation} />
    </div>

    <div class="caption">
      <span class="state {board}"></span>
      <span>{caption}</span>
      {#if board === "off"}
        <button class="link" onclick={() => (settings = true)}>Connection settings</button>
      {/if}
    </div>

    <div class="toggle" role="group" aria-label="Screen rendering">
      <button class:active={!pixels} onclick={() => (pixels = false)}>Glass</button>
      <button class:active={pixels} onclick={() => (pixels = true)}>Pixels</button>
      <span class="thumb" class:right={pixels}></span>
    </div>
    <div class="spec" aria-hidden="true">
      ESP32-S3 · 2 × GC9A01 · 240 × 240
      {#if pixels}<span class="tip">Hover a screen to inspect its pixels</span>{/if}
    </div>
  </section>

  <section class="cards">
    <Telemetry id="cpu" lcd={1} metrics={monitor.last?.cpu} fan={fanRpm(monitor.last, "cpu")} samples={monitor.history} />
    <Telemetry id="gpu" lcd={2} metrics={monitor.last?.gpu} fan={fanRpm(monitor.last, "gpu")} samples={monitor.history} />
  </section>

  <Drawer bind:open={settings} bind:tab={drawerTab} />
</div>

<style>
  .app {
    position: relative;
    height: 100vh;
    display: grid;
    grid-template-rows: auto auto auto minmax(0, 1fr);
    overflow: hidden;
  }

  .backdrop {
    position: absolute;
    inset: 0;
    pointer-events: none;
    background:
      radial-gradient(60% 50% at 50% 38%, rgba(255, 255, 255, 0.035), transparent 70%),
      radial-gradient(40% 30% at 50% 100%, rgba(120, 130, 160, 0.05), transparent 70%);
  }
  .backdrop::before {
    /* A dotted floor that fades toward the edges. */
    content: "";
    position: absolute;
    inset: 0;
    background-image: radial-gradient(rgba(255, 255, 255, 0.07) 1px, transparent 1.2px);
    background-size: 22px 22px;
    mask-image: radial-gradient(70% 55% at 50% 45%, #000 10%, transparent 75%);
    -webkit-mask-image: radial-gradient(70% 55% at 50% 45%, #000 10%, transparent 75%);
    opacity: 0.55;
  }
  .backdrop::after {
    content: "";
    position: absolute;
    inset: 0;
    opacity: 0.06;
    background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='160' height='160'><filter id='n'><feTurbulence type='fractalNoise' baseFrequency='.85' numOctaves='3' stitchTiles='stitch'/></filter><rect width='100%' height='100%' filter='url(%23n)'/></svg>");
    mix-blend-mode: overlay;
  }

  .hero {
    position: relative;
    display: grid;
    place-items: center;
    /* Room for the toggle above the board and the caption below it. */
    padding: 34px 0 30px;
    /* Clip the screen glow without making the hero scrollable. */
    contain: paint;
  }
  .board {
    transition:
      filter 800ms ease,
      opacity 800ms ease;
    animation: rise 1100ms cubic-bezier(0.2, 0.8, 0.2, 1) both;
  }
  .board.off {
    filter: saturate(0.6) brightness(0.85);
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(24px) scale(0.97);
    }
  }

  .caption {
    position: absolute;
    bottom: 12px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 10px;
    font: 450 12.5px/1 var(--sans);
    color: var(--dim);
    white-space: nowrap;
  }
  .state {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--faint);
  }
  .state.live {
    background: #5ee38a;
    box-shadow: 0 0 8px #5ee38a;
  }
  .state.stale {
    background: var(--stale);
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    color: var(--text);
    font: inherit;
    text-decoration: underline;
    text-decoration-color: rgba(255, 255, 255, 0.25);
    text-underline-offset: 3px;
    cursor: pointer;
  }

  .toggle {
    position: absolute;
    top: 6px;
    right: 22px;
    display: grid;
    grid-template-columns: 1fr 1fr;
    padding: 3px;
    border-radius: 10px;
    background: rgba(255, 255, 255, 0.035);
    border: 1px solid var(--line);
  }
  .toggle button {
    position: relative;
    z-index: 1;
    width: 62px;
    height: 24px;
    border: 0;
    background: none;
    color: var(--faint);
    font: 550 11px/1 var(--sans);
    cursor: pointer;
    transition: color 200ms;
  }
  .toggle button.active {
    color: var(--text);
  }
  .thumb {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 62px;
    height: 24px;
    border-radius: 7px;
    background: rgba(255, 255, 255, 0.08);
    transition: transform 280ms cubic-bezier(0.3, 0.8, 0.2, 1);
  }
  .thumb.right {
    transform: translateX(62px);
  }
  .tip {
    display: block;
    margin-top: 8px;
    color: var(--dim);
    letter-spacing: 0.04em;
    text-transform: none;
    font-family: var(--sans);
    font-size: 11.5px;
  }
  .spec {
    position: absolute;
    top: 12px;
    left: 24px;
    font: 500 10px/1 var(--mono);
    letter-spacing: 0.12em;
    color: var(--faint);
    text-transform: uppercase;
  }

  .cards {
    position: relative;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 14px;
    padding: 0 18px 18px;
    min-height: 0;
  }
  @media (max-width: 1040px) {
    .cards {
      grid-template-columns: 1fr;
      height: auto;
    }
    .app {
      overflow-y: auto;
      grid-template-rows: auto auto auto auto;
    }
  }
</style>
