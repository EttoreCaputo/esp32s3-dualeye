<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import Eye from "./Eye.svelte";
  import { DEVICES, FACES, ROTATIONS, formatTokens, isClaudeFace, screenFor, type DeviceId, type Face, type Rotation } from "./firmware";
  import {
    fanRpm,
    monitor,
    type ClaudeLink,
    type FirmwareInfo,
    type Hardware,
    type McpInfo,
    type PortInfo,
    type Reading,
    type SttLanguage,
    type VoiceInfo,
    type VoiceSettings,
  } from "./monitor.svelte";

  type Tab = "connection" | "display" | "voice" | "device" | "sensors" | "console";
  const TABS: [Tab, string][] = [
    ["connection", "Connection"],
    ["display", "Display"],
    ["voice", "Voice"],
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

  let voice = $state<VoiceInfo | null>(null);
  let voiceError = $state("");

  $effect(() => {
    if (!open || tab !== "voice") return;
    let alive = true;
    const load = () => monitor.voiceInfo().then((v) => alive && (voice = v));
    load();
    // Faster while a model downloads, for its progress bar.
    const id = setInterval(load, 700);
    return () => {
      alive = false;
      clearInterval(id);
    };
  });

  // The speaker's volume lives on the board: read it when the tab opens.
  let volume = $state<number | null>(null);
  $effect(() => {
    if (!open || tab !== "voice" || monitor.link !== "connected") return;
    let alive = true;
    monitor
      .boardVolume()
      .then((v) => alive && (volume = v))
      .catch(() => alive && (volume = null));
    return () => {
      alive = false;
    };
  });

  async function setVolume(percent: number) {
    volume = percent;
    try {
      await monitor.setBoardVolume(percent);
    } catch (e) {
      voiceError = String(e);
    }
  }

  let piperError = $state("");
  async function installPiper() {
    piperError = "";
    try {
      await monitor.installPiper();
    } catch (e) {
      piperError = String(e);
    }
    voice = await monitor.voiceInfo();
  }

  let testing = $state<string | null>(null);
  async function testVoice(language: string) {
    testing = language;
    voiceError = "";
    try {
      await monitor.testVoice(language);
    } catch (e) {
      voiceError = String(e);
    } finally {
      testing = null;
    }
  }

  const VOICE_LANGUAGES: [string, string][] = [
    ["it", "Italiano"],
    ["en", "English"],
  ];

  async function setVoice(change: Partial<VoiceSettings>) {
    if (!voice) return;
    voiceError = "";
    voice = await monitor.setVoice({ ...voice.settings, ...change });
  }

  async function download(id: string) {
    voiceError = "";
    try {
      await monitor.downloadModel(id);
    } catch (e) {
      if (!/cancel/i.test(String(e))) voiceError = String(e);
    }
    voice = await monitor.voiceInfo();
  }

  const gb = (mbytes: number) => `${Math.round(mbytes / 1024)} GB`;
  function hardwareLine(hw: Hardware): string {
    const gpu = hw.gpu.kind === "apple" ? "Apple silicon GPU" : hw.gpu.kind === "nvidia" ? `${hw.gpu.name} (${gb(hw.gpu.memory_mb)})` : "no GPU the models can use";
    return `${hw.cpu}, ${hw.cores} cores, ${gb(hw.memory_mb)}, ${gpu}`;
  }

  function usesRecommended(v: VoiceInfo): boolean {
    const r = v.recommendation;
    return v.settings.model === r.whisper && (r.llm ? v.settings.llm && v.settings.llm_model === r.llm : !v.settings.llm);
  }

  /** Pick the models recommended for this computer, and download those missing. */
  async function useRecommended() {
    if (!voice) return;
    const r = voice.recommendation;
    await setVoice(r.llm ? { model: r.whisper, llm: true, llm_model: r.llm } : { model: r.whisper, llm: false });
    for (const id of [r.whisper, r.llm]) {
      if (id && voice && !voice.models.find((m) => m.id === id)?.installed) await download(id);
    }
  }

  async function removeModel(id: string) {
    voiceError = "";
    try {
      voice = await monitor.deleteModel(id);
    } catch (e) {
      voiceError = String(e);
    }
  }

  const LANGUAGES: [SttLanguage, string][] = [
    ["auto", "Auto"],
    ["it", "Italiano"],
    ["en", "English"],
  ];
  const STT_STATUS = { off: "Off", starting: "Loading the model…", ready: "Ready", error: "Not working" };
  const TTS_STATUS = { off: "Off", starting: "Loading the voices…", ready: "Ready", error: "Not working" };
  const LLM_STATUS = { off: "Off", starting: "Loading the model…", ready: "Ready", error: "Not working" };
  const mb = (n: number) => `${Math.round(n / 1_000_000)} MB`;
  const clock = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  const newestFirst = $derived([...monitor.transcripts].reverse());

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
      {:else if tab === "voice"}
        <p class="hint">
          After its wake word, <b>“Alexa”</b>, the board sends what you say to this computer. With voice on, whisper.cpp transcribes it
          here, in Italian or English, a small language model works out what to do (“metti la faccia rings a sinistra”, “what's the
          temperature?”) and Piper answers through the board's speaker. Nothing leaves the computer.
        </p>
        {#if !voice}
          <p class="empty">Loading…</p>
        {:else}
          <section class="voice">
            <h3>Voice</h3>
            <label class="switch">
              <input type="checkbox" checked={voice.settings.enabled} onchange={(e) => setVoice({ enabled: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">Transcribe what the board hears</span>
            </label>
            <label class="switch">
              <input type="checkbox" checked={voice.settings.speak} onchange={(e) => setVoice({ speak: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">Answer out loud</span>
            </label>
            <label class="switch">
              <input
                type="checkbox"
                checked={voice.settings.follow_up}
                disabled={!voice.settings.speak}
                onchange={(e) => setVoice({ follow_up: e.currentTarget.checked })}
              />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">Keep listening after an answer, without the wake word</span>
            </label>
            <label class="switch">
              <input type="checkbox" checked={voice.settings.llm} onchange={(e) => setVoice({ llm: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">Understand with a language model</span>
            </label>
            <dl class="facts">
              <div>
                <dt>Speech-to-text</dt>
                <dd class:good={voice.stt === "ready"} class:bad={voice.stt === "error"}>{STT_STATUS[voice.stt]}</dd>
              </div>
              <div>
                <dt>Text-to-speech</dt>
                <dd class:good={voice.tts === "ready"} class:bad={voice.tts === "error"}>{TTS_STATUS[voice.tts]}</dd>
              </div>
              <div>
                <dt>Language model</dt>
                <dd class:good={voice.llm === "ready"} class:bad={voice.llm === "error"}>{LLM_STATUS[voice.llm]}</dd>
              </div>
              <div><dt>Board</dt><dd>{monitor.link === "connected" ? monitor.voice : "offline"}</dd></div>
            </dl>
            {#if voice.stt === "error" && voice.stt_error}
              <p class="hint error">{voice.stt_error}</p>
            {/if}
            {#if voice.tts === "error" && voice.tts_error}
              <p class="hint error">{voice.tts_error}</p>
            {/if}
            {#if voice.llm === "error" && voice.llm_error}
              <p class="hint error">{voice.llm_error}. Meanwhile a few fixed phrases work.</p>
            {/if}
            <div class="volume">
              <span class="rlabel">Volume</span>
              <input
                type="range"
                min="0"
                max="100"
                step="5"
                value={volume ?? 60}
                disabled={volume === null}
                aria-label="Speaker volume"
                onchange={(e) => setVolume(Number(e.currentTarget.value))}
              />
              <span class="pmeta">{volume === null ? "—" : `${volume}%`}</span>
            </div>
            <div class="rotation">
              <span class="rlabel">Language</span>
              <div class="rots langs" role="radiogroup" aria-label="Language">
                {#each LANGUAGES as [id, label] (id)}
                  <button
                    class="rot"
                    class:checked={voice.settings.language === id}
                    role="radio"
                    aria-checked={voice.settings.language === id}
                    title={id === "auto" ? "Whisper tells Italian from English" : `Always ${label}`}
                    onclick={() => setVoice({ language: id })}>{label}</button
                  >
                {/each}
              </div>
            </div>
            <div class="hwbox">
              <span class="rlabel">This computer</span>
              <p class="hint">{hardwareLine(voice.hardware)}. {voice.recommendation.why}</p>
              {#if !usesRecommended(voice)}
                <button class="btn small" onclick={useRecommended}>
                  Use {voice.recommendation.whisper}{voice.recommendation.llm ? ` and ${voice.recommendation.llm}` : " without a language model"}
                </button>
              {/if}
            </div>
            <label class="check">
              <input type="checkbox" checked={voice.settings.keep_recordings} onchange={(e) => setVoice({ keep_recordings: e.currentTarget.checked })} />
              <span>Keep recordings as WAV files, for debugging</span>
            </label>
          </section>

          <section>
            <h3>Speech model</h3>
            {#if !voice.server}
              <p class="hint error">
                whisper-server wasn't found. The app's installer comes with it; for a build of your own, run
                <code>tools/build_sidecars.sh</code> or install whisper.cpp (<code>brew install whisper-cpp</code> on macOS).
              </p>
            {:else}
              <p class="hint">Downloaded from Hugging Face once and checked. whisper-server: <code>{voice.server}</code></p>
            {/if}
            <div class="ports">
              {#each voice.models.filter((m) => m.kind === "whisper") as m (m.id)}
                {@const downloading = voice.download?.[0] === m.id}
                <div class="port model" class:checked={voice.settings.model === m.id}>
                  <label class="mpick">
                    <input type="radio" name="model" checked={voice.settings.model === m.id} onchange={() => setVoice({ model: m.id })} />
                    <span class="radio"></span>
                    <span class="mtext">
                      <span class="pname">{m.id}{#if m.id === voice.recommendation.whisper || m.id === voice.recommendation.llm}<span class="rec" title="What this computer runs best">recommended</span>{/if}</span>
                      <span class="mnote">{m.note}</span>
                    </span>
                  </label>
                  <span class="mside">
                    <span class="pmeta">{mb(m.bytes)}</span>
                    {#if downloading}
                      <button class="btn small" onclick={() => monitor.cancelDownload()}>{Math.round(voice.download?.[1] ?? 0)}% · Stop</button>
                    {:else if m.installed}
                      <button class="btn small" disabled={voice.settings.model === m.id && voice.settings.enabled} title="Delete the file" onclick={() => removeModel(m.id)}>Delete</button>
                    {:else}
                      <button class="btn small primary" disabled={!!voice.download} onclick={() => download(m.id)}>Download</button>
                    {/if}
                  </span>
                  {#if downloading}
                    <div class="progress mprogress"><span style:width="{voice.download?.[1] ?? 0}%"></span></div>
                  {/if}
                </div>
              {/each}
            </div>
            {#if voiceError}
              <p class="hint error">{voiceError}</p>
            {/if}
          </section>

          <section>
            <h3>Language model</h3>
            {#if !voice.llm_server}
              <p class="hint error">
                llama-server wasn't found. The app's installer comes with it; for a build of your own, run
                <code>tools/build_sidecars.sh</code> or install llama.cpp (<code>brew install llama.cpp</code> on macOS).
              </p>
            {:else}
              <p class="hint">
                It turns what you say into actions on the board and a short answer. Bigger models understand more and take longer.
                llama-server: <code>{voice.llm_server}</code>
              </p>
            {/if}
            <div class="ports">
              {#each voice.models.filter((m) => m.kind === "llm") as m (m.id)}
                {@const downloading = voice.download?.[0] === m.id}
                {@const chosen = voice.settings.llm_model === m.id}
                <div class="port model" class:checked={chosen}>
                  <label class="mpick" title={"License: " + m.license}>
                    <input type="radio" name="llm" checked={chosen} onchange={() => setVoice({ llm_model: m.id })} />
                    <span class="radio"></span>
                    <span class="mtext">
                      <span class="pname">{m.id}{#if m.id === voice.recommendation.whisper || m.id === voice.recommendation.llm}<span class="rec" title="What this computer runs best">recommended</span>{/if}</span>
                      <span class="mnote">{m.note}</span>
                    </span>
                  </label>
                  <span class="mside">
                    <span class="pmeta">{mb(m.bytes)}</span>
                    {#if downloading}
                      <button class="btn small" onclick={() => monitor.cancelDownload()}>{Math.round(voice.download?.[1] ?? 0)}% · Stop</button>
                    {:else if m.installed}
                      <button class="btn small" disabled={chosen && voice.settings.enabled && voice.settings.llm} title="Delete the file" onclick={() => removeModel(m.id)}>Delete</button>
                    {:else}
                      <button class="btn small primary" disabled={!!voice.download} onclick={() => download(m.id)}>Download</button>
                    {/if}
                  </span>
                  {#if downloading}
                    <div class="progress mprogress"><span style:width="{voice.download?.[1] ?? 0}%"></span></div>
                  {/if}
                </div>
              {/each}
            </div>
          </section>

          <section>
            <h3>Voices</h3>
            {#if voice.piper_install}
              <p class="hint">Installing Piper… <code>{voice.piper_install}</code></p>
            {:else if !voice.piper}
              <p class="hint">
                Piper speaks the answers. It's a Python program (GPL-3.0) that runs next to the app; installing it puts it in a private
                virtualenv, about 100 MB from PyPI.
              </p>
              <button class="btn primary" onclick={installPiper}>Install Piper</button>
            {:else}
              <p class="hint">Piper voices from Hugging Face, one per language. Check each voice's license before sharing what it says.</p>
            {/if}
            {#if piperError}
              <p class="hint error">{piperError}</p>
            {/if}
            {#each VOICE_LANGUAGES as [lang, label] (lang)}
              <div class="vlang">
                <span class="rlabel">{label}</span>
                <button
                  class="btn small"
                  disabled={voice.tts !== "ready" || testing !== null || monitor.link !== "connected" || !voice.models.find((m) => m.id === voice?.settings.voices[lang])?.installed}
                  onclick={() => testVoice(lang)}>{testing === lang ? "Speaking…" : "Test"}</button
                >
              </div>
              <div class="ports">
                {#each voice.models.filter((m) => m.kind === "voice" && m.language === lang) as m (m.id)}
                  {@const downloading = voice.download?.[0] === m.id}
                  {@const chosen = voice.settings.voices[lang] === m.id}
                  <div class="port model" class:checked={chosen}>
                    <label class="mpick" title={m.license}>
                      <input
                        type="radio"
                        name={"voice-" + lang}
                        checked={chosen}
                        onchange={() => voice && setVoice({ voices: { ...voice.settings.voices, [lang]: m.id } })}
                      />
                      <span class="radio"></span>
                      <span class="mtext">
                        <span class="pname">{m.id}</span>
                        <span class="mnote">{m.note}</span>
                      </span>
                    </label>
                    <span class="mside">
                      <span class="pmeta">{mb(m.bytes)}</span>
                      {#if downloading}
                        <button class="btn small" onclick={() => monitor.cancelDownload()}>{Math.round(voice.download?.[1] ?? 0)}% · Stop</button>
                      {:else if m.installed}
                        <button class="btn small" title="Delete the files" onclick={() => removeModel(m.id)}>Delete</button>
                      {:else}
                        <button class="btn small primary" disabled={!!voice.download} onclick={() => download(m.id)}>Download</button>
                      {/if}
                    </span>
                    {#if downloading}
                      <div class="progress mprogress"><span style:width="{voice.download?.[1] ?? 0}%"></span></div>
                    {/if}
                  </div>
                {/each}
              </div>
            {/each}
          </section>

          <section>
            <h3>Transcripts</h3>
            {#each newestFirst as entry (entry.at + "-" + entry.id)}
              <div class="transcript">
                <span class="tmeta">{clock(entry.at)}</span>
                {#if entry.transcript}
                  <span class="tlang">{entry.transcript.language}</span>
                  <span class="ttext">{entry.transcript.text}</span>
                  <span class="tmeta">{(entry.transcript.elapsed_ms / 1000).toFixed(1)} s</span>
                  {#if entry.reply}
                    <span class="treply" class:muted={!entry.reply.understood}>
                      → {entry.reply.text}
                      {#if entry.reply.by === "rules" && voice.settings.llm}
                        <span class="taction" title="The language model wasn't available: a fixed phrase answered">fixed phrase</span>
                      {/if}
                      {#each entry.reply.actions as action, i (i)}
                        <span class="taction">{action}</span>
                      {/each}
                      {#if entry.spoken?.reason === "barge_in"}
                        <span class="taction" title="The wake word was said over the answer">interrupted</span>
                      {/if}
                    </span>
                    <span class="tmeta">{entry.spoken ? `${(entry.spoken.first_audio_ms / 1000).toFixed(1)} s` : ""}</span>
                  {/if}
                {:else}
                  <span class="ttext muted">No words heard</span>
                {/if}
              </div>
            {:else}
              <p class="empty">{voice.settings.enabled ? "Say “Alexa”, then a command." : "Turn voice on to see what the board hears."}</p>
            {/each}
          </section>
        {/if}
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
    grid-template-columns: repeat(6, 1fr);
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
    width: calc((100% - 6px) / 6);
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

  .voice {
    --accent: #30d5f0;
  }
  .switch {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
    cursor: pointer;
  }
  .switch input,
  .check input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }
  .track {
    position: relative;
    width: 32px;
    height: 18px;
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.1);
    transition: background 200ms;
  }
  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--dim);
    transition:
      transform 200ms,
      background 200ms;
  }
  .switch input:checked + .track {
    background: color-mix(in srgb, var(--accent) 40%, transparent);
  }
  .switch input:checked + .track .knob {
    transform: translateX(14px);
    background: var(--accent);
  }
  .switch:has(input:disabled) {
    opacity: 0.45;
    cursor: default;
  }
  .switch:has(input:focus-visible) .track,
  .check:has(input:focus-visible) span {
    outline: 1.5px solid var(--accent);
  }
  .slabel {
    font: 550 13px/1.2 var(--sans);
  }
  .facts dd.good {
    color: #5ee38a;
  }
  .facts dd.bad {
    color: var(--hot);
  }
  .langs {
    grid-template-columns: repeat(3, 1fr);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
    font: 450 12px/1.4 var(--sans);
    color: var(--dim);
    cursor: pointer;
  }
  .check span::before {
    content: "";
    display: inline-block;
    width: 12px;
    height: 12px;
    margin-right: 8px;
    vertical-align: -2px;
    border-radius: 3px;
    border: 1.5px solid var(--faint);
    transition: all 200ms;
  }
  .check input:checked + span::before {
    border-color: var(--accent);
    background: var(--accent);
  }
  .model {
    grid-template-columns: 1fr auto;
    cursor: default;
  }
  .mpick {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    cursor: pointer;
  }
  .mtext {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .rec {
    margin-left: 6px;
    padding: 1px 5px;
    border-radius: 4px;
    font: 600 9.5px/1.4 var(--sans);
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    vertical-align: 1px;
  }
  .hwbox {
    margin: 4px 0 12px;
  }
  .hwbox .hint {
    margin: 4px 0 8px;
  }
  .mnote {
    font: 450 11px/1.35 var(--sans);
    color: var(--faint);
  }
  .mside {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .btn.small {
    height: 26px;
    padding: 0 10px;
    font-size: 11.5px;
  }
  .mprogress {
    grid-column: 1 / -1;
    margin: 2px 0 0;
  }
  .transcript {
    display: grid;
    grid-template-columns: auto auto 1fr auto;
    align-items: baseline;
    gap: 8px;
    padding: 9px 0;
    border-bottom: 1px solid var(--line);
  }
  .transcript:last-child {
    border-bottom: 0;
  }
  .tmeta {
    font: 500 10.5px/1 var(--mono);
    color: var(--faint);
  }
  .tlang {
    padding: 2px 5px;
    border-radius: 5px;
    background: color-mix(in srgb, #30d5f0 14%, transparent);
    color: #30d5f0;
    font: 600 10px/1 var(--mono);
    text-transform: uppercase;
  }
  .ttext {
    font: 450 13px/1.4 var(--sans);
  }
  .ttext.muted {
    grid-column: span 3;
    color: var(--faint);
  }
  .treply {
    grid-column: 2 / 4;
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 4px 8px;
    font: 450 12.5px/1.4 var(--sans);
    color: var(--dim);
  }
  .treply.muted {
    color: var(--faint);
  }
  .taction {
    font: 500 10.5px/1.3 var(--mono);
    color: var(--faint);
  }
  .volume {
    display: grid;
    grid-template-columns: auto 1fr 3.5em;
    align-items: center;
    gap: 12px;
    margin-top: 12px;
  }
  .volume input {
    width: 100%;
    accent-color: var(--accent);
  }
  .volume .pmeta {
    text-align: right;
  }
  .vlang {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin: 14px 0 8px;
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
