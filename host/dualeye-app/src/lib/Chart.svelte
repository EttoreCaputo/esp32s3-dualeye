<!-- Temperature line over a load area, both on the same 0–100 scale (°C and %). -->
<script lang="ts">
  import type { Sample } from "./monitor.svelte";
  import { i18n } from "./i18n/index.svelte";

  let { samples, temp, load, color, span = 180_000 }: {
    samples: Sample[];
    temp: (s: Sample) => number | undefined;
    load: (s: Sample) => number | undefined;
    color: string;
    span?: number;
  } = $props();

  const t = $derived(i18n.t.main.telemetry);
  let w = $state(0);
  let h = $state(0);
  let hover = $state<number | null>(null);
  const uid = Math.random().toString(36).slice(2, 8);

  const pad = { t: 10, b: 20, l: 0, r: 34 };
  const end = $derived(samples.length ? samples[samples.length - 1].t : Date.now());
  // Until three minutes of history exist, stretch what there is across the plot.
  const start = $derived(samples.length > 1 ? Math.max(samples[0].t, end - span) : end - span);
  const width = $derived(Math.max(1, end - start));
  const x = (t: number) => pad.l + ((t - start) / width) * (w - pad.l - pad.r);
  const y = (v: number) => pad.t + (1 - Math.min(100, Math.max(0, v)) / 100) * (h - pad.t - pad.b);

  function line(get: (s: Sample) => number | undefined) {
    let d = "";
    let pen = false;
    for (const s of samples) {
      const v = get(s);
      if (v === undefined) {
        pen = false;
        continue;
      }
      d += `${pen ? "L" : "M"}${x(s.t).toFixed(1)},${y(v).toFixed(1)}`;
      pen = true;
    }
    return d;
  }

  const tempPath = $derived(w ? line(temp) : "");
  const loadPath = $derived(w ? line(load) : "");
  const loadArea = $derived.by(() => {
    const pts = samples.filter((s) => load(s) !== undefined);
    if (!w || pts.length < 2) return "";
    const base = y(0);
    return `${loadPath}L${x(pts[pts.length - 1].t).toFixed(1)},${base}L${x(pts[0].t).toFixed(1)},${base}Z`;
  });

  const lastTemp = $derived.by(() => {
    for (let i = samples.length - 1; i >= 0; i--) {
      const v = temp(samples[i]);
      if (v !== undefined) return { s: samples[i], v };
    }
    return null;
  });

  const hovered = $derived(hover === null ? null : samples[hover]);

  function ago(ms: number) {
    const s = Math.round(ms / 1000);
    return s >= 60 ? `−${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")} min` : `−${s} s`;
  }

  function pointer(e: PointerEvent) {
    if (!samples.length) return;
    const rect = (e.currentTarget as SVGElement).getBoundingClientRect();
    const t = start + ((e.clientX - rect.left - pad.l) / (w - pad.l - pad.r)) * width;
    let best = 0;
    for (let i = 1; i < samples.length; i++) {
      if (Math.abs(samples[i].t - t) < Math.abs(samples[best].t - t)) best = i;
    }
    hover = best;
  }
</script>

<div class="chart" bind:clientWidth={w} bind:clientHeight={h}>
  {#if w && h}
    <svg width={w} height={h} role="img" aria-label={t.chart} onpointermove={pointer} onpointerleave={() => (hover = null)}>
      <defs>
        <linearGradient id="area-{uid}" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stop-color={color} stop-opacity="0.32" />
          <stop offset="1" stop-color={color} stop-opacity="0" />
        </linearGradient>
        <linearGradient id="fade-{uid}" x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stop-color="#fff" stop-opacity="0" />
          <stop offset="0.12" stop-color="#fff" stop-opacity="1" />
        </linearGradient>
        <mask id="mask-{uid}">
          <rect x="0" y="0" width={w} height={h} fill="url(#fade-{uid})" />
        </mask>
        <filter id="glow-{uid}" x="-10%" y="-50%" width="120%" height="200%">
          <feGaussianBlur stdDeviation="3" />
        </filter>
      </defs>

      {#each [25, 50, 75, 100] as g (g)}
        <line class="grid" x1={pad.l} x2={w - pad.r} y1={y(g)} y2={y(g)} />
        <text class="axis" x={w - pad.r + 8} y={y(g) + 3.5}>{g}</text>
      {/each}
      <line class="base" x1={pad.l} x2={w - pad.r} y1={y(0)} y2={y(0)} />
      <text class="axis" x={pad.l} y={h - 4}>{ago(width)}</text>
      <text class="axis" x={w - pad.r} y={h - 4} text-anchor="end">{t.now}</text>

      <g mask="url(#mask-{uid})">
        <path d={loadArea} fill="url(#area-{uid})" />
        <path d={loadPath} fill="none" stroke={color} stroke-width="1.25" stroke-opacity="0.75" />
        <path d={tempPath} fill="none" stroke="#fff" stroke-width="5" stroke-opacity="0.18" filter="url(#glow-{uid})" />
        <path d={tempPath} fill="none" stroke="#f4f5f7" stroke-width="1.75" stroke-linejoin="round" stroke-linecap="round" />
      </g>

      {#if lastTemp && !hovered}
        <circle class="pulse" cx={x(lastTemp.s.t)} cy={y(lastTemp.v)} r="7" fill="#fff" />
        <circle cx={x(lastTemp.s.t)} cy={y(lastTemp.v)} r="3" fill="#fff" />
      {/if}

      {#if hovered}
        {@const hx = x(hovered.t)}
        {@const tv = temp(hovered)}
        {@const lv = load(hovered)}
        <line class="cursor" x1={hx} x2={hx} y1={pad.t} y2={y(0)} />
        {#if tv !== undefined}<circle cx={hx} cy={y(tv)} r="3.5" fill="#fff" />{/if}
        {#if lv !== undefined}<circle cx={hx} cy={y(lv)} r="3" fill={color} />{/if}
        <g transform="translate({Math.min(Math.max(hx - 58, 0), w - pad.r - 116)},{pad.t})">
          <rect class="tip" width="116" height="44" rx="8" />
          <text class="tip-k" x="10" y="17">{t.tipTemp}</text>
          <text class="tip-v" x="106" y="17" text-anchor="end">{tv === undefined ? "—" : `${tv.toFixed(1)} °C`}</text>
          <text class="tip-k" x="10" y="34">{t.tipLoad}</text>
          <text class="tip-v" x="106" y="34" text-anchor="end" fill={color}>{lv === undefined ? "—" : `${lv.toFixed(0)} %`}</text>
        </g>
      {/if}
    </svg>
  {/if}
</div>

<style>
  .chart {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 120px;
  }
  svg {
    position: absolute;
    inset: 0;
    display: block;
    overflow: visible;
    cursor: crosshair;
  }
  .grid {
    stroke: rgba(255, 255, 255, 0.045);
    stroke-dasharray: 2 4;
  }
  .base {
    stroke: rgba(255, 255, 255, 0.08);
  }
  .axis {
    font: 500 9.5px var(--mono);
    fill: var(--faint);
    letter-spacing: 0.04em;
  }
  .cursor {
    stroke: rgba(255, 255, 255, 0.25);
    stroke-dasharray: 2 3;
  }
  .tip {
    fill: rgba(16, 17, 20, 0.92);
    stroke: rgba(255, 255, 255, 0.1);
  }
  .tip-k {
    font: 600 9px var(--mono);
    letter-spacing: 0.1em;
    fill: var(--faint);
  }
  .tip-v {
    font: 600 11.5px var(--mono);
    fill: var(--text);
  }
  .pulse {
    transform-box: fill-box;
    transform-origin: center;
    animation: pulse 2s ease-out infinite;
  }
  @keyframes pulse {
    from {
      opacity: 0.35;
      transform: scale(0.4);
    }
    to {
      opacity: 0;
      transform: scale(2.2);
    }
  }
</style>
