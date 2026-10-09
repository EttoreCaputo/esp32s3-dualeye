<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { monitor } from "./monitor.svelte";
  import { MOODS } from "./petMood";

  let { onSettings, onPet }: { onSettings: () => void; onPet: () => void } = $props();

  const pet = $derived(monitor.pet ? { ...(MOODS[monitor.pet.mood] ?? MOODS.content), away: monitor.pet.away } : null);

  const pill = $derived.by(() => {
    const port = monitor.port ?? "";
    if (monitor.setup) return { tone: "busy", text: "Preparing esptool", detail: monitor.setup.message };
    if (monitor.job === "flash") return { tone: "busy", text: `Flashing ${Math.round(monitor.flashPercent)}%`, detail: "esptool" };
    if (monitor.job === "identify") return { tone: "busy", text: "Identifying", detail: "esptool" };
    switch (monitor.boardState) {
      case "live":
        return { tone: "live", text: "Live", detail: port };
      case "stale":
        return { tone: "stale", text: "Stale", detail: "no data for 3 s" };
      case "waiting":
        return { tone: "busy", text: "Connected", detail: "waiting for first frame" };
      case "boot":
        return { tone: "busy", text: "Board booting", detail: port };
    }
    if (monitor.link === "offline") return { tone: "error", text: "Disconnected", detail: monitor.message };
    return { tone: "busy", text: "Searching", detail: "looking for the board on USB" };
  });

  // Restarting the ping animation on every frame written makes the dot "beat" with the data.
  const beat = $derived(monitor.sentAt);

  const win = monitor.preview ? null : getCurrentWindow();
</script>

<header class="bar" data-tauri-drag-region>
  <div class="brand" data-tauri-drag-region>
    <svg class="logo" viewBox="0 0 40 20" aria-hidden="true">
      <circle cx="9" cy="10" r="8" fill="none" stroke="var(--cpu)" stroke-width="2.4" stroke-dasharray="36 60" transform="rotate(-90 9 10)" stroke-linecap="round" />
      <circle cx="31" cy="10" r="8" fill="none" stroke="var(--gpu)" stroke-width="2.4" stroke-dasharray="24 60" transform="rotate(-90 31 10)" stroke-linecap="round" />
      <circle cx="9" cy="10" r="8" fill="none" stroke="rgba(255,255,255,.08)" stroke-width="2.4" />
      <circle cx="31" cy="10" r="8" fill="none" stroke="rgba(255,255,255,.08)" stroke-width="2.4" />
    </svg>
    <span class="name" data-tauri-drag-region>DualEye</span>
    {#if monitor.preview}<span class="badge" title="Opened outside Tauri: synthetic data">Preview data</span>{/if}
  </div>

  <div class="pill {pill.tone}" data-tauri-drag-region>
    {#key beat}<span class="led"></span>{/key}
    <span class="text">{pill.text}</span>
    {#if pill.detail}<span class="detail">{pill.detail}</span>{/if}
  </div>

  <div class="actions">
    {#if pet}
      <button class="mood" style:--c={pet.color} onclick={onPet} title="The pet's mood: open the Pet tab">
        <span class="face" aria-hidden="true"><i></i><i></i></span>
        <span>{pet.away ? "Resting" : pet.label}</span>
      </button>
    {/if}
    <button class="icon" onclick={onSettings} aria-label="Settings" title="Settings">
      <svg viewBox="0 0 24 24"><path d="M4 7h10M18 7h2M4 17h4M12 17h8" /><circle cx="16" cy="7" r="2" /><circle cx="10" cy="17" r="2" /></svg>
    </button>
    {#if win}
      <span class="sep"></span>
      <button class="icon" onclick={() => win.minimize()} aria-label="Minimize" title="Minimize">
        <svg viewBox="0 0 24 24"><path d="M6 12h12" /></svg>
      </button>
      <button class="icon" onclick={() => win.toggleMaximize()} aria-label="Maximize" title="Maximize">
        <svg viewBox="0 0 24 24"><rect x="6.5" y="6.5" width="11" height="11" rx="2.5" /></svg>
      </button>
      <button class="icon close" onclick={() => win.close()} aria-label="Hide to tray" title="Hide to tray (keeps streaming)">
        <svg viewBox="0 0 24 24"><path d="M7 7l10 10M17 7L7 17" /></svg>
      </button>
    {/if}
  </div>
</header>

<style>
  .bar {
    position: relative;
    z-index: 10;
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    height: 52px;
    padding: 0 12px 0 18px;
    user-select: none;
    -webkit-user-select: none;
  }
  .brand {
    display: flex;
    white-space: nowrap;
    min-width: 0;
    align-items: center;
    gap: 10px;
  }
  .logo {
    width: 32px;
    height: 16px;
  }
  .name {
    font: 650 13.5px/1 var(--sans);
    letter-spacing: 0.02em;
  }
  .badge {
    white-space: nowrap;
    font: 600 9.5px/1 var(--mono);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--stale);
    padding: 4px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--stale) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--stale) 25%, transparent);
  }

  .pill {
    --tone: var(--faint);
    display: flex;
    align-items: center;
    gap: 9px;
    height: 30px;
    padding: 0 14px 0 12px;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.035);
    border: 1px solid var(--line);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
    font: 550 12px/1 var(--sans);
    max-width: 46vw;
    overflow: hidden;
  }
  .pill.live {
    --tone: #5ee38a;
  }
  .pill.stale {
    --tone: var(--stale);
  }
  .pill.busy {
    --tone: #8f96a3;
  }
  .pill.error {
    --tone: var(--hot);
  }
  .led {
    position: relative;
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--tone);
    box-shadow: 0 0 10px var(--tone);
  }
  .led::after {
    content: "";
    position: absolute;
    inset: -1px;
    border-radius: 50%;
    border: 1.5px solid var(--tone);
    animation: ping 1s cubic-bezier(0, 0.5, 0.4, 1) forwards;
  }
  .pill.busy .led {
    animation: breathe 1.6s ease-in-out infinite;
  }
  .pill.busy .led::after,
  .pill.error .led::after,
  .pill.stale .led::after {
    display: none;
  }
  @keyframes ping {
    from {
      transform: scale(1);
      opacity: 0.9;
    }
    to {
      transform: scale(3.2);
      opacity: 0;
    }
  }
  @keyframes breathe {
    50% {
      opacity: 0.3;
    }
  }
  .text {
    color: var(--text);
  }
  .detail {
    font: 500 11px/1 var(--mono);
    color: var(--faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .actions {
    justify-self: end;
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .mood {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 28px;
    margin-right: 6px;
    padding: 0 11px 0 9px;
    border-radius: 999px;
    border: 1px solid color-mix(in srgb, var(--c) 30%, transparent);
    background: color-mix(in srgb, var(--c) 10%, transparent);
    color: var(--text);
    font: 550 11.5px/1 var(--sans);
    cursor: pointer;
    transition:
      background 300ms,
      border-color 300ms;
  }
  .mood:hover {
    background: color-mix(in srgb, var(--c) 18%, transparent);
  }
  /* Two little eyes in the mood's colour, blinking now and then. */
  .face {
    display: flex;
    gap: 3px;
  }
  .face i {
    width: 5px;
    height: 7px;
    border-radius: 2px;
    background: var(--c);
    animation: blink 5s infinite;
  }
  @keyframes blink {
    0%,
    94%,
    100% {
      transform: scaleY(1);
    }
    97% {
      transform: scaleY(0.15);
    }
  }
  .sep {
    width: 1px;
    height: 16px;
    background: var(--line);
    margin: 0 6px;
  }
  .icon {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border: 0;
    border-radius: 9px;
    background: transparent;
    color: var(--dim);
    cursor: pointer;
    transition:
      background 160ms,
      color 160ms;
  }
  .icon:hover {
    background: rgba(255, 255, 255, 0.06);
    color: var(--text);
  }
  .icon.close:hover {
    background: color-mix(in srgb, var(--hot) 80%, transparent);
    color: #fff;
  }
  .icon:focus-visible {
    outline: 1.5px solid var(--cpu);
    outline-offset: 1px;
  }
  .icon svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
  }
</style>
