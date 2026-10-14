<script lang="ts">
  import { formatTokens, isClaudeFace } from "../firmware";
  import { i18n } from "../i18n/index.svelte";
  import InfoTip from "../InfoTip.svelte";
  import { monitor } from "../monitor.svelte";
  import Advanced from "./Advanced.svelte";
  import { settingsData } from "./data.svelte";
  import Glyph from "./Glyph.svelte";
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

  /** The three things "Connect" sets up in Claude Code, and what each has seen since. */
  const pieces = $derived(
    link && alerts && mcp
      ? [
          {
            id: "statusline",
            name: tcl.pieces.statusline,
            tip: tcl.pieces.statuslineTip,
            on: link.connected,
            live: link.last_update_s === null ? tcl.waitingReply : tcl.updated(ago(link.last_update_s)),
          },
          {
            id: "hooks",
            name: tcl.pieces.hooks,
            tip: tcl.pieces.hooksTip,
            on: alerts.hooks.connected,
            live: alerts.hooks.last_event_s === null ? tcl.waitingReply : tcl.updated(ago(alerts.hooks.last_event_s)),
          },
          {
            id: "mcp",
            name: tcl.pieces.mcp,
            tip: tcl.pieces.mcpTip,
            on: mcp.registered,
            live: mcp.hub?.clients ? tcl.clientsN(mcp.hub.clients) : tcl.restart,
          },
        ]
      : null,
  );
  const allOn = $derived(!!pieces?.every((p) => p.on));
  const anyOn = $derived(!!pieces?.some((p) => p.on));

  let busy = $state(false);
  let error = $state("");

  /** Set up (or take out) the status line, the hooks and the MCP server, then read them back. */
  async function connect(on: boolean) {
    busy = true;
    error = "";
    const errors: string[] = [];
    const steps: [boolean | undefined, () => Promise<unknown>][] = [
      [link?.connected, () => monitor.claudeConnect(on)],
      [alerts?.hooks.connected, () => monitor.claudeHooks(on)],
      [mcp?.registered, () => monitor.claudeMcp(on)],
    ];
    for (const [now, step] of steps) {
      if (now === on) continue;
      await step().catch((e) => errors.push(String(e)));
    }
    error = errors.join("\n");
    await settingsData.loadClaude().catch(() => {});
    busy = false;
  }

  // The manual way, for those who'd rather paste a command.
  const shellQuote = (s: string) => (/^[\w@%+=:,./-]+$/.test(s) ? s : s.includes("\\") ? `"${s}"` : `'${s.replaceAll("'", `'\\''`)}'`);
  const mcpCommand = $derived(mcp?.command ? `claude mcp add --scope user dualeye -- ${shellQuote(mcp.command)} ${mcp.args.join(" ")}` : "");
  const mcpDesktop = $derived(mcp?.command ? JSON.stringify({ mcpServers: { dualeye: { command: mcp.command, args: mcp.args } } }, null, 2) : "");
  let copied = $state<"code" | "desktop" | null>(null);
  async function copy(which: "code" | "desktop") {
    await navigator.clipboard.writeText(which === "code" ? mcpCommand : mcpDesktop);
    copied = which;
    setTimeout(() => copied === which && (copied = null), 1500);
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
  {#if pieces}
    {#if anyOn}
      <div class="statuscard">
        {#each pieces as p (p.id)}
          <div class="srow">
            <Glyph state={p.on ? "ok" : "off"} />
            <span class="sname">{p.name}&nbsp;<InfoTip text={p.tip} /></span>
            <span class="sstate">{p.on ? p.live : tcl.notConnected}</span>
          </div>
        {/each}
      </div>
    {/if}
    <div class="actions">
      {#if !allOn}
        <button class="btn primary" disabled={busy} onclick={() => connect(true)}>{busy ? tcl.connecting : anyOn ? tcl.repair : tcl.connect}</button>
      {/if}
      {#if anyOn}
        <button class="btn" disabled={busy} onclick={() => connect(false)}>{tc.disconnect}</button>
      {/if}
      <InfoTip label={tcl.manualLabel}>
        {tcl.connectTip(link?.settings_path ?? "~/.claude/settings.json", mcp?.config_path ?? "~/.claude.json")}
        {#if mcpCommand}<br /><br />{tcl.manual}<br /><code>{mcpCommand}</code>{/if}
      </InfoTip>
      {#if mcpCommand}
        <button class="btn small" onclick={() => copy("code")}>{copied === "code" ? tc.copied : tcl.copyCommand}</button>
      {/if}
    </div>
  {:else}
    <p class="empty">{tc.loading}</p>
  {/if}
  {#if error}<p class="hint error">{error}</p>{/if}
  {#if mcp?.hub_error}<p class="hint error">{tcl.hubError(mcp.hub_error)}</p>{/if}
  {#if link?.connected && link.last_update_s === null}<p class="hint">{tcl.limitsLater}</p>{/if}
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
        <button class="btn" onclick={() => copy("desktop")}>{copied === "desktop" ? tc.copied : tc.copy}</button>
      </div>
    {/if}
  </div>
</Advanced>
