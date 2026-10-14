<!--
  Setup, step 3: the board's firmware. Older than the app's (or none at all):
  one Update button, its progress, esptool's output folded away. Up to date:
  ✓, and going ahead it moves on by itself.
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import InfoTip from "../InfoTip.svelte";
  import { monitor, type PortInfo } from "../monitor.svelte";
  import Glyph from "../settings/Glyph.svelte";

  let { forward, onnext }: { forward: boolean; onnext: () => void } = $props();
  const t = $derived(i18n.t.onboarding.firmware);

  let ports = $state<PortInfo[]>([]);
  $effect(() => {
    const load = () =>
      monitor
        .listPorts()
        .then((p) => (ports = p))
        .catch(() => {});
    load();
    const id = setInterval(load, 2000);
    return () => clearInterval(id);
  });
  $effect(() => {
    if (!monitor.bundled) void monitor.firmwareInfo().catch(() => {});
  });

  const connected = $derived(monitor.link === "connected");
  const boards = $derived(ports.filter((p) => p.is_board));
  /** The port esptool will use: the pinned one, else the only Espressif device plugged in. */
  const target = $derived(monitor.portSetting ?? (boards.length === 1 ? boards[0].name : null));
  const fw = $derived(monitor.boardFirmware);
  const to = $derived(monitor.bundled?.version ?? "");
  const busy = $derived(monitor.job !== "idle");
  const flashing = $derived(monitor.job === "flash" || !!monitor.setup);

  /** Where the board stands: no board, being asked, fine, or something to install. */
  const stage = $derived.by(() => {
    if (flashing) return "flashing";
    if (!connected) return boards.length ? "blank" : "noBoard";
    if (!fw) return "checking";
    if (monitor.update) return "old";
    return "ok";
  });

  // Up to date when arriving from the step before: on to the next one.
  let moved = false;
  $effect(() => {
    if (stage !== "ok" || !forward || moved || monitor.flashedAt) return;
    moved = true;
    const id = setTimeout(onnext, 1300);
    return () => clearTimeout(id);
  });

  const from = $derived(fw?.state === "version" ? `v${fw.version}` : i18n.t.main.update.old);
</script>

<p class="lead">{t.lead}</p>

<div class="fwcard">
  {#if monitor.flashedAt && (stage === "noBoard" || stage === "checking" || stage === "blank")}
    <div class="frow"><Glyph state="busy" /><span>{t.done}</span></div>
  {:else if stage === "noBoard"}
    <div class="frow"><Glyph state="off" /><span>{t.needsBoard}</span></div>
  {:else if stage === "checking"}
    <div class="frow"><Glyph state="busy" /><span>{t.checking}</span></div>
  {:else if stage === "ok"}
    <div class="frow ok">
      <Glyph state="ok" />
      <span>{t.upToDate(fw?.state === "version" ? fw.version : to)}</span>
    </div>
  {:else if stage === "flashing"}
    {#if monitor.setup}
      <div class="progress" class:indeterminate={monitor.setup.percent === null}><span style:width="{monitor.setup.percent ?? 30}%"></span></div>
      <p class="hint">{i18n.t.settings.device.preparing(monitor.setup.message)}</p>
    {:else}
      <div class="progress" role="progressbar" aria-valuenow={Math.round(monitor.flashPercent)} aria-valuemin={0} aria-valuemax={100}>
        <span style:width="{monitor.flashPercent}%"></span>
      </div>
      <p class="hint">{i18n.t.settings.device.writing(Math.round(monitor.flashPercent))}</p>
    {/if}
    <p class="hint">{t.keepPlugged}</p>
  {:else}
    <div class="frow warn">
      <Glyph state="warn" />
      <span>{stage === "blank" ? t.blank(to) : t.old(from, to)}</span>
      <InfoTip text={t.olderTip} />
    </div>
    {#if target}
      <div class="actions">
        <button class="btn primary big" disabled={busy || !to} onclick={() => monitor.flash(monitor.portSetting)}>
          {monitor.jobError ? t.again : stage === "blank" ? t.install : t.update}
        </button>
      </div>
    {:else}
      <p class="hint">{boards.length > 1 ? t.several : t.usbNeeded}</p>
    {/if}
  {/if}
</div>

{#if monitor.jobError}
  <p class="hint error">{t.failed}: {monitor.jobError}</p>
{/if}
{#if monitor.jobLog.length}
  <details>
    <summary>{t.output}</summary>
    <div class="console joblog">
      {#each monitor.jobLog as line, i (i)}<div class="log">{line}</div>{/each}
    </div>
  </details>
{/if}

<style>
  .fwcard {
    display: grid;
    gap: 10px;
    padding: 14px;
    border-radius: 12px;
    background: rgba(255, 255, 255, 0.035);
    border: 1px solid var(--line);
  }
  .frow {
    display: flex;
    align-items: center;
    gap: 10px;
    font: 500 13.5px/1.4 var(--sans);
  }
  .actions {
    margin: 0;
  }
</style>
