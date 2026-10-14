<!--
  Setup, step 5 (optional): Connect to Claude Code, the same button as in the
  Claude tab, then which alerts the board gives.
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import ClaudeConnect from "../settings/ClaudeConnect.svelte";
  import { settingsData } from "../settings/data.svelte";
  import NeedRow from "../settings/NeedRow.svelte";
  import { need } from "../settings/requirements";
  import Toggle from "../settings/Toggle.svelte";

  const t = $derived(i18n.t.onboarding.claude);
  const tcl = $derived(i18n.t.settings.claude);
  const alerts = $derived(settingsData.alerts);
  const connected = $derived(!!(settingsData.link?.connected && alerts?.hooks.connected && settingsData.mcp?.registered));

  $effect(() => {
    const load = () => settingsData.loadClaude().catch(() => {});
    load();
    const id = setInterval(load, 3000);
    return () => clearInterval(id);
  });
</script>

<p class="lead">{t.lead}</p>
<section class="claude">
  <ClaudeConnect />
</section>
{#if !connected}
  <p class="hint">{t.noClaude}</p>
{:else if alerts}
  {@const a = alerts.settings}
  <section class="alerts">
    <h3>{t.alerts}</h3>
    <Toggle checked={a.needs_you} label={tcl.needsYou} onchange={(on) => settingsData.setAlerts({ needs_you: on })} />
    <Toggle checked={a.done} label={tcl.done} onchange={(on) => settingsData.setAlerts({ done: on })} />
    <Toggle checked={a.usage} label={tcl.usageAlert} onchange={(on) => settingsData.setAlerts({ usage: on })} />
    <Toggle checked={a.speak} label={tcl.speak} onchange={(on) => settingsData.setAlerts({ speak: on })} />
    <!-- Here only the one-click fix: "Set up" would open the settings behind the setup. -->
    {#if a.speak && need("speak")?.fix.kind === "enable"}<NeedRow need={need("speak")} />{/if}
  </section>
{/if}
