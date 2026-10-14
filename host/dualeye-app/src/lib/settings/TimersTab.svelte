<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import InfoTip from "../InfoTip.svelte";
  import { monitor, type ShowOn, type TimerInfo, type TimerKind } from "../monitor.svelte";
  import NeedRow from "./NeedRow.svelte";
  import { need } from "./requirements";
  import Section from "./Section.svelte";

  const tt = $derived(i18n.t.settings.timers);
  const tc = $derived(i18n.t.settings.common);
  const info = $derived(monitor.timers);

  // Read every second while the tab is open, so the times run.
  let error = $state("");
  $effect(() => {
    const load = () => monitor.timersInfo().catch((e) => (error = String(e)));
    load();
    const id = setInterval(load, 1000);
    return () => clearInterval(id);
  });

  let minutes = $state(10);
  let label = $state("");
  let remindText = $state("");
  let remindAt = $state("");
  let pomo = $state({ work: 25, rest: 5, rounds: 4 });

  async function tool(name: string, args: Record<string, unknown>) {
    error = "";
    try {
      await monitor.timerTool(name, args);
    } catch (e) {
      error = String(e);
    }
  }
  const start = (secs: number, text = "") => tool("set_timer", text.trim() ? { seconds: secs, label: text.trim() } : { seconds: secs });
  const control = (action: "cancel" | "pause" | "resume", t: TimerInfo) => tool("control_timer", { action, which: String(t.id) });
  async function remind() {
    await tool("set_reminder", { text: remindText.trim(), at: remindAt });
    if (!error) remindText = "";
  }

  const COLORS: Record<TimerKind, string> = { timer: "#f8a639", work: "#ff6347", break: "#40e080", reminder: "#3ae7ed" };
  const SHOW_ON: ShowOn[] = ["left", "right", "none"];
  const clockText = (secs: number) => {
    const two = (n: number) => String(n).padStart(2, "0");
    return secs >= 3600 ? `${Math.trunc(secs / 3600)}:${two(Math.trunc((secs % 3600) / 60))}:${two(secs % 60)}` : `${Math.trunc(secs / 60)}:${two(secs % 60)}`;
  };
  const sub = (t: TimerInfo) => (t.state === "ring" ? tt.ringing : t.state === "pause" ? tt.paused : t.ends_at ? tt.until(t.ends_at) : "");
</script>

<Section class="timers" anchor="running" title={tt.running} tip={tt.tip}>
  {#each info?.timers ?? [] as t (t.id)}
    {@const kind = tt.kinds[t.kind]}
    <div class="trow" class:ringing={t.state === "ring"} style:--tcolor={COLORS[t.kind]}>
      <span class="tdot"></span>
      <div class="tname">
        <b>{t.label ?? kind}</b>
        <small>{t.label ? `${kind} · ` : ""}{sub(t)}</small>
      </div>
      <span class="tleft" class:paused={t.state === "pause"}>{clockText(t.left_s)}</span>
      <div class="tbtns">
        {#if t.state === "run"}<button class="btn small" onclick={() => control("pause", t)}>{tt.pause}</button>{/if}
        {#if t.state === "pause"}<button class="btn small" onclick={() => control("resume", t)}>{tt.resume}</button>{/if}
        <button class="btn small" onclick={() => control("cancel", t)}>{t.state === "ring" ? tt.stop : tt.cancel}</button>
      </div>
    </div>
  {:else}
    <p class="empty">{tt.none}</p>
  {/each}
  {#if error}<p class="hint error">{error}</p>{/if}
</Section>
<Section class="timers" anchor="new-timer" title={tt.new}>
  <div class="rots quick">
    {#each [1, 3, 5, 10, 15, 30] as m (m)}
      <button class="rot" onclick={() => start(m * 60)}>{m} {tt.min}</button>
    {/each}
  </div>
  <div class="tform">
    <input class="tnum" type="number" min="1" max="1440" bind:value={minutes} aria-label={tt.minutes} />
    <span class="tunit">{tt.min}</span>
    <input class="ttext" type="text" maxlength="40" placeholder={tt.labelPlaceholder} bind:value={label} />
    <button class="btn primary" disabled={!(minutes > 0)} onclick={() => start(Math.round(minutes * 60), label).then(() => (label = ""))}>{tc.start}</button>
  </div>
</Section>
<Section class="timers" anchor="reminder" title={tt.reminder}>
  <div class="tform">
    <input class="ttext" type="text" maxlength="80" placeholder={tt.remindPlaceholder} bind:value={remindText} />
    <input class="ttime" type="time" bind:value={remindAt} aria-label={tt.at} />
    <button class="btn primary" disabled={!remindText.trim() || !remindAt} onclick={remind}>{tc.set}</button>
  </div>
</Section>
<Section class="timers" anchor="pomodoro" title={tt.pomodoro}>
  {#if info?.pomodoro}
    {@const p = info.pomodoro}
    <p class="hint">{tt.round(p.round, p.rounds, p.work_s / 60, p.break_s / 60)}</p>
    <div class="actions"><button class="btn" onclick={() => tool("pomodoro", { action: "stop" })}>{tt.stopPomodoro}</button></div>
  {:else}
    <div class="tform">
      <input class="tnum" type="number" min="1" max="120" bind:value={pomo.work} aria-label={tt.focusAria} />
      <span class="tunit">{tt.minFocus}</span>
      <input class="tnum" type="number" min="1" max="60" bind:value={pomo.rest} aria-label={tt.breakAria} />
      <span class="tunit">{tt.minBreak}</span>
      <input class="tnum" type="number" min="1" max="12" bind:value={pomo.rounds} aria-label={tt.roundsAria} />
      <span class="tunit">{tt.rounds}</span>
      <button class="btn primary" onclick={() => tool("pomodoro", { action: "start", work_minutes: pomo.work, break_minutes: pomo.rest, rounds: pomo.rounds })}>{tc.start}</button>
    </div>
  {/if}
</Section>
<Section class="timers" anchor="takes-over" title={tt.onBoard}>
  <div class="rotation">
    <span class="rlabel">{tt.takesOver}&nbsp;<InfoTip text={tt.takesOverTip} /></span>
    <div class="rots three" role="radiogroup" aria-label={tt.takesOverAria}>
      {#each SHOW_ON as value (value)}
        <button class="rot" class:checked={info?.show_on === value} role="radio" aria-checked={info?.show_on === value} onclick={() => monitor.setTimerScreen(value)}>{tt.showOn[value]}</button>
      {/each}
    </div>
  </div>
  {#if need("speak")}
    <p class="hint spaced">{tt.spokenLabel}</p>
    <NeedRow need={need("speak")} />
  {/if}
</Section>
