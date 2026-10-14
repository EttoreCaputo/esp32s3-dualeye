<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import InfoTip from "../InfoTip.svelte";
  import { monitor, type PowerHelper, type Reading } from "../monitor.svelte";
  import Section from "./Section.svelte";
  import Toggle from "./Toggle.svelte";

  const tg = $derived(i18n.t.settings.diag);
  const isMac = typeof navigator !== "undefined" && /Mac/.test(navigator.userAgent);

  /** The sensors or the board's console. */
  let view = $state<"sensors" | "console">("sensors");
  let readings = $state<Reading[]>([]);
  let powerHelper = $state<PowerHelper | null>(null);
  let powerBusy = $state(false);

  $effect(() => {
    if (view !== "sensors") return;
    let alive = true;
    const load = async () => {
      const [r, p] = await Promise.all([monitor.readings(), monitor.powerHelper()]);
      if (!alive) return;
      readings = r;
      // Keep the last error until the next toggle.
      powerHelper = { state: p.state, error: powerHelper?.error ?? null };
    };
    load();
    const id = setInterval(load, 2000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  });

  async function togglePowerHelper(on: boolean) {
    powerBusy = true;
    try {
      powerHelper = await monitor.setPowerHelper(on);
    } catch (e) {
      powerHelper = { state: powerHelper?.state ?? "off", error: String(e) };
    } finally {
      powerBusy = false;
    }
  }

  const groups = $derived.by(() => {
    const m = new Map<string, Reading[]>();
    for (const r of readings) m.set(r.source, [...(m.get(r.source) ?? []), r]);
    return [...m.entries()];
  });
  const fmt = (r: Reading) => (r.unit === "RPM" || r.unit === "%" || r.unit === "MB" ? r.value.toFixed(0) : r.value.toFixed(1));

  // The console follows new lines unless scrolled up.
  let consoleEl = $state<HTMLElement>();
  let follow = $state(true);
  $effect(() => {
    void monitor.logs.length;
    if (follow && consoleEl) queueMicrotask(() => consoleEl && (consoleEl.scrollTop = consoleEl.scrollHeight));
  });
  function scrolled() {
    if (consoleEl) follow = consoleEl.scrollHeight - consoleEl.scrollTop - consoleEl.clientHeight < 24;
  }
</script>

<div class="rots sub" role="tablist" aria-label={tg.aria}>
  <button class="rot" class:checked={view === "sensors"} role="tab" aria-selected={view === "sensors"} onclick={() => (view = "sensors")}>{tg.sensors}</button>
  <button class="rot" class:checked={view === "console"} role="tab" aria-selected={view === "console"} data-anchor="console" onclick={() => (view = "console")}>{tg.console}</button>
</div>
{#if view === "sensors"}
  <h3 class="flat" data-anchor="sensors">{tg.allReadings}&nbsp;<InfoTip text={tg.sensorsTip} /></h3>
  {#if isMac && powerHelper}
    <Section anchor="power" title={tg.power}>
      {#if powerHelper.state === "unavailable"}
        <p class="hint">{tg.powerUnavailable}</p>
      {:else}
        <Toggle checked={powerHelper.state !== "off"} disabled={powerBusy} label={tg.powerSwitch} tip={tg.powerTip} onchange={togglePowerHelper} />
        {#if powerHelper.state === "needs_approval"}
          <p class="hint">{tg.approve}</p>
          <div class="actions">
            <button class="btn" onclick={() => monitor.openPowerHelperSettings()}>{tg.openSettings}</button>
          </div>
        {/if}
      {/if}
      {#if powerHelper.error}<p class="hint error">{powerHelper.error}</p>{/if}
    </Section>
  {/if}
  {#each groups as [source, list] (source)}
    <section class="group">
      <h3>{source}</h3>
      <ul>
        {#each list as r (r.label)}
          <li><span>{r.label}</span><b>{fmt(r)}<small>{r.unit}</small></b></li>
        {/each}
      </ul>
    </section>
  {:else}
    <p class="empty">{tg.readingSensors}</p>
  {/each}
  <Section anchor="stream" title={tg.stream}>
    <dl class="facts">
      <div><dt>{tg.rate}</dt><dd>1 Hz</dd></div>
      <div><dt>{tg.baud}</dt><dd>115200</dd></div>
      <div><dt>{tg.format}</dt><dd>{tg.jsonLine}</dd></div>
      <div><dt>{tg.staleAfter}</dt><dd>3 s</dd></div>
    </dl>
  </Section>
{:else}
  <div class="console" bind:this={consoleEl} onscroll={scrolled}>
    {#each monitor.logs as line, i (i)}
      <div class="log" class:warn={line.startsWith("W ")} class:err={line.startsWith("E ")}>{line}</div>
    {:else}
      <p class="empty">{tg.consoleEmpty}</p>
    {/each}
  </div>
{/if}
