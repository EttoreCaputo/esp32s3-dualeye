<!--
  "Connect to Claude Code": sets up the status line, the hooks and the MCP
  server in one go, then shows what each has seen since. The manual command
  is behind the ⓘ. The Claude tab and the first-run setup show the same one.
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import InfoTip from "../InfoTip.svelte";
  import { monitor } from "../monitor.svelte";
  import { settingsData } from "./data.svelte";
  import Glyph from "./Glyph.svelte";

  const tcl = $derived(i18n.t.settings.claude);
  const tc = $derived(i18n.t.settings.common);
  const link = $derived(settingsData.link);
  const alerts = $derived(settingsData.alerts);
  const mcp = $derived(settingsData.mcp);

  const ago = (s: number) => tc.ago(s);

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
  let copied = $state(false);
  async function copy() {
    await navigator.clipboard.writeText(mcpCommand);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }
</script>

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
      <button class="btn small" onclick={copy}>{copied ? tc.copied : tcl.copyCommand}</button>
    {/if}
  </div>
{:else}
  <p class="empty">{tc.loading}</p>
{/if}
{#if error}<p class="hint error">{error}</p>{/if}
{#if mcp?.hub_error}<p class="hint error">{tcl.hubError(mcp.hub_error)}</p>{/if}
{#if link?.connected && link.last_update_s === null}<p class="hint">{tcl.limitsLater}</p>{/if}
