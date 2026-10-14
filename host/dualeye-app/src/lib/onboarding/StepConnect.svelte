<!--
  Setup, step 2: the board on USB, found by itself (the bridge looks all the
  time), or on WiFi, which is set up through USB once. A board without the
  DualEye firmware shows up as an Espressif port only: the next step installs it.
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import InfoTip from "../InfoTip.svelte";
  import { monitor, type PortInfo } from "../monitor.svelte";
  import Glyph from "../settings/Glyph.svelte";
  import WifiSetup from "../settings/WifiSetup.svelte";

  const t = $derived(i18n.t.onboarding.connect);
  const td = $derived(i18n.t.settings.device);
  let mode = $state<"usb" | "wifi">(monitor.port?.startsWith("WiFi") ? "wifi" : "usb");

  let ports = $state<PortInfo[]>([]);
  /** When an Espressif port first showed up without the board answering on it. */
  let blankSince = $state(0);
  const started = Date.now();
  $effect(() => {
    const load = () =>
      monitor
        .listPorts()
        .then((p) => (ports = p))
        .catch(() => {});
    load();
    const id = setInterval(load, 1500);
    return () => clearInterval(id);
  });

  const connected = $derived(monitor.link === "connected");
  const boards = $derived(ports.filter((p) => p.is_board));
  $effect(() => {
    if (connected || !boards.length) blankSince = 0;
    else if (!blankSince) blankSince = Date.now();
  });
  /** A board that's been plugged in a while and still doesn't answer has no firmware (or a very old one). */
  const blank = $derived(!connected && !!blankSince && monitor.now - blankSince > 5000);
  const slow = $derived(!connected && !blank && monitor.now - started > 8000);
</script>

<div class="rots two modes" role="radiogroup" aria-label={t.title}>
  <button class="rot" class:checked={mode === "usb"} role="radio" aria-checked={mode === "usb"} onclick={() => (mode = "usb")}>
    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 1.5v10M5.5 4 8 1.5 10.5 4M8 8.5l-3-2V5M8 10l3-2V6.5M8 11.5a1.5 1.5 0 1 1 0 3 1.5 1.5 0 0 1 0-3z" /></svg>
    {t.usb}
  </button>
  <button class="rot" class:checked={mode === "wifi"} role="radio" aria-checked={mode === "wifi"} onclick={() => (mode = "wifi")}>
    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M1.8 6.2a9 9 0 0 1 12.4 0M4 8.6a6 6 0 0 1 8 0M6.2 11a3 3 0 0 1 3.6 0M8 13.3v.1" /></svg>
    {t.wifi}
  </button>
</div>

<!-- The cable going into the board: it plugs in for as long as nothing is found. -->
<div class="plug" class:in={connected} aria-hidden="true">
  <svg viewBox="0 0 300 90">
    <rect x="20" y="15" width="150" height="60" rx="30" class="pcb" />
    <circle cx="50" cy="45" r="22" class="lcd l" />
    <circle cx="140" cy="45" r="22" class="lcd r" />
    <rect x="166" y="38" width="10" height="14" rx="3" class="port" />
    <g class="cable">
      <rect x="180" y="39" width="16" height="12" rx="4" class="metal" />
      <rect x="194" y="35" width="30" height="20" rx="6" class="boot" />
      <path d="M224 45 H300" class="wire" />
    </g>
  </svg>
</div>

{#if mode === "usb"}
  {#if connected}
    <div class="found ok" role="status">
      <Glyph state="ok" />
      <span>{t.found(monitor.port ?? "")}</span>
    </div>
  {:else if blank}
    <div class="found warn" role="status">
      <Glyph state="warn" />
      <span>{t.blank}</span>
    </div>
  {:else}
    <p class="lead">{t.plug}</p>
    <div class="found" role="status">
      <Glyph state="busy" />
      <span>{slow ? t.notFound : t.looking}</span>
      {#if slow}
        <InfoTip label={t.tipsLabel}>
          • {t.tipCable}<br />• {t.tipPort}<br />• {t.tipDriver}
        </InfoTip>
      {/if}
    </div>
    {#if monitor.permissionDenied}
      <p class="hint">{td.linux[0]}<code>dialout</code>{td.linux[1]}</p>
      <pre>sudo usermod -aG dialout "$USER"</pre>
    {/if}
  {/if}
{:else}
  <p class="lead">{t.wifiFirst}</p>
  <section class="wifi">
    {#if connected}
      <WifiSetup />
    {:else}
      <div class="found" role="status">
        <Glyph state="busy" />
        <span>{t.looking}</span>
      </div>
    {/if}
  </section>
  <p class="hint">{t.wifiLater}</p>
{/if}

<style>
  .modes {
    max-width: 300px;
  }
  .modes svg {
    width: 14px;
    height: 14px;
    margin-right: 6px;
    vertical-align: -2px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .plug {
    display: grid;
    place-items: center;
    padding: 8px 0;
  }
  .plug svg {
    width: min(320px, 100%);
    height: auto;
  }
  .pcb {
    fill: #15171b;
    stroke: rgba(255, 255, 255, 0.12);
  }
  .lcd {
    fill: #08090a;
    stroke-width: 2;
    transition: fill 600ms;
  }
  .lcd.l {
    stroke: var(--cpu);
  }
  .lcd.r {
    stroke: var(--gpu);
  }
  .in .lcd.l {
    fill: color-mix(in srgb, var(--cpu) 22%, #08090a);
  }
  .in .lcd.r {
    fill: color-mix(in srgb, var(--gpu) 22%, #08090a);
  }
  .port {
    fill: #2a2d33;
  }
  .metal {
    fill: #9aa0a8;
  }
  .boot {
    fill: #2c2f35;
    stroke: rgba(255, 255, 255, 0.15);
  }
  .wire {
    fill: none;
    stroke: #2c2f35;
    stroke-width: 7;
    stroke-linecap: round;
  }
  .cable {
    animation: plug 2.6s cubic-bezier(0.5, 0, 0.3, 1) infinite;
  }
  .in .cable {
    animation: none;
    transform: translateX(-12px);
    transition: transform 400ms;
  }
  @keyframes plug {
    0%,
    15% {
      transform: translateX(34px);
    }
    55%,
    85% {
      transform: translateX(-12px);
    }
    100% {
      transform: translateX(34px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .cable {
      animation: none;
      transform: translateX(-12px);
    }
  }
  .found {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border-radius: 12px;
    background: rgba(255, 255, 255, 0.035);
    border: 1px solid var(--line);
    font: 500 13.5px/1.4 var(--sans);
  }
  .found.ok {
    border-color: color-mix(in srgb, #5ee38a 35%, transparent);
  }
  .found.warn {
    border-color: color-mix(in srgb, var(--stale) 35%, transparent);
  }
</style>
