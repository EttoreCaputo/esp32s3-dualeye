<script lang="ts">
  import { formatTokens, isClaudeFace } from "../firmware";
  import { i18n } from "../i18n/index.svelte";
  import InfoTip from "../InfoTip.svelte";
  import { monitor } from "../monitor.svelte";
  import Advanced from "./Advanced.svelte";
  import ClaudeConnect from "./ClaudeConnect.svelte";
  import { settingsData } from "./data.svelte";
  import NeedRow from "./NeedRow.svelte";
  import { need } from "./requirements";
  import Section from "./Section.svelte";
  import Toggle from "./Toggle.svelte";

  const tx = $derived(i18n.t.settings);
  const tcl = $derived(tx.claude);
  const tc = $derived(tx.common);
  const link = $derived(settingsData.link);
  const alerts = $derived(settingsData.alerts);
  const mcp = $derived(settingsData.mcp);

  $effect(() => {
    const load = () => settingsData.loadClaude().catch(() => {});
    load();
    const id = setInterval(load, 3000);
    return () => clearInterval(id);
  });

  const ago = (s: number) => tc.ago(s);
  const usage = $derived(monitor.last?.claude);
  const usesClaude = $derived(isClaudeFace(monitor.faces.cpu) || isClaudeFace(monitor.faces.gpu));

  const anyOn = $derived(!!link?.connected || !!alerts?.hooks.connected || !!mcp?.registered);

  // The manual way for Claude Desktop, which "Connect" doesn't set up.
  const mcpDesktop = $derived(mcp?.command ? JSON.stringify({ mcpServers: { dualeye: { command: mcp.command, args: mcp.args } } }, null, 2) : "");
  let copied = $state(false);
  async function copyDesktop() {
    await navigator.clipboard.writeText(mcpDesktop);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }

  let alertsError = $state("");
  let testingAlert = $state(false);
  async function testAlert() {
    testingAlert = true;
    alertsError = "";
    try {
      await monitor.testClaudeAlert();
      // The bridge gives it in a second or two (longer while it speaks).
      setTimeout(() => settingsData.loadClaude(), 1500);
    } catch (e) {
      alertsError = String(e);
    } finally {
      testingAlert = false;
    }
  }
  const DONE_AFTER = [30, 60, 120, 300];
  const ALERT_LANGUAGES: [string, string][] = [
    ["it", "Italiano"],
    ["en", "English"],
  ];
</script>

<Section class="claude" anchor="connect" title={tcl.code} tip={tcl.codeTip}>
  <p class="hint">{usage ? tcl.usage(formatTokens(usage.tok), formatTokens(usage.today)) : usesClaude ? tcl.noUsage : tcl.why}</p>
  <ClaudeConnect />
</Section>

{#if alerts}
  {@const a = alerts.settings}
  <Section class="alerts" anchor="alerts" title={tcl.alerts} tip={tcl.alertsTip} dimmed={!anyOn}>
    <!-- Not connected at all: "Connect to Claude Code" above covers these. -->
    {#if anyOn}<NeedRow need={need("hooks")} />{/if}
    <Toggle anchor="alert-needs" checked={a.needs_you} label={tcl.needsYou} onchange={(on) => settingsData.setAlerts({ needs_you: on })} />
    <Toggle anchor="alert-done" checked={a.done} label={tcl.done} onchange={(on) => settingsData.setAlerts({ done: on })} />
    <Toggle anchor="alert-usage" checked={a.usage} label={tcl.usageAlert} onchange={(on) => settingsData.setAlerts({ usage: on })} />
    {#if a.usage && anyOn}<NeedRow need={need("statusline")} />{/if}
    <Toggle anchor="alert-speak" checked={a.speak} label={tcl.speak} onchange={(on) => settingsData.setAlerts({ speak: on })} />
    {#if a.speak}<NeedRow need={need("speak")} />{/if}
    <div class="rotation" data-anchor="alert-long">
      <span class="rlabel">{tcl.longTask}</span>
      <div class="rots" role="radiogroup" aria-label={tcl.longTask}>
        {#each DONE_AFTER as secs (secs)}
          <button
            class="rot"
            class:checked={a.done_after_s === secs}
            role="radio"
            aria-checked={a.done_after_s === secs}
            disabled={!a.done}
            title={tcl.longTaskTitle}
            onclick={() => settingsData.setAlerts({ done_after_s: secs })}>{secs < 60 ? `${secs} s` : `${secs / 60} min`}</button
          >
        {/each}
      </div>
    </div>
    <div class="rotation" data-anchor="alert-language">
      <span class="rlabel">{tcl.language}</span>
      <div class="rots two" role="radiogroup" aria-label={tcl.languageAria}>
        {#each ALERT_LANGUAGES as [code, label] (code)}
          <button class="rot" class:checked={a.language === code} role="radio" aria-checked={a.language === code} onclick={() => settingsData.setAlerts({ language: code })}>{label}</button>
        {/each}
      </div>
    </div>
    {#if alerts.last}
      <p class="hint">{tcl.lastAlert(ago(alerts.last.age_s))}: “{alerts.last.text}”{alerts.last.error ? tcl.notGiven(alerts.last.error) : ""}</p>
    {/if}
    <div class="actions">
      <button class="btn" disabled={testingAlert || !alerts.hooks.connected || monitor.link !== "connected"} onclick={testAlert}>{tc.tryIt}</button>
    </div>
    {#if alertsError}<p class="hint error">{alertsError}</p>{/if}
  </Section>
{/if}

<Advanced tab="claude">
  <div class="mcp" data-anchor="mcp">
    <h3>{tcl.mcp}&nbsp;<InfoTip text={tcl.mcpTip} /></h3>
    {#if mcp?.hub}
      <dl class="facts">
        <div><dt>{tcl.clients}</dt><dd>{tcl.clientsN(mcp.hub.clients)}</dd></div>
        <div>
          <dt>{tcl.lastCall}</dt>
          <dd>{mcp.hub.last_tool === null || mcp.hub.last_call_age_ms === null ? tcl.noneYet : `${mcp.hub.last_tool}, ${ago(Math.round(mcp.hub.last_call_age_ms / 1000))}`}</dd>
        </div>
        {#if link?.chained}
          <div class="wide"><dt>{tcl.chained}</dt><dd class="small">{link.chained}</dd></div>
        {/if}
      </dl>
    {/if}
    {#if mcpDesktop}
      <p class="hint">{tcl.forDesktop[0]}<code>claude_desktop_config.json</code>{tcl.forDesktop[1]}</p>
      <div class="snippet">
        <pre>{mcpDesktop}</pre>
        <button class="btn" onclick={copyDesktop}>{copied ? tc.copied : tc.copy}</button>
      </div>
    {/if}
  </div>
</Advanced>
