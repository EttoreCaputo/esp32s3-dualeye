<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Eye from "./Eye.svelte";
  import { DEVICES, FACES, ROTATIONS, formatTokens, isClaudeFace, screenFor, type DeviceId, type Face, type Rotation } from "./firmware";
  import { fanRpm, monitor, type ClaudeLink, type FirmwareInfo, type McpInfo, type PortInfo, type Reading } from "./monitor.svelte";

  type Tab = "connection" | "display" | "device" | "sensors" | "console";
  const TABS: [Tab, string][] = [
    ["connection", "Connection"],
    ["display", "Display"],
    ["device", "Device"],
    ["sensors", "Sensors"],
    ["console", "Console"],
  ];

  let { open = $bindable(false), tab = $bindable("connection") }: { open: boolean; tab?: Tab } = $props();

  let ports = $state<PortInfo[]>([]);
  let firmware = $state<FirmwareInfo | null>(null);
  let confirming = $state(false);
  let readings = $state<Reading[]>([]);
  let consoleEl = $state<HTMLElement>();
  let claudeLink = $state<ClaudeLink | null>(null);
  let claudeError = $state("");
  let claudeBusy = $state(false);
  let mcp = $state<McpInfo | null>(null);
  let copied = $state<"code" | "desktop" | null>(null);
  let follow = $state(true);

  $effect(() => {
    if (!open || (tab !== "connection" && tab !== "device")) return;
    const load = () => monitor.listPorts().then((p) => (ports = p));
    load();
    const id = setInterval(load, 2000);
    return () => clearInterval(id);
  });

  $effect(() => {
    if (!open || tab !== "sensors") return;
    let alive = true;
    const load = async () => {
      const r = await monitor.readings();
      if (alive) readings = r;
    };
    load();
    const id = setInterval(load, 2000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  });

  $effect(() => {
    if (!open || tab !== "device") return;
    let alive = true;
    monitor.firmwareInfo().then((f) => alive && (firmware = f));
    return () => {
      alive = false;
      confirming = false;
    };
  });

  $effect(() => {
    if (!open || tab !== "display") return;
    let alive = true;
    const load = () => {
      monitor.claudeLink().then((l) => alive && (claudeLink = l));
      monitor.mcpInfo().then((m) => alive && (mcp = m));
    };
    load();
    const id = setInterval(load, 3000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  });

  async function linkClaude(connect: boolean) {
    claudeBusy = true;
    claudeError = "";
    try {
      claudeLink = await monitor.claudeConnect(connect);
    } catch (e) {
      claudeError = String(e);
    } finally {
      claudeBusy = false;
    }
  }

  // What to paste into Claude Code's terminal and into Claude Desktop's config.
  const shellQuote = (s: string) => (/^[\w@%+=:,./-]+$/.test(s) ? s : s.includes("\\") ? `"${s}"` : `'${s.replaceAll("'", `'\\''`)}'`);
  const mcpCommand = $derived(mcp?.command ? `claude mcp add --scope user dualeye -- ${shellQuote(mcp.command)} ${mcp.args.join(" ")}` : "");
  const mcpDesktop = $derived(
    mcp?.command ? JSON.stringify({ mcpServers: { dualeye: { command: mcp.command, args: mcp.args } } }, null, 2) : "",
  );

  async function copy(which: "code" | "desktop") {
    await navigator.clipboard.writeText(which === "code" ? mcpCommand : mcpDesktop);
    copied = which;
    setTimeout(() => copied === which && (copied = null), 1500);
  }

  const claudeUsage = $derived(monitor.last?.claude);
  const usesClaude = $derived(isClaudeFace(monitor.faces.cpu) || isClaudeFace(monitor.faces.gpu));
  const ago = (s: number) => (s < 60 ? `${s} s ago` : s < 3600 ? `${Math.round(s / 60)} min ago` : `${Math.round(s / 3600)} h ago`);

  // The port esptool will use: the pinned one, else the only Espressif device plugged in.
  const boards = $derived(ports.filter((p) => p.is_board));
  const target = $derived(monitor.portSetting ?? (boards.length === 1 ? boards[0].name : null));
  const busy = $derived(monitor.job !== "idle");

  const onBoard = $derived.by(() => {
    const fw = monitor.boardFirmware;
    if (monitor.link !== "connected") return { text: "—", note: "offline" };
    if (!fw) return { text: "—", note: "asking…" };
    if (fw.state === "version") return { text: `v${fw.version}`, note: fw.idf ? `IDF ${fw.idf}` : "" };
    if (fw.state === "legacy") return { text: "before 0.2.0", note: "unversioned" };
    return { text: "none", note: "blank flash" };
  });

  async function identify() {
    await monitor.identify(monitor.portSetting);
    firmware = await monitor.firmwareInfo();
  }

  async function flash() {
    confirming = false;
    await monitor.flash(monitor.portSetting);
    firmware = await monitor.firmwareInfo();
  }

  $effect(() => {
    void monitor.logs.length;
    if (follow && consoleEl) queueMicrotask(() => consoleEl && (consoleEl.scrollTop = consoleEl.scrollHeight));
  });

  const groups = $derived.by(() => {
    const m = new Map<string, Reading[]>();
    for (const r of readings) m.set(r.source, [...(m.get(r.source) ?? []), r]);
    return [...m.entries()];
  });

  function onKey(e: KeyboardEvent) {
    if (open && e.key === "Escape") open = false;
  }

  function scrolled() {
    if (!consoleEl) return;
    follow = consoleEl.scrollHeight - consoleEl.scrollTop - consoleEl.clientHeight < 24;
  }

  const SCREENS: [DeviceId, string][] = [
    ["cpu", "Left screen"],
    ["gpu", "Right screen"],
  ];
  // Thumbnails use the host's latest sample, so they have data even with no board attached.
  const preview = (id: DeviceId, face: Face) => screenFor(id, face, monitor.last?.[id], false, false, fanRpm(monitor.last, id), monitor.last?.claude);
  const pick = (id: DeviceId, face: Face) => monitor.setFaces({ ...monitor.faces, [id]: face });
  const turn = (id: DeviceId, rotation: Rotation) => monitor.setRotation({ ...monitor.rotation, [id]: rotation });

  const hex = (n: number) => n.toString(16).padStart(4, "0");
  const kb = (n: number) => `${Math.round(n / 1024)} KB`;
  const fmt = (r: Reading) => (r.unit === "RPM" || r.unit === "%" || r.unit === "MB" ? r.value.toFixed(0) : r.value.toFixed(1));
</script>

<svelte:window onkeydown={onKey} />

{#if open}
  <button class="scrim" transition:fade={{ duration: 200 }} onclick={() => (open = false)} aria-label="Close settings"></button>
  <aside class="drawer" transition:fly={{ x: 40, duration: 320, easing: cubicOut, opacity: 0 }} aria-label="Settings">
    <nav>
      {#each TABS as [key, label] (key)}
        <button class:active={tab === key} onclick={() => (tab = key)}>{label}</button>
      {/each}
      <span class="indicator" style:--i={TABS.findIndex(([key]) => key === tab)}></span>
    </nav>

    <div class="content">
      {#if tab === "connection"}
        <section>
          <h3>Serial port</h3>
          <p class="hint">The board is found automatically by its Espressif USB ID. Pin a port only if you have more than one.</p>
          <div class="ports">
            <label class="port" class:checked={monitor.portSetting === null}>
              <input type="radio" name="port" checked={monitor.portSetting === null} onchange={() => monitor.setPort(null)} />
              <span class="radio"></span>
              <span class="pname">Automatic</span>
              <span class="pmeta">{monitor.portSetting === null && monitor.port ? monitor.port : "303a:*"}</span>
            </label>
            {#each ports as p (p.name)}
              <label class="port" class:checked={monitor.portSetting === p.name}>
                <input type="radio" name="port" checked={monitor.portSetting === p.name} onchange={() => monitor.setPort(p.name)} />
                <span class="radio"></span>
                <span class="pname">{p.name}</span>
                <span class="pmeta">{hex(p.vid)}:{hex(p.pid)}{p.is_board ? " · DualEye" : ""}</span>
              </label>
            {:else}
              <p class="empty">No USB serial ports found.</p>
            {/each}
          </div>
        </section>

        {#if monitor.message && monitor.link !== "connected"}
          <section class="alert" class:error={monitor.link === "offline"}>
            <h3>{monitor.link === "offline" ? "Connection lost" : "Waiting"}</h3>
            <p class="mono">{monitor.message}</p>
            {#if monitor.permissionDenied}
              <p class="hint">Linux: add yourself to the <code>dialout</code> group, then log out and back in.</p>
              <pre>sudo usermod -aG dialout "$USER"</pre>
            {/if}
          </section>
        {/if}

        <section>
          <h3>Stream</h3>
          <dl class="facts">
            <div><dt>Rate</dt><dd>1 Hz</dd></div>
            <div><dt>Baud</dt><dd>115200</dd></div>
            <div><dt>Format</dt><dd>JSON line · v1</dd></div>
            <div><dt>Stale after</dt><dd>3 s</dd></div>
          </dl>
        </section>
      {:else if tab === "display"}
        <p class="hint">
          Pick a watch face for each screen, and turn it if the board sits another way round. The board switches with the next
          frame it gets, and the choice is kept.
        </p>
        {#each SCREENS as [id, side] (id)}
          {@const current = monitor.faces[id]}
          <section style:--accent={DEVICES[id].accent}>
            <h3>{side} · {DEVICES[id].title}</h3>
            <div class="faces" role="radiogroup" aria-label="{side} face">
              {#each FACES as face (face.id)}
                <button
                  class="face"
                  class:checked={current === face.id}
                  role="radio"
                  aria-checked={current === face.id}
                  title={face.blurb}
                  onclick={() => pick(id, face.id)}
                >
                  <span class="thumb"><Eye {id} screen={preview(id, face.id)} board="live" size={66} /></span>
                  <span class="fname">{face.name}</span>
                </button>
              {/each}
            </div>
            <p class="fblurb">{FACES.find((f) => f.id === current)?.blurb}</p>
            <div class="rotation">
              <span class="rlabel">Rotation</span>
              <div class="rots" role="radiogroup" aria-label="{side} rotation">
                {#each ROTATIONS as deg (deg)}
                  <button
                    class="rot"
                    class:checked={monitor.rotation[id] === deg}
                    role="radio"
                    aria-checked={monitor.rotation[id] === deg}
                    title={deg === 0 ? "Upright" : `Turned ${deg}° clockwise`}
                    onclick={() => turn(id, deg)}
                  >
                    <svg viewBox="0 0 16 16" aria-hidden="true" style:rotate="{deg}deg">
                      <circle cx="8" cy="8" r="6.2" fill="none" stroke="currentColor" stroke-width="1.3" opacity="0.45" />
                      <path d="M8 3.2 L10.4 6.6 H5.6 Z" fill="currentColor" />
                    </svg>
                    {deg}°
                  </button>
                {/each}
              </div>
            </div>
          </section>
        {/each}

        <section class="claude" class:dimmed={!usesClaude}>
          <h3>Claude Code</h3>
          <p class="hint">
            The Claude faces count tokens from Claude Code's transcripts on this computer.
            {#if claudeUsage}
              This 5-hour window: {formatTokens(claudeUsage.tok)} · today: {formatTokens(claudeUsage.today)}.
            {:else}
              None found yet.
            {/if}
          </p>
          {#if claudeLink?.connected}
            <dl class="facts">
              <div><dt>Status line</dt><dd>Connected</dd></div>
              <div>
                <dt>Last update</dt>
                <dd>{claudeLink.last_update_s === null ? "Waiting" : ago(claudeLink.last_update_s)}</dd>
              </div>
              {#if claudeLink.chained}
                <div class="wide"><dt>Also shows your status line</dt><dd class="small">{claudeLink.chained}</dd></div>
              {/if}
            </dl>
            {#if claudeLink.last_update_s === null}
              <p class="hint">Limits show up after Claude Code's next reply. They're only reported on Pro and Max plans.</p>
            {/if}
            <div class="actions">
              <button class="btn" disabled={claudeBusy} onclick={() => linkClaude(false)}>Disconnect</button>
            </div>
          {:else}
            <p class="hint">
              Connect the status line to add your plan's 5-hour and weekly limits. This sets <code>statusLine</code> in
              <code>{claudeLink?.settings_path ?? "~/.claude/settings.json"}</code>, keeps a backup, and your current status line keeps
              working through it.
            </p>
            <div class="actions">
              <button class="btn primary" disabled={claudeBusy || !claudeLink} onclick={() => linkClaude(true)}>Connect status line</button>
            </div>
          {/if}
          {#if claudeError}
            <p class="hint error">{claudeError}</p>
          {/if}
        </section>

        <section class="mcp">
          <h3>MCP server</h3>
          <p class="hint">
            Let Claude Code, Claude Desktop or another MCP client change the faces, show messages and read this computer's metrics.
            While the app runs they go through it, so the board stays connected here.
          </p>
          {#if mcp?.hub}
            <dl class="facts">
              <div>
                <dt>Clients</dt>
                <dd>{mcp.hub.clients === 0 ? "None connected" : mcp.hub.clients === 1 ? "1 connected" : `${mcp.hub.clients} connected`}</dd>
              </div>
              <div>
                <dt>Last tool call</dt>
                <dd>{mcp.hub.last_tool === null || mcp.hub.last_call_age_ms === null ? "None yet" : `${mcp.hub.last_tool}, ${ago(Math.round(mcp.hub.last_call_age_ms / 1000))}`}</dd>
              </div>
            </dl>
          {:else if mcp?.hub_error}
            <p class="hint error">MCP clients can't reach the board through the app: {mcp.hub_error}</p>
          {/if}
          {#if mcpCommand}
            <p class="hint">Claude Code: run this once in a terminal.</p>
            <div class="snippet">
              <pre>{mcpCommand}</pre>
              <button class="btn" onclick={() => copy("code")}>{copied === "code" ? "Copied" : "Copy"}</button>
            </div>
            <p class="hint">Claude Desktop: add this to <code>claude_desktop_config.json</code> (Settings → Developer → Edit Config), then restart it.</p>
            <div class="snippet">
              <pre>{mcpDesktop}</pre>
              <button class="btn" onclick={() => copy("desktop")}>{copied === "desktop" ? "Copied" : "Copy"}</button>
            </div>
          {/if}
        </section>
      {:else if tab === "device"}
        <section>
          <h3>Board</h3>
          {#if target}
            <p class="hint">
              esptool talks to the ROM bootloader, so this works even on a blank board. Streaming pauses while it runs and the board
              reboots afterwards.
            </p>
            <div class="target">
              <span class="chipdot" class:found={monitor.chip?.port === target}></span>
              <span class="pname">{target}</span>
              <button class="btn" disabled={busy || !firmware} onclick={() => identify()}>
                {monitor.job === "identify" ? "Reading…" : "Identify"}
              </button>
            </div>
          {:else if boards.length > 1}
            <p class="hint">More than one Espressif device is plugged in. Pin the DualEye's port in Connection first.</p>
          {:else}
            <p class="empty">No ESP32-S3 found on USB. Plug the DualEye in with a data cable.</p>
          {/if}
          {#if monitor.chip && monitor.chip.port === target}
            <dl class="facts chipinfo">
              <div class="wide"><dt>Chip</dt><dd>{monitor.chip.chip ?? "—"}</dd></div>
              <div><dt>Flash</dt><dd>{monitor.chip.flash_size ?? "—"}</dd></div>
              <div><dt>Crystal</dt><dd>{monitor.chip.crystal ?? "—"}</dd></div>
              <div class="wide"><dt>MAC</dt><dd>{monitor.chip.mac ?? "—"}</dd></div>
              {#if monitor.chip.features}<div class="wide"><dt>Features</dt><dd class="small">{monitor.chip.features}</dd></div>{/if}
            </dl>
          {/if}
        </section>

        <section>
          <h3>Firmware</h3>
          {#if !firmware}
            <p class="empty">Checking esptool…</p>
          {:else}
            <dl class="facts">
              <div><dt>On the board</dt><dd>{onBoard.text} {#if onBoard.note}<span class="muted sub">{onBoard.note}</span>{/if}</dd></div>
              <div>
                <dt>In the app</dt>
                <dd>
                  {firmware.bundled ? `v${firmware.bundled.version}` : "—"}
                  {#if monitor.update}<span class="muted sub new">newer</span>{:else if firmware.bundled}<span class="muted sub">IDF {firmware.bundled.idf}</span>{/if}
                </dd>
              </div>
              <div><dt>Image</dt><dd>merged-binary.bin</dd></div>
              <div><dt>Size</dt><dd>{kb(firmware.size)} · at 0x0</dd></div>
              <div class="wide">
                <dt>esptool</dt>
                {#if firmware.esptool}
                  <dd>v{firmware.esptool.version} <span class="muted">ready</span></dd>
                {:else}
                  <dd class="small">Set up automatically on first use: about 50 MB download, needs internet once.</dd>
                {/if}
              </div>
            </dl>
            {#if monitor.update && monitor.update.changes.length && monitor.job !== "flash"}
              <div class="changes">
                <p class="hint">Updating to v{monitor.update.to} brings:</p>
                {#each monitor.update.changes as release (release.version)}
                  <div class="release">
                    <span class="ver">v{release.version}</span>
                    <ul>
                      {#each release.notes as note, i (i)}<li>{note}</li>{/each}
                    </ul>
                  </div>
                {/each}
              </div>
            {/if}
            {#if monitor.setup}
              <div class="progress" class:indeterminate={monitor.setup.percent === null}>
                <span style:width="{monitor.setup.percent ?? 30}%"></span>
              </div>
              <p class="hint setup">Preparing esptool · {monitor.setup.message}{monitor.setup.percent !== null ? ` ${Math.round(monitor.setup.percent)}%` : ""}</p>
            {:else if monitor.job === "flash"}
              <div class="progress" role="progressbar" aria-valuenow={Math.round(monitor.flashPercent)} aria-valuemin={0} aria-valuemax={100}>
                <span style:width="{monitor.flashPercent}%"></span>
              </div>
              <p class="hint">Writing {Math.round(monitor.flashPercent)}% · don't unplug the board.</p>
            {:else if confirming}
              <p class="hint">This replaces the firmware on {target}. Continue?</p>
              <div class="actions">
                <button class="btn primary" onclick={flash}>Flash now</button>
                <button class="btn" onclick={() => (confirming = false)}>Cancel</button>
              </div>
            {:else}
              <div class="actions">
                <button class="btn primary" disabled={busy || !target} onclick={() => (confirming = true)}>
                  {monitor.update ? `Update to v${monitor.update.to}` : "Flash firmware"}
                </button>
              </div>
            {/if}
          {/if}
          {#if monitor.jobError}
            <section class="alert error">
              <h3>esptool failed</h3>
              <p class="mono">{monitor.jobError}</p>
              {#if /busy|Errno 16/i.test(monitor.jobError)}
                <p class="hint">Another program has the port open: a second DualEye window, <code>idf.py monitor</code> or the <code>dualeye</code> CLI. Close it and try again.</p>
              {/if}
            </section>
          {:else if monitor.flashedAt && monitor.job === "idle"}
            <p class="ok">Flashed and verified. The board is rebooting into the new firmware.</p>
          {/if}
          {#if monitor.jobLog.length}
            <details open={busy || !!monitor.jobError}>
              <summary>esptool output</summary>
              <div class="console joblog">
                {#each monitor.jobLog as line, i (i)}<div class="log">{line}</div>{/each}
              </div>
            </details>
          {/if}
        </section>
      {:else if tab === "sensors"}
        <p class="hint">Every raw value the host can read. The board gets the CPU average, the first GPU, the fastest fan of each kind, RAM and VRAM.</p>
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
          <p class="empty">Reading sensors…</p>
        {/each}
      {:else}
        <div class="console" bind:this={consoleEl} onscroll={scrolled}>
          {#each monitor.logs as line, i (i)}
            <div class="log" class:warn={line.startsWith("W ")} class:err={line.startsWith("E ")}>{line}</div>
          {:else}
            <p class="empty">Nothing logged by the board yet.</p>
          {/each}
        </div>
      {/if}
    </div>
  </aside>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    border: 0;
    background: rgba(0, 0, 0, 0.45);
    backdrop-filter: blur(3px);
    cursor: default;
  }
  .drawer {
    position: fixed;
    z-index: 50;
    top: 10px;
    right: 10px;
    bottom: 10px;
    width: min(400px, calc(100vw - 20px));
    display: flex;
    flex-direction: column;
    border-radius: 20px;
    background: rgba(17, 18, 21, 0.86);
    backdrop-filter: blur(28px) saturate(1.4);
    border: 1px solid rgba(255, 255, 255, 0.09);
    box-shadow:
      0 40px 80px -20px rgba(0, 0, 0, 0.8),
      inset 0 1px 0 rgba(255, 255, 255, 0.06);
    overflow: hidden;
  }

  nav {
    position: relative;
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    margin: 14px;
    padding: 3px;
    border-radius: 12px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--line);
  }
  nav button {
    position: relative;
    z-index: 1;
    height: 30px;
    border: 0;
    background: none;
    color: var(--dim);
    font: 550 12px/1 var(--sans);
    cursor: pointer;
    transition: color 200ms;
  }
  nav button.active {
    color: var(--text);
  }
  .indicator {
    position: absolute;
    top: 3px;
    bottom: 3px;
    left: 3px;
    width: calc((100% - 6px) / 5);
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.08);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.06);
    transform: translateX(calc(var(--i) * 100%));
    transition: transform 320ms cubic-bezier(0.3, 0.8, 0.2, 1);
  }

  .content {
    flex: 1;
    overflow-y: auto;
    padding: 4px 18px 18px;
    display: flex;
    flex-direction: column;
    gap: 22px;
    min-height: 0;
  }
  h3 {
    margin: 0 0 8px;
    font: 600 10px/1 var(--mono);
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--faint);
  }
  .hint {
    margin: 0 0 12px;
    font: 400 12.5px/1.5 var(--sans);
    color: var(--dim);
  }
  .empty {
    color: var(--faint);
    font: 400 12.5px/1.5 var(--sans);
    margin: 0;
  }
  .mono,
  pre,
  code {
    font-family: var(--mono);
    font-size: 11.5px;
  }
  pre {
    margin: 0;
    padding: 10px 12px;
    border-radius: 10px;
    background: rgba(0, 0, 0, 0.4);
    border: 1px solid var(--line);
    color: var(--text);
    user-select: text;
  }

  .ports {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .port {
    display: grid;
    grid-template-columns: 18px 1fr auto;
    align-items: center;
    gap: 10px;
    padding: 11px 12px;
    border-radius: 12px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.02);
    cursor: pointer;
    transition:
      border-color 200ms,
      background 200ms;
  }
  .port:hover {
    background: rgba(255, 255, 255, 0.04);
  }
  .port.checked {
    border-color: color-mix(in srgb, var(--cpu) 45%, transparent);
    background: color-mix(in srgb, var(--cpu) 6%, transparent);
  }
  .port input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }
  .port:has(input:focus-visible) {
    outline: 1.5px solid var(--cpu);
  }
  .radio {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 1.5px solid var(--faint);
    transition: all 200ms;
  }
  .port.checked .radio {
    border: 4px solid var(--cpu);
  }
  .pname {
    font: 550 13px/1.2 var(--sans);
  }
  .pmeta {
    font: 500 10.5px/1 var(--mono);
    color: var(--faint);
  }

  .faces {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }
  .face {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 10px 0 9px;
    border-radius: 12px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.02);
    color: var(--dim);
    cursor: pointer;
    transition:
      border-color 200ms,
      background 200ms,
      color 200ms;
  }
  .face:hover {
    background: rgba(255, 255, 255, 0.04);
  }
  .face.checked {
    border-color: color-mix(in srgb, var(--accent) 50%, transparent);
    background: color-mix(in srgb, var(--accent) 7%, transparent);
    color: var(--text);
  }
  .face:focus-visible {
    outline: 1.5px solid var(--accent);
  }
  .thumb {
    border-radius: 50%;
    line-height: 0;
    box-shadow:
      0 0 0 1px #000,
      0 0 0 2px rgba(255, 255, 255, 0.08);
  }
  .fname {
    font: 550 11.5px/1 var(--sans);
  }
  .fblurb {
    margin: 10px 0 0;
    font: 400 12px/1.4 var(--sans);
    color: var(--faint);
  }

  .rotation {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-top: 12px;
  }
  .rlabel {
    font: 550 12px/1 var(--sans);
    color: var(--dim);
  }
  .rots {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 3px;
    padding: 3px;
    border-radius: 10px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.02);
  }
  .rot {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 5px 9px;
    border: 0;
    border-radius: 7px;
    background: none;
    color: var(--faint);
    font: 550 11px/1 var(--mono);
    cursor: pointer;
    transition:
      background 200ms,
      color 200ms;
  }
  .rot svg {
    width: 12px;
    height: 12px;
  }
  .rot:hover {
    color: var(--dim);
  }
  .rot.checked {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--text);
  }
  .rot:focus-visible {
    outline: 1.5px solid var(--accent);
  }

  .claude.dimmed {
    opacity: 0.7;
  }
  .claude .facts {
    margin-bottom: 12px;
  }
  .hint.error {
    color: var(--hot);
  }
  .mcp .facts {
    margin-bottom: 12px;
  }
  .mcp .hint + .snippet {
    margin-top: 8px;
  }
  .snippet + .hint {
    margin-top: 12px;
  }
  .snippet {
    position: relative;
  }
  .snippet pre {
    padding-right: 72px;
    white-space: pre-wrap;
    word-break: break-all;
    user-select: text;
  }
  .snippet .btn {
    position: absolute;
    top: 6px;
    right: 6px;
    height: 24px;
    padding: 0 10px;
    font-size: 11px;
  }

  .alert {
    padding: 14px;
    border-radius: 12px;
    background: color-mix(in srgb, var(--stale) 7%, transparent);
    border: 1px solid color-mix(in srgb, var(--stale) 22%, transparent);
  }
  .alert.error {
    background: color-mix(in srgb, var(--hot) 8%, transparent);
    border-color: color-mix(in srgb, var(--hot) 28%, transparent);
  }
  .alert p {
    margin: 0 0 10px;
    color: var(--text);
  }

  .facts {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1px;
    margin: 0;
    border-radius: 12px;
    overflow: hidden;
    border: 1px solid var(--line);
    background: var(--line);
  }
  .facts div {
    padding: 11px 12px;
    background: #121316;
  }
  .facts dt {
    font: 500 10.5px/1 var(--sans);
    color: var(--faint);
    margin-bottom: 6px;
  }
  .facts dd {
    margin: 0;
    font: 550 12.5px/1 var(--mono);
  }

  .facts .wide {
    grid-column: 1 / -1;
  }
  .facts dd.small {
    font-size: 11px;
    line-height: 1.5;
  }
  .chipinfo {
    margin-top: 10px;
  }
  .muted {
    color: var(--faint);
    font-size: 10.5px;
    margin-left: 6px;
    word-break: break-all;
  }
  .muted.sub {
    display: block;
    margin: 5px 0 0;
  }
  .muted.new {
    color: var(--cpu);
  }
  .changes {
    margin-top: 14px;
    display: grid;
    gap: 10px;
  }
  .changes .hint {
    margin: 0;
  }
  .release {
    display: grid;
    grid-template-columns: 48px 1fr;
    gap: 8px;
  }
  .ver {
    font: 500 11px/1.6 var(--mono);
    color: var(--cpu);
  }
  .release ul {
    margin: 0;
    padding-left: 16px;
    font: 400 12.5px/1.55 var(--sans);
    color: var(--dim);
  }

  .target {
    display: grid;
    grid-template-columns: 14px 1fr auto;
    align-items: center;
    gap: 10px;
    padding: 8px 8px 8px 12px;
    border-radius: 12px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.02);
  }
  .chipdot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--faint);
  }
  .chipdot.found {
    background: #5ee38a;
    box-shadow: 0 0 8px #5ee38a;
  }
  .actions {
    display: flex;
    gap: 8px;
    margin-top: 12px;
  }
  .btn {
    height: 30px;
    padding: 0 14px;
    border-radius: 9px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.05);
    color: var(--text);
    font: 550 12px/1 var(--sans);
    cursor: pointer;
    transition: background 200ms;
  }
  .btn:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.09);
  }
  .btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .btn.primary {
    border-color: color-mix(in srgb, var(--cpu) 45%, transparent);
    background: color-mix(in srgb, var(--cpu) 14%, transparent);
  }
  .btn.primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--cpu) 22%, transparent);
  }
  .progress {
    height: 6px;
    margin: 14px 0 8px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.06);
    overflow: hidden;
  }
  .progress span {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--cpu);
    box-shadow: 0 0 10px var(--cpu);
    transition: width 200ms linear;
  }
  .progress.indeterminate span {
    animation: sweep 1.2s ease-in-out infinite;
  }
  @keyframes sweep {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(340%);
    }
  }
  .hint.setup {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ok {
    margin: 12px 0 0;
    font: 450 12.5px/1.5 var(--sans);
    color: #5ee38a;
  }
  details {
    margin-top: 14px;
  }
  summary {
    font: 500 11px/1 var(--sans);
    color: var(--dim);
    cursor: pointer;
    margin-bottom: 8px;
  }
  .joblog {
    max-height: 180px;
    margin: 0;
  }

  .group ul {
    list-style: none;
    margin: 0;
    padding: 0;
    border-radius: 12px;
    border: 1px solid var(--line);
    overflow: hidden;
  }
  .group li {
    display: flex;
    justify-content: space-between;
    padding: 8px 12px;
    font: 450 12.5px/1.3 var(--sans);
    color: var(--dim);
  }
  .group li:nth-child(odd) {
    background: rgba(255, 255, 255, 0.02);
  }
  .group b {
    font: 550 12px/1.3 var(--mono);
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .group small {
    color: var(--faint);
    margin-left: 4px;
    font-weight: 500;
  }

  .console {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin: 0 -6px;
    padding: 10px 12px;
    border-radius: 12px;
    background: rgba(0, 0, 0, 0.45);
    border: 1px solid var(--line);
    font: 450 11px/1.65 var(--mono);
    color: #b9bdc6;
    user-select: text;
  }
  .log {
    white-space: pre-wrap;
    word-break: break-all;
  }
  .log.warn {
    color: var(--stale);
  }
  .log.err {
    color: var(--hot);
  }
</style>
