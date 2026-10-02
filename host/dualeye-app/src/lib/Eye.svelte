<!--
  One GC9A01 screen, laid out like the create_*() face builders in main/ui_watch.c.
  Everything is drawn in panel pixels and scaled as a whole. Line heights are
  the LVGL fonts' own (bold 12: 9, montserrat 14: 16, bold 48: 35, bold 72: 52),
  which puts Montserrat's baseline where LVGL puts it.
-->
<script lang="ts">
  import {
    ARC_WIDTH,
    COLOR,
    DEVICES,
    FAN_PATH,
    CLASSIC_LAYOUT,
    LCD,
    RING_GAP,
    USAGE_ARC_SIZE,
    WARN_PATH,
    clawdPose,
    type ClaudeView,
    type DeviceId,
    type Screen,
  } from "./firmware";
  import { clawdClock, startClawdClock } from "./clawd.svelte";
  import type { BoardState } from "./monitor.svelte";

  let { id, screen, board, size, pixels = false }: {
    id: DeviceId;
    screen: Screen;
    board: BoardState;
    size: number;
    pixels?: boolean;
  } = $props();

  // The metrics faces wear their source's colours, whichever screen they're on.
  const dev = $derived(DEVICES[screen.source ?? id]);
  const lit = $derived(board !== "off");
  const ui = $derived(board !== "off" && board !== "boot");
  const C = LCD / 2;
  startClawdClock();
</script>

<!-- lv_arc: `value` 0–100 over `sweep` degrees, starting `rotation` degrees clockwise from 3 o'clock. -->
{#snippet arc(diameter: number, color: string, track: string, value: number, sweep = 360, rotation = 270)}
  {@const r = (diameter - ARC_WIDTH) / 2}
  {@const span = (sweep / 360) * 100}
  <g transform="rotate({rotation} {C} {C})">
    <circle
      cx={C}
      cy={C}
      {r}
      fill="none"
      stroke={track}
      stroke-width={ARC_WIDTH}
      stroke-linecap="round"
      pathLength="100"
      stroke-dasharray={sweep === 360 ? undefined : `${span} 100`}
    />
    {#if value > 0}
      <circle
        class="indicator"
        cx={C}
        cy={C}
        {r}
        fill="none"
        stroke={color}
        stroke-width={ARC_WIDTH}
        stroke-linecap="round"
        pathLength="100"
        stroke-dasharray="{(value / 100) * span} 100"
      />
    {/if}
  </g>
{/snippet}

<!-- create_clawd(): 16 × 5 cells of px; the large one snores. -->
{#snippet clawd(px: number, c: ClaudeView, marginBottom: number)}
  {@const pose = clawdPose(px, c.mascot, clawdClock.tick)}
  <svg class="clawd" width={16 * px} height={5 * px} style:margin-bottom="{marginBottom}px" aria-hidden="true">
    {#each pose.body as r, i (i)}<rect x={r.x} y={r.y} width={r.w} height={r.h} fill={c.mascotColor} />{/each}
    {#each pose.eyes as r, i (i)}<rect x={r.x} y={r.y} width={r.w} height={r.h} fill="#000" />{/each}
    {#if px >= 8 && pose.zzz}<text class="zzz" x={15 * px} y={-2 * px + 12} fill={COLOR.textDim}>{pose.zzz}</text>{/if}
  </svg>
{/snippet}

<!-- create_pair(): a bold 12 name and a dim 14 px value. -->
{#snippet pair(name: string, color: string, value: string, marginTop = 0)}
  <div class="row" style:gap="6px" style:margin-top="{marginTop}px">
    <span class="title" style:color={color}>{name}</span>
    <span class="dim">{value}</span>
  </div>
{/snippet}

{#snippet title(marginBottom: number)}
  <div class="title-row" style:color={screen.labelColor} style:margin-bottom="{marginBottom}px">
    {#if screen.warn}
      <svg class="warn" viewBox="0 0 512 512" aria-hidden="true"><path fill="currentColor" d={WARN_PATH} /></svg>
    {/if}
    <span class="title">{screen.title}</span>
  </div>
{/snippet}

<div class="panel" class:lit style:--scale={size / LCD} style:width="{size}px" style:height="{size}px">
  <div class="fb" class:ui>
    <svg class="rings" viewBox="0 0 {LCD} {LCD}" aria-hidden="true">
      {#if screen.claude}
        {@render arc(USAGE_ARC_SIZE, screen.claude.sessionColor, COLOR.claudeTrack, screen.claude.sessionPct)}
        {#if screen.face === "claude" && screen.claude.showWeek}
          {@render arc(USAGE_ARC_SIZE - RING_GAP, screen.claude.weekColor, COLOR.weekTrack, screen.claude.weekPct)}
        {/if}
      {:else if screen.net}
        {@render arc(USAGE_ARC_SIZE, COLOR.cyan, COLOR.tempTrack, screen.net.rxPct)}
        {@render arc(USAGE_ARC_SIZE - RING_GAP, COLOR.green, COLOR.greenTrack, screen.net.txPct)}
      {:else if screen.disk}
        {@render arc(USAGE_ARC_SIZE, screen.disk.color, COLOR.memTrack, screen.disk.pct)}
      {:else if screen.battery}
        {@render arc(USAGE_ARC_SIZE, screen.battery.color, COLOR.greenTrack, screen.battery.pct)}
      {:else if screen.face === "timer"}
        {@render arc(USAGE_ARC_SIZE, screen.timer?.color ?? COLOR.timerTrack, screen.timer?.track ?? COLOR.timerTrack, screen.timer?.pct ?? 0)}
      {:else if screen.face !== "image"}
        {@render arc(USAGE_ARC_SIZE, dev.accent, dev.track, screen.usagePct)}
        {#if screen.face === "rings"}
          {@render arc(USAGE_ARC_SIZE - RING_GAP, screen.tempRing, COLOR.tempTrack, screen.tempPct)}
          {@render arc(USAGE_ARC_SIZE - 2 * RING_GAP, COLOR.mem, COLOR.memTrack, screen.memPct)}
        {/if}
      {/if}
    </svg>

    {#if screen.claude}
      {@const c = screen.claude}
      {#if screen.face === "claude"}
        <div class="col" style:--y="0px">
          {@render clawd(4, c, 8)}
          <div class="value" style:color={c.valueColor} style:margin-bottom="6px">{c.value}</div>
          {@render pair("5H", COLOR.claude, c.reset)}
          {@render pair(c.weekName, COLOR.week, c.week, 2)}
        </div>
      {:else}
        <div class="col" style:--y="2px">
          <span class="title" style:color={COLOR.textDim} style:margin-bottom="14px">{c.model}</span>
          {@render clawd(8, c, 14)}
          {@render pair(c.status, c.statusColor, c.tokens)}
        </div>
      {/if}
    {:else if screen.net}
      <div class="col" style:--y="4px">
        {@render title(8)}
        <div class="value" style:color={screen.valueColor}>{screen.value}</div>
        <div class="row" style:color={COLOR.cyan} style:gap="5px" style:margin-top="6px"><span class="icon">↓</span>{screen.net.unit}</div>
        <div class="divider"></div>
        <div class="row big" style:color={COLOR.green} style:gap="5px"><span class="icon">↑</span>{screen.net.tx}</div>
      </div>
    {:else if screen.disk}
      <div class="col" style:--y="4px">
        {@render title(8)}
        <div class="value" style:color={screen.valueColor}>{screen.value}</div>
        <div class="row dim" style:margin-top="6px">{screen.disk.space}</div>
        {#if screen.disk.read !== null}
          <div class="divider"></div>
          {#each [["R", COLOR.mem, screen.disk.read], ["W", COLOR.warm, screen.disk.write ?? "--"]] as [name, color, value] (name)}
            <div class="row io" style:gap="6px">
              <span class="title io-name" style:color={color}>{name}</span>
              <span class="io-value">{value}</span>
            </div>
          {/each}
        {/if}
      </div>
    {:else if screen.face === "timer"}
      {@const t = screen.timer}
      {#if t}
        <div class="col" style:--y="2px">
          <span class="title" style:color={t.titleColor} style:margin-bottom="8px">{t.title}</span>
          <div class="value" class:small={t.small} style:color={t.valueColor} style:opacity={t.dim ? 0.3 : 1}>{t.value}</div>
          {#if t.label}<span class="title timer-label" style:margin-top="8px">{t.label}</span>{/if}
          <div class="row" style:color={t.infoColor} style:margin-top="4px">{t.info}</div>
        </div>
        {#if t.more}<span class="more">{t.more}</span>{/if}
      {:else}
        <div class="col" style:--y="0px">
          <span class="title" style:color={COLOR.textDim} style:margin-bottom="6px">NO TIMERS</span>
          <div class="row dim hint">Say "Alexa, set a<br />timer for 10 minutes"</div>
        </div>
      {/if}
    {:else if screen.battery}
      <div class="col" style:--y="2px">
        {@render title(6)}
        <div class="value" style:color={screen.valueColor}>{screen.value}</div>
        <span class="title" style:color={screen.battery.statusColor} style:margin="6px 0 4px">
          {screen.battery.status === "CHARGING" ? "⚡ " : ""}{screen.battery.status}
        </span>
        <div class="row dim">{screen.battery.time}</div>
      </div>
    {:else if screen.face === "image"}
      {#if screen.image}
        <img class="picture" src={screen.image} alt="" />
      {:else}
        <div class="col" style:--y="0px">
          <span class="title" style:color={COLOR.textDim} style:margin-bottom="6px">NO IMAGE</span>
          <div class="row dim hint">Pick one in the<br />DualEye app</div>
        </div>
      {/if}
    {:else if screen.face === "rings"}
      <div class="col" style:--y="2px">
        {@render title(6)}
        <div class="value" style:color={screen.valueColor}>{screen.value}</div>
        <div class="row" style:gap="8px" style:margin-top="6px">
          <span style:color={screen.usageColor}>{screen.usage}</span>
          <span style:color={screen.memColor}>{screen.mem}</span>
        </div>
      </div>
    {:else}
      {@const layout = CLASSIC_LAYOUT[screen.face as keyof typeof CLASSIC_LAYOUT]}
      <div class="col" style:--y="{layout.y}px">
        {@render title(layout.titleGap)}
        <div class="value" style:color={screen.valueColor} style:margin-bottom="2px">{screen.value}</div>
        <div class="row dim" style:gap="8px" style:margin-top="4px">
          <span>{screen.clock}</span>
          <span>{screen.watts}</span>
        </div>
        <div class="row" style:gap="6px" style:margin="2px 0 3px">
          <span style:color={screen.usageColor}>{screen.usage}</span>
          <svg class="fan" viewBox="0 0 512 512" aria-hidden="true"><path fill={COLOR.text} d={FAN_PATH} /></svg>
          <span>{screen.rpm}</span>
        </div>
        {#if layout.barW > 0}
          <div
            class="bar"
            style:width="{layout.barW}px"
            style:height="{layout.barH}px"
            style:background={COLOR.memTrack}
            style:margin-top="6px"
          >
            {#if screen.memPct > 0}
              <div class="bar-fill" style:width="{screen.memPct}%" style:background={screen.barColor}></div>
            {/if}
          </div>
        {/if}
        {#if layout.memText}
          <div class="row" style:gap="6px" style:margin-top="5px">
            <span class="title" style:color={screen.barColor}>{screen.memName}</span>
            <span class="dim">{screen.memValue}</span>
          </div>
        {/if}
      </div>
    {/if}
  </div>
  {#if pixels}<div class="grid" aria-hidden="true"></div>{/if}
</div>

<style>
  .panel {
    position: relative;
    border-radius: 50%;
    overflow: hidden;
    /* An IPS panel with the backlight off is not black, it is a deep grey-green. */
    background: radial-gradient(circle at 50% 45%, #0d0f10 0%, #07090a 70%, #050606 100%);
    transition: background 600ms ease;
    isolation: isolate;
  }
  .panel.lit {
    background: #000;
  }

  .fb {
    position: absolute;
    left: 0;
    top: 0;
    width: 240px;
    height: 240px;
    transform: scale(var(--scale));
    transform-origin: 0 0;
    opacity: 0;
    transition: opacity 380ms ease;
  }
  .fb.ui {
    opacity: 1;
  }

  .rings {
    position: absolute;
    inset: 0;
    width: 240px;
    height: 240px;
    overflow: visible;
  }
  .indicator {
    transition:
      stroke-dasharray 320ms cubic-bezier(0.3, 0.7, 0.2, 1),
      stroke 320ms ease;
  }

  /* lv_obj_align(col, LV_ALIGN_CENTER, 0, y) */
  .col {
    position: absolute;
    left: 50%;
    top: calc(50% + var(--y));
    transform: translate(-50%, -50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    white-space: nowrap;
    font-family: "Montserrat", sans-serif;
    color: #fff;
  }
  /* The hidden warning icon takes no room in LVGL's flex layout. */
  .title-row {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .title {
    font: 700 12px/9px "Montserrat", sans-serif;
    letter-spacing: 1px;
  }
  .warn {
    width: 12px;
    height: 12px;
    margin: 1.5px 0;
  }
  .value {
    font: 700 48px/35px "Montserrat", sans-serif;
    /* LVGL has no tracking; Montserrat's default is already a touch wide at 48 px. */
    letter-spacing: -0.5px;
  }
  /* The bold 32 font, for a timer over an hour. */
  .value.small {
    font: 700 32px/23px "Montserrat", sans-serif;
  }
  .divider {
    width: 56px;
    height: 1px;
    margin: 7px 0;
    background: #3a3a3c;
  }
  .icon {
    font: 700 12px/13px "Montserrat", sans-serif;
  }
  .row.big {
    font: 500 16px/18px "Montserrat", sans-serif;
  }
  .row.io + .row.io {
    margin-top: 2px;
  }
  .io-name {
    width: 10px;
  }
  .io-value {
    min-width: 72px;
    text-align: left;
  }
  .timer-label {
    max-width: 150px;
    overflow: hidden;
    text-overflow: ellipsis;
    color: #fff;
  }
  .more {
    position: absolute;
    left: 50%;
    bottom: 40px;
    transform: translateX(-50%);
    font: 500 12px/14px "Montserrat", sans-serif;
    color: #9a9a9c;
  }
  .bar {
    border-radius: 999px;
    overflow: hidden;
  }
  .bar-fill {
    height: 100%;
    border-radius: 999px;
    transition:
      width 320ms cubic-bezier(0.3, 0.7, 0.2, 1),
      background 320ms ease;
  }
  .row {
    display: flex;
    align-items: center;
    font: 500 14px/16px "Montserrat", sans-serif;
  }
  .dim {
    color: #9a9a9c;
  }
  .fan {
    width: 16px;
    height: 16px;
  }
  /* The host's copy, cropped to the middle square like the board's. */
  .picture {
    position: absolute;
    inset: 0;
    width: 240px;
    height: 240px;
    object-fit: cover;
  }
  .hint {
    text-align: center;
    white-space: normal;
  }
  .clawd {
    display: block;
    overflow: visible;
  }
  .zzz {
    font: 500 14px/16px "Montserrat", sans-serif;
  }

  /* 240 × 240 pixel lattice; only legible when drawn large (the loupe). */
  .grid {
    position: absolute;
    inset: 0;
    pointer-events: none;
    background-image:
      linear-gradient(90deg, rgba(0, 0, 0, 0.7) 0 18%, transparent 18%),
      linear-gradient(0deg, rgba(0, 0, 0, 0.7) 0 18%, transparent 18%);
    background-size: calc(var(--scale) * 1px) calc(var(--scale) * 1px);
  }
</style>
