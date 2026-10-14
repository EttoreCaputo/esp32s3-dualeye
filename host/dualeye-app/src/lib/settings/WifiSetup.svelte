<!--
  The board's WiFi: where it stands, finding a network and joining it,
  pairing with this computer, forgetting it. The Device tab and the first-run
  setup show the same one.
-->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import { monitor, type WifiInfo, type WifiNetwork, type WifiStatus } from "../monitor.svelte";

  let { wifi = $bindable(null) }: { wifi?: WifiInfo | null } = $props();

  const td = $derived(i18n.t.settings.device);
  const tc = $derived(i18n.t.settings.common);

  let wifiBusy = $state<"" | "scan" | "join" | "toggle" | "forget" | "pair">("");
  let wifiError = $state("");
  let wifiForm = $state(false);
  let networks = $state<WifiNetwork[] | null>(null);
  let wifiSsid = $state("");
  let wifiPassword = $state("");
  let forgetting = $state(false);

  const loadWifi = () =>
    monitor
      .wifiInfo()
      .then((w) => (wifi = w))
      .catch(() => {});
  $effect(() => {
    loadWifi();
    const id = setInterval(loadWifi, 3000);
    return () => clearInterval(id);
  });

  const onWifi = $derived(monitor.link === "connected" && !!monitor.port?.startsWith("WiFi"));
  const pairedHere = $derived(!!wifi?.board && wifi.paired.some((p) => p.id === wifi?.board?.id));

  function wifiLabel(b: WifiStatus): string {
    const net = b.ssid ? `“${b.ssid}”` : "";
    const say = td.state;
    switch (b.state) {
      case "off":
        return b.ssid ? say.off(net) : say.notSetUp;
      case "connecting":
        return say.joining(net);
      case "connected":
        return say.on(net);
      case "no_network":
        return say.notFound(net);
      case "wrong_password":
        return say.wrongPassword(net);
      default:
        return say.reconnecting(net);
    }
  }

  /** 0 to 3 bars from the board's signal strength. */
  const bars = (rssi?: number) => (rssi === undefined ? "" : rssi > -60 ? "▂▄▆" : rssi > -70 ? "▂▄" : "▂");

  async function wifiDo(what: typeof wifiBusy, job: () => Promise<unknown>) {
    wifiBusy = what;
    wifiError = "";
    try {
      await job();
      await loadWifi();
    } catch (e) {
      wifiError = String(e);
    } finally {
      wifiBusy = "";
    }
  }
  const scanWifi = () =>
    wifiDo("scan", async () => {
      networks = await monitor.wifiScan();
      if (!wifiSsid && networks.length) wifiSsid = networks[0].ssid;
    });
  const joinWifi = () =>
    wifiDo("join", async () => {
      await monitor.wifiSetup(wifiSsid.trim(), wifiPassword);
      wifiPassword = "";
      wifiForm = false;
      networks = null;
    });
</script>

{#if !wifi}
  <p class="empty">{td.asking}</p>
{:else if !wifi.board}
  <p class="empty">{wifi.error ?? td.noAnswer}</p>
{:else}
  {@const b = wifi.board}
  <div class="wstatus">
    <span class="wdot {b.state}"></span>
    <span class="pname">{wifiLabel(b)}</span>
    <span class="pmeta">{b.ip ? `${b.ip} ${bars(b.rssi)}` : b.host}</span>
  </div>
  {#if b.ssid}
    <label class="switch">
      <input type="checkbox" checked={b.enabled} disabled={!!wifiBusy || onWifi} onchange={(e) => wifiDo("toggle", () => monitor.wifiEnable(e.currentTarget.checked))} />
      <span class="track"><span class="knob"></span></span>
      <span class="slabel">{td.wifiSwitch}</span>
    </label>
  {/if}
  {#if b.paired && !pairedHere}
    <p class="hint">{td.otherComputer}</p>
    <div class="actions">
      <button class="btn" disabled={!!wifiBusy || onWifi} onclick={() => wifiDo("pair", () => monitor.wifiPair())}>{wifiBusy === "pair" ? td.pairing : td.pair}</button>
    </div>
  {/if}
  {#if wifiForm || !b.ssid}
    {#if onWifi}
      <p class="hint error">{td.plugUsb}</p>
    {:else}
      <div class="actions">
        <button class="btn" disabled={!!wifiBusy} onclick={scanWifi}>{wifiBusy === "scan" ? td.looking : networks ? td.lookAgain : td.find}</button>
        {#if b.ssid}<button class="btn" onclick={() => ((wifiForm = false), (networks = null))}>{tc.cancel}</button>{/if}
      </div>
      {#if networks}
        <div class="ports nets">
          {#each networks as n (n.ssid)}
            <label class="port" class:checked={wifiSsid === n.ssid}>
              <input type="radio" name="ssid" checked={wifiSsid === n.ssid} onchange={() => (wifiSsid = n.ssid)} />
              <span class="radio"></span>
              <span class="pname">{n.ssid}</span>
              <span class="pmeta">{n.secure ? "🔒 " : ""}{bars(n.rssi)}</span>
            </label>
          {:else}
            <p class="empty">{td.noNetwork}</p>
          {/each}
        </div>
      {/if}
      <div class="tform">
        <input class="ttext" type="text" maxlength="32" spellcheck="false" placeholder={td.networkName} aria-label={td.networkName} bind:value={wifiSsid} />
        <input class="ttext" type="password" maxlength="64" autocomplete="off" placeholder={td.password} aria-label={td.passwordAria} bind:value={wifiPassword} />
        <button class="btn small primary" disabled={!!wifiBusy || !wifiSsid.trim() || (wifiPassword.length > 0 && wifiPassword.length < 8)} onclick={joinWifi}
          >{wifiBusy === "join" ? td.sending : td.connect}</button
        >
      </div>
    {/if}
  {:else if forgetting}
    <p class="hint">{td.forgetConfirm(b.ssid)}</p>
    <div class="actions">
      <button class="btn primary" onclick={() => ((forgetting = false), wifiDo("forget", () => monitor.wifiForget()))}>{td.forget}</button>
      <button class="btn" onclick={() => (forgetting = false)}>{tc.cancel}</button>
    </div>
  {:else}
    <div class="actions">
      <button class="btn" disabled={!!wifiBusy || onWifi} onclick={() => (wifiForm = true)}>{td.change}</button>
      <button class="btn" disabled={!!wifiBusy || onWifi} onclick={() => (forgetting = true)}>{td.forgetNetwork}</button>
    </div>
  {/if}
{/if}
{#if wifiError}<p class="hint error">{wifiError}</p>{/if}
