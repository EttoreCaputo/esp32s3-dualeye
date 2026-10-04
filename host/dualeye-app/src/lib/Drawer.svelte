<script module lang="ts">
  export type Tab = "voice" | "timers" | "claude" | "device" | "diagnostics";
</script>

<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { formatTokens, isClaudeFace } from "./firmware";
  import {
    monitor,
    type ClaudeAlertSettings,
    type ClaudeAlertsInfo,
    type ClaudeLink,
    type FirmwareInfo,
    type Hardware,
    type McpInfo,
    type PortInfo,
    type PowerHelper,
    type Personality,
    type Reading,
    type SttLanguage,
    type ShowOn,
    type TimerInfo,
    type TimerKind,
    type TtsEngine,
    type ModelInfo,
    type VoiceInfo,
    type VoiceSettings,
  } from "./monitor.svelte";

  const TABS: [Tab, string][] = [
    ["voice", "Voice"],
    ["timers", "Timers"],
    ["claude", "Claude"],
    ["device", "Device"],
    ["diagnostics", "Diagnostics"],
  ];

  let { open = $bindable(false), tab = $bindable("voice") }: { open: boolean; tab?: Tab } = $props();

  let ports = $state<PortInfo[]>([]);
  let firmware = $state<FirmwareInfo | null>(null);
  let confirming = $state(false);
  let readings = $state<Reading[]>([]);
  let powerHelper = $state<PowerHelper | null>(null);
  let powerBusy = $state(false);
  const isMac = typeof navigator !== "undefined" && /Mac/.test(navigator.userAgent);
  let consoleEl = $state<HTMLElement>();
  let claudeLink = $state<ClaudeLink | null>(null);
  let claudeError = $state("");
  let claudeBusy = $state(false);
  let alerts = $state<ClaudeAlertsInfo | null>(null);
  let alertsError = $state("");
  let alertsBusy = $state(false);
  let mcp = $state<McpInfo | null>(null);
  let copied = $state<"code" | "desktop" | null>(null);
  let follow = $state(true);

  $effect(() => {
    if (!open || tab !== "device") return;
    const load = () => monitor.listPorts().then((p) => (ports = p));
    load();
    const id = setInterval(load, 2000);
    return () => clearInterval(id);
  });

  // The Timers tab: read every second while it's open, so the times run.
  let timerError = $state("");
  let timerMinutes = $state(10);
  let timerLabel = $state("");
  let remindText = $state("");
  let remindAt = $state("");
  let pomo = $state({ work: 25, rest: 5, rounds: 4 });
  $effect(() => {
    if (!open || tab !== "timers") return;
    const load = () => monitor.timersInfo().catch((e) => (timerError = String(e)));
    load();
    const id = setInterval(load, 1000);
    return () => clearInterval(id);
  });
  async function timerTool(name: string, args: Record<string, unknown>) {
    timerError = "";
    try {
      await monitor.timerTool(name, args);
    } catch (e) {
      timerError = String(e);
    }
  }
  const startTimer = (secs: number, label = "") => timerTool("set_timer", label.trim() ? { seconds: secs, label: label.trim() } : { seconds: secs });
  const controlTimer = (action: "cancel" | "pause" | "resume", t: TimerInfo) => timerTool("control_timer", { action, which: String(t.id) });
  async function remind() {
    await timerTool("set_reminder", { text: remindText.trim(), at: remindAt });
    if (!timerError) remindText = "";
  }
  const TIMER_KINDS: Record<TimerKind, [string, string]> = {
    timer: ["Timer", "#f8a639"],
    work: ["Pomodoro: focus", "#ff6347"],
    break: ["Pomodoro: break", "#40e080"],
    reminder: ["Reminder", "#3ae7ed"],
  };
  const clockText = (secs: number) => {
    const two = (n: number) => String(n).padStart(2, "0");
    return secs >= 3600 ? `${Math.trunc(secs / 3600)}:${two(Math.trunc((secs % 3600) / 60))}:${two(secs % 60)}` : `${Math.trunc(secs / 60)}:${two(secs % 60)}`;
  };
  const timerSub = (t: TimerInfo) => {
    if (t.state === "ring") return "ringing";
    if (t.state === "pause") return "paused";
    return t.ends_at ? `until ${t.ends_at}` : "";
  };
  const SHOW_ON: [ShowOn, string][] = [
    ["left", "Left"],
    ["right", "Right"],
    ["none", "Neither"],
  ];

  $effect(() => {
    if (!open || tab !== "diagnostics" || diag !== "sensors") return;
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
    if (!open || tab !== "claude") return;
    let alive = true;
    const load = () => {
      monitor.claudeLink().then((l) => alive && (claudeLink = l));
      monitor.claudeAlerts().then((a) => alive && (alerts = a));
      monitor.mcpInfo().then((m) => alive && (mcp = m));
    };
    load();
    const id = setInterval(load, 3000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  });

  /** Diagnostics shows the sensors or the board's console. */
  let diag = $state<"sensors" | "console">("sensors");
  /** The online services' keys are open. */
  let keysOpen = $state(false);
  let customLength = $state<number | null>(null);

  const PERSONALITIES: [Personality, string, string][] = [
    ["cute", "Cute", "Warm, cheerful, a little playful"],
    ["playful", "Playful", "Jokes, puns, gentle teasing"],
    ["calm", "Calm", "Soft and reassuring"],
    ["sassy", "Sassy", "A cat with dry humour"],
    ["butler", "Butler", "Formal, at your service"],
    ["minimal", "Minimal", "Just the facts, no chat"],
    ["custom", "Your own", "Describe it yourself"],
  ];
  /** `agent::MAX_PERSONALITY`. */
  const MAX_PERSONALITY = 300;
  const KIND_NAMES: Record<ModelInfo["kind"], string> = { whisper: "Hearing", llm: "Understanding", voice: "Voice" };
  const ENGINE_NAMES: Record<TtsEngine, string> = { piper: "Piper", kokoro: "Kokoro" };
  const STATUS = { off: "Off", starting: "Loading…", ready: "Ready", error: "Not working" };

  /** A model as its list shows it: size and whether it's here, or whether its service has a key. */
  function optionLabel(v: VoiceInfo, m: ModelInfo): string {
    if (m.provider) return `${m.id.slice(m.id.indexOf(":") + 1)}${m.installed ? "" : ` · needs a ${providerName(v, m.provider)} key`}`;
    return `${m.id} · ${mb(m.bytes)}${m.installed ? " · downloaded" : ""}${m.id === v.recommendation.whisper || m.id === v.recommendation.llm ? " · recommended" : ""}`;
  }

  let voice = $state<VoiceInfo | null>(null);
  let voiceError = $state("");

  $effect(() => {
    if (!open || tab !== "voice") return;
    let alive = true;
    customLength = null;
    const load = () => monitor.voiceInfo().then((v) => alive && (voice = v));
    load();
    // Faster while a model downloads, for its progress bar.
    const id = setInterval(load, 700);
    return () => {
      alive = false;
      clearInterval(id);
    };
  });

  // The speaker's volume and the eyes live on the board: read them when the tab opens.
  let volume = $state<number | null>(null);
  let eyes = $state<boolean | null>(null);
  let idleEyes = $state<boolean | null>(null);
  $effect(() => {
    if (!open || tab !== "voice" || monitor.link !== "connected") return;
    let alive = true;
    monitor
      .boardVoice()
      .then((b) => {
        if (!alive) return;
        volume = b.volume;
        eyes = b.eyes;
        idleEyes = b.idle_eyes;
      })
      .catch(() => {
        if (!alive) return;
        volume = null;
        eyes = null;
        idleEyes = null;
      });
    return () => {
      alive = false;
    };
  });

  async function setIdleEyes(on: boolean) {
    idleEyes = on;
    try {
      await monitor.setBoardIdleEyes(on);
    } catch (e) {
      idleEyes = !on;
      voiceError = String(e);
    }
  }

  async function setEyes(on: boolean) {
    eyes = on;
    try {
      await monitor.setBoardEyes(on);
    } catch (e) {
      eyes = !on;
      voiceError = String(e);
    }
  }

  async function setVolume(percent: number) {
    volume = percent;
    try {
      await monitor.setBoardVolume(percent);
    } catch (e) {
      voiceError = String(e);
    }
  }

  let engineError = $state("");
  async function installEngine(engine: TtsEngine) {
    engineError = "";
    try {
      await monitor.installEngine(engine);
    } catch (e) {
      engineError = String(e);
    }
    voice = await monitor.voiceInfo();
  }
  const ENGINES: [TtsEngine, string, string][] = [
    ["piper", "Piper", "Light and quick (GPL-3.0, about 100 MB from PyPI)"],
    ["kokoro", "Kokoro", "Warmer, livelier voices, a little slower to answer (MIT, about 150 MB from PyPI)"],
  ];

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

  /** API keys being typed, by provider. */
  let keyDrafts = $state<Record<string, string>>({});
  let keyError = $state("");
  let savingKey = $state<string | null>(null);

  async function saveKey(provider: string, key: string | null) {
    keyError = "";
    savingKey = provider;
    try {
      voice = await monitor.setApiKey(provider, key);
      keyDrafts[provider] = "";
    } catch (e) {
      keyError = String(e);
      voice = await monitor.voiceInfo();
    } finally {
      savingKey = null;
    }
  }

  const providerName = (v: VoiceInfo, id: string | null) => v.providers.find((p) => p.id === id)?.name ?? id ?? "";

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

  async function linkHooks(connect: boolean) {
    alertsBusy = true;
    alertsError = "";
    try {
      alerts = await monitor.claudeHooks(connect);
    } catch (e) {
      alertsError = String(e);
    } finally {
      alertsBusy = false;
    }
  }

  async function setAlerts(change: Partial<ClaudeAlertSettings>) {
    if (!alerts) return;
    alertsError = "";
    alerts = await monitor.setClaudeAlerts({ ...alerts.settings, ...change });
  }

  const DONE_AFTER = [30, 60, 120, 300];

  async function testAlert() {
    alertsBusy = true;
    alertsError = "";
    try {
      await monitor.testClaudeAlert();
      // The bridge gives it in a second or two (longer while it speaks).
      setTimeout(() => monitor.claudeAlerts().then((a) => (alerts = a)), 1500);
    } catch (e) {
      alertsError = String(e);
    } finally {
      alertsBusy = false;
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

  function onKey(e: KeyboardEvent) {
    if (open && e.key === "Escape") open = false;
  }

  // Drag the left edge to widen the drawer; the width is kept across launches.
  const MIN_W = 380;
  const MAX_W = 720;
  const WIDTH_KEY = "dualeye.drawerWidth";
  let width = $state(460);
  let resizing = $state(false);
  try {
    const saved = Number(localStorage.getItem(WIDTH_KEY));
    if (saved) width = Math.min(MAX_W, Math.max(MIN_W, saved));
  } catch {}

  function startResize(e: PointerEvent) {
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    resizing = true;
    const [x0, w0] = [e.clientX, width];
    const move = (ev: PointerEvent) => (width = Math.min(MAX_W, Math.max(MIN_W, w0 + x0 - ev.clientX)));
    const end = () => {
      resizing = false;
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", end);
      handle.removeEventListener("pointercancel", end);
      try {
        localStorage.setItem(WIDTH_KEY, String(Math.round(width)));
      } catch {}
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", end);
    handle.addEventListener("pointercancel", end);
  }

  function resetWidth() {
    width = 460;
    try {
      localStorage.removeItem(WIDTH_KEY);
    } catch {}
  }

  function scrolled() {
    if (!consoleEl) return;
    follow = consoleEl.scrollHeight - consoleEl.scrollTop - consoleEl.clientHeight < 24;
  }

  const hex = (n: number) => n.toString(16).padStart(4, "0");
  const kb = (n: number) => `${Math.round(n / 1024)} KB`;
  const fmt = (r: Reading) => (r.unit === "RPM" || r.unit === "%" || r.unit === "MB" ? r.value.toFixed(0) : r.value.toFixed(1));
</script>

<!-- One model to pick, from this computer's or an online service's, and what it still needs. -->
{#snippet picker(v: VoiceInfo, label: string, what: string, kind: ModelInfo["kind"], lang: string | null, chosen: string, onpick: (id: string) => void, off: boolean)}
  {@const list = v.models.filter((m) => m.kind === kind && (lang === null || m.language === lang))}
  {@const m = list.find((x) => x.id === chosen)}
  {@const downloading = !!m && v.download?.[0] === m.id}
  {@const installing = m?.engine ? v[`${m.engine}_install`] : null}
  <div class="mrow" class:off>
    <div class="mhead">
      <span class="mtext">
        <span class="pname">{label}</span>
        <span class="mnote">{what}</span>
      </span>
      {#if kind === "voice" && lang}
        <button
          class="btn small"
          disabled={v.tts !== "ready" || testing !== null || monitor.link !== "connected" || !m?.installed}
          title="Say a sentence through the board"
          onclick={() => testVoice(lang)}>{testing === lang ? "Speaking…" : "Test"}</button
        >
      {/if}
    </div>
    <div class="select">
      <select value={chosen} disabled={off} aria-label={label} onchange={(e) => onpick(e.currentTarget.value)}>
        {#if !m}<option value={chosen} disabled>Choose…</option>{/if}
        <optgroup label="On this computer">
          {#each list.filter((o) => !o.provider) as o (o.id)}
            <option value={o.id}>{optionLabel(v, o)}</option>
          {/each}
        </optgroup>
        {#each v.providers as p (p.id)}
          {@const cloud = list.filter((o) => o.provider === p.id)}
          {#if cloud.length}
            <optgroup label="{p.name}, online">
              {#each cloud as o (o.id)}
                <option value={o.id}>{optionLabel(v, o)}</option>
              {/each}
            </optgroup>
          {/if}
        {/each}
      </select>
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 6.5 8 10l3.5-3.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /></svg>
    </div>
    {#if m && !off}
      <div class="mstatus">
        <span class="mnote">
          {m.note}
        </span>
        <span class="mside">
          {#if m.provider}
            {#if m.installed}
              <span class="chip online" title="Runs online: what you say is sent there">Online</span>
            {:else}
              <button class="btn small primary" onclick={() => (keysOpen = true)}>Add a {providerName(v, m.provider)} key</button>
            {/if}
          {:else if downloading}
            <button class="btn small" onclick={() => monitor.cancelDownload()}>{Math.round(v.download?.[1] ?? 0)}% · Stop</button>
          {:else if !m.installed}
            <button class="btn small primary" disabled={!!v.download} onclick={() => download(m.id)}>Download · {mb(m.bytes)}</button>
          {:else if m.engine && !v[m.engine]}
            <button class="btn small primary" disabled={!!installing} onclick={() => m.engine && installEngine(m.engine)}
              >{installing ? "Installing…" : `Install ${ENGINE_NAMES[m.engine]}`}</button
            >
          {:else}
            <span class="chip ready">Ready</span>
          {/if}
        </span>
      </div>
      {#if downloading}
        <div class="progress mprogress"><span style:width="{v.download?.[1] ?? 0}%"></span></div>
      {/if}
      {#if installing}
        <p class="hint small">{installing}</p>
      {/if}
    {/if}
  </div>
{/snippet}

<svelte:window onkeydown={onKey} />

{#if open}
  <button class="scrim" transition:fade={{ duration: 200 }} onclick={() => (open = false)} aria-label="Close settings"></button>
  <aside
    class="drawer"
    class:resizing
    style:--w="{width}px"
    transition:fly={{ x: 40, duration: 320, easing: cubicOut, opacity: 0 }}
    aria-label="Settings"
  >
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="grip" onpointerdown={startResize} ondblclick={resetWidth} title="Drag to resize, double-click to reset"></div>
    <nav>
      {#each TABS as [key, label] (key)}
        <button class:active={tab === key} onclick={() => (tab = key)}>{label}</button>
      {/each}
      <span class="indicator" style:--i={TABS.findIndex(([key]) => key === tab)}></span>
    </nav>

    <div class="content">
      {#if tab === "voice"}
        {#if !voice}
          <p class="empty">Loading…</p>
        {:else}
          {@const v = voice}
          <section class="voice">
            <label class="switch master">
              <input type="checkbox" checked={v.settings.enabled} onchange={(e) => setVoice({ enabled: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="mtext">
                <span class="slabel">Voice assistant</span>
                <span class="mnote">Say “Alexa”, then a command: “metti la faccia rings a sinistra”, “what's the temperature?”</span>
              </span>
            </label>
            {#if v.settings.enabled}
              <div class="pills">
                {#each [["Hearing", v.stt], ["Understanding", v.settings.llm ? v.llm : "off"], ["Speaking", v.tts]] as [name, state] (name)}
                  <span class="pill {state}" title={STATUS[state as keyof typeof STATUS]}><span class="dot"></span>{name}</span>
                {/each}
                <span class="pill board" title="The board's voice state">Board: {monitor.link === "connected" ? monitor.voice : "offline"}</span>
              </div>
              {#each [v.stt === "error" && v.stt_error, v.tts === "error" && v.tts_error, v.settings.llm && v.llm === "error" && v.llm_error && `${v.llm_error}. Meanwhile a few fixed phrases work.`] as err, i (i)}
                {#if err}<p class="hint error">{err}</p>{/if}
              {/each}
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
                    class:checked={v.settings.language === id}
                    role="radio"
                    aria-checked={v.settings.language === id}
                    title={id === "auto" ? "Whisper tells Italian from English" : `Always ${label}`}
                    onclick={() => setVoice({ language: id })}>{label}</button
                  >
                {/each}
              </div>
            </div>
          </section>

          <section class="voice" class:dimmed={!v.settings.llm}>
            <h3>Personality</h3>
            <div class="persona" role="radiogroup" aria-label="Personality">
              {#each PERSONALITIES as [id, name, blurb] (id)}
                <button class="pcard" class:checked={v.settings.personality === id} role="radio" aria-checked={v.settings.personality === id} onclick={() => setVoice({ personality: id })}>
                  <b>{name}</b>
                  <small>{blurb}</small>
                </button>
              {/each}
            </div>
            {#if v.settings.personality === "custom"}
              <div class="custom">
                <textarea
                  rows="3"
                  maxlength={MAX_PERSONALITY}
                  placeholder="A grumpy old pirate parrot who loves bad weather and calls me captain"
                  value={v.settings.personality_custom}
                  oninput={(e) => (customLength = e.currentTarget.value.length)}
                  onchange={(e) => setVoice({ personality_custom: e.currentTarget.value })}
                ></textarea>
                <span class="pmeta">{customLength ?? v.settings.personality_custom.length}/{MAX_PERSONALITY}</span>
              </div>
            {/if}
            <p class="hint">
              {v.settings.llm
                ? "Changes how DualEye talks, from the next command. It still does what you ask the same way."
                : "Turn on “Understand with a language model” below: the fixed phrases have no personality."}
            </p>
          </section>

          <section class="voice">
            <h3>Behaviour</h3>
            <label class="switch">
              <input type="checkbox" checked={v.settings.speak} onchange={(e) => setVoice({ speak: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">Answer out loud</span>
            </label>
            <label class="switch">
              <input type="checkbox" checked={v.settings.follow_up} disabled={!v.settings.speak} onchange={(e) => setVoice({ follow_up: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">Keep listening after an answer, without the wake word</span>
            </label>
            <label class="switch">
              <input type="checkbox" checked={v.settings.pause_music} onchange={(e) => setVoice({ pause_music: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">Pause the music while listening</span>
            </label>
            <label class="switch">
              <input type="checkbox" checked={v.settings.llm} onchange={(e) => setVoice({ llm: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">Understand with a language model</span>
            </label>
            <label class="switch" title={eyes === null ? "Needs the board connected, with firmware 1.0.1 or newer" : ""}>
              <input type="checkbox" checked={eyes ?? true} disabled={eyes === null} onchange={(e) => setEyes(e.currentTarget.checked)} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">Show animated eyes while talking, instead of the ring</span>
            </label>
            <label class="switch" title={idleEyes === null ? "Needs the board connected, with firmware 1.0.2 or newer" : ""}>
              <input type="checkbox" checked={idleEyes ?? true} disabled={idleEyes === null} onchange={(e) => setIdleEyes(e.currentTarget.checked)} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">Let the eyes play now and then while idle</span>
            </label>
          </section>

          <section class="voice">
            <h3>Models</h3>
            {#if !usesRecommended(v)}
              <div class="recbox">
                <p class="hint">{v.recommendation.why}</p>
                <button class="btn small" onclick={useRecommended}>
                  Use {v.recommendation.whisper}{v.recommendation.llm ? ` + ${v.recommendation.llm}` : " without a language model"}
                </button>
              </div>
            {/if}
            {@render picker(v, "Hearing", "Turns what you say into text", "whisper", null, v.settings.model, (id) => setVoice({ model: id }), false)}
            {@render picker(v, "Understanding", "Works out what to do and what to answer", "llm", null, v.settings.llm_model, (id) => setVoice({ llm_model: id }), !v.settings.llm)}
            {#each VOICE_LANGUAGES as [lang, label] (lang)}
              {@render picker(v, `Voice · ${label}`, "Speaks the answers", "voice", lang, v.settings.voices[lang] ?? "", (id) => setVoice({ voices: { ...v.settings.voices, [lang]: id } }), !v.settings.speak)}
            {/each}
            {#if voiceError}<p class="hint error">{voiceError}</p>{/if}
            {#if engineError}<p class="hint error">{engineError}</p>{/if}

            <details class="more" bind:open={keysOpen}>
              <summary>Online services <span class="pmeta">{v.providers.filter((p) => p.key).length ? "key saved" : "no key"}</span></summary>
              <p class="hint">
                Instead of this computer, an online service can hear, understand and speak: nothing to download, but what you say leaves this
                computer, and free plans have daily limits. With a key, its models show up in the lists above.
              </p>
              {#each v.providers as p (p.id)}
                <div class="provider">
                  <div class="phead">
                    <span class="mtext"><span class="pname">{p.name}</span><span class="mnote">{p.note}</span></span>
                    {#if p.key === "env"}
                      <span class="pmeta" title="Its environment variable wins over a saved key">Key from {p.key_env}</span>
                    {:else if p.key === "saved"}
                      <button class="btn small" disabled={savingKey === p.id} onclick={() => saveKey(p.id, null)}>Forget key</button>
                    {/if}
                  </div>
                  {#if p.key !== "env"}
                    <div class="tform">
                      <input
                        class="ttext key"
                        type="password"
                        autocomplete="off"
                        spellcheck="false"
                        placeholder={p.key ? "Replace the API key" : `API key, from ${p.keys_url}`}
                        aria-label={`${p.name} API key`}
                        bind:value={keyDrafts[p.id]}
                      />
                      <button class="btn small primary" disabled={!keyDrafts[p.id]?.trim() || savingKey === p.id} onclick={() => saveKey(p.id, keyDrafts[p.id])}
                        >{savingKey === p.id ? "Checking…" : "Save"}</button
                      >
                    </div>
                  {/if}
                </div>
              {/each}
              {#if keyError}<p class="hint error">{keyError}</p>{/if}
            </details>

            <details class="more">
              <summary>Downloads and advanced</summary>
              <p class="hint">
                {hardwareLine(v.hardware)}. whisper-server: <code>{v.server ?? "not found"}</code> · llama-server: <code>{v.llm_server ?? "not found"}</code>
              </p>
              {#if !v.server}
                <p class="hint error">
                  whisper-server wasn't found. The app's installer comes with it; for a build of your own, run <code>tools/build_sidecars.sh</code> or
                  install whisper.cpp (<code>brew install whisper-cpp</code> on macOS).
                </p>
              {/if}
              {#if !v.llm_server}
                <p class="hint error">
                  llama-server wasn't found. The app's installer comes with it; for a build of your own, run <code>tools/build_sidecars.sh</code> or
                  install llama.cpp (<code>brew install llama.cpp</code> on macOS).
                </p>
              {/if}
              <div class="ports">
                {#each ENGINES as [engine, name, note] (engine)}
                  {@const installing = v[`${engine}_install`]}
                  <div class="port model" class:checked={!!v[engine]}>
                    <span class="mtext">
                      <span class="pname">{name}</span>
                      <span class="mnote">{installing ? `Installing… ${installing}` : note}</span>
                    </span>
                    <span class="mside">
                      {#if v[engine]}
                        <span class="pmeta">Installed</span>
                      {:else}
                        <button class="btn small primary" disabled={!!installing} onclick={() => installEngine(engine)}>{installing ? "Installing…" : "Install"}</button>
                      {/if}
                    </span>
                  </div>
                {/each}
                {#each v.models.filter((m) => !m.provider && m.installed) as m (m.id)}
                  {@const used = [v.settings.model, v.settings.llm_model, ...Object.values(v.settings.voices)].includes(m.id)}
                  <div class="port model">
                    <span class="mtext">
                      <span class="pname">{m.id}</span>
                      <span class="mnote">{KIND_NAMES[m.kind]} · {mb(m.bytes)}{used ? " · in use" : ""}</span>
                    </span>
                    <span class="mside">
                      <button class="btn small" disabled={used && v.settings.enabled} title={m.engine === "kokoro" ? "Every Kokoro voice shares these files" : "Delete the file"} onclick={() => removeModel(m.id)}>Delete</button>
                    </span>
                  </div>
                {/each}
              </div>
              <label class="check">
                <input type="checkbox" checked={v.settings.keep_recordings} onchange={(e) => setVoice({ keep_recordings: e.currentTarget.checked })} />
                <span>Keep recordings as WAV files, for debugging</span>
              </label>
            </details>
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
                      {#if entry.reply.by === "rules" && v.settings.llm}
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
              <p class="empty">{v.settings.enabled ? "Say “Alexa”, then a command." : "Turn the voice assistant on to see what the board hears."}</p>
            {/each}
          </section>
        {/if}
      {:else if tab === "claude"}
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

        <section class="alerts">
          <h3>Claude alerts</h3>
          <p class="hint">
            The board tells you when Claude Code needs you (a permission or a question), when it finishes a long task and when your plan's
            limits pass 80 % and 95 %: the eyes react, a message shows over the watch face and, with spoken replies on, it says so out loud.
          </p>
          {#if alerts}
            {#if alerts.hooks.connected}
              <dl class="facts">
                <div><dt>Hooks</dt><dd>Connected</dd></div>
                <div>
                  <dt>Last event</dt>
                  <dd>{alerts.hooks.last_event_s === null ? "Waiting" : ago(alerts.hooks.last_event_s)}</dd>
                </div>
                {#if alerts.last}
                  <div class="wide">
                    <dt>Last alert, {ago(alerts.last.age_s)}</dt>
                    <dd class="small">{alerts.last.text}{alerts.last.error ? ` (not given: ${alerts.last.error})` : ""}</dd>
                  </div>
                {/if}
              </dl>
              {#if alerts.hooks.last_event_s === null}
                <p class="hint">Claude Code sessions that were already open may need a restart to pick the hooks up.</p>
              {/if}
            {:else}
              <p class="hint">
                This adds hooks to <code>{alerts.hooks.settings_path ?? "~/.claude/settings.json"}</code> that run in the background and
                never slow Claude down; yours stay as they are. The limit alerts need the status line above.
              </p>
            {/if}
            <label class="switch">
              <input type="checkbox" checked={alerts.settings.needs_you} onchange={(e) => setAlerts({ needs_you: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">When Claude needs you</span>
            </label>
            <label class="switch">
              <input type="checkbox" checked={alerts.settings.done} onchange={(e) => setAlerts({ done: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">When Claude finishes a long task</span>
            </label>
            <label class="switch">
              <input type="checkbox" checked={alerts.settings.usage} onchange={(e) => setAlerts({ usage: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">When a limit passes 80 % or 95 %</span>
            </label>
            <label class="switch">
              <input type="checkbox" checked={alerts.settings.speak} onchange={(e) => setAlerts({ speak: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">Say them out loud</span>
            </label>
            <div class="rotation">
              <span class="rlabel">Long task</span>
              <div class="rots" role="radiogroup" aria-label="Long task">
                {#each DONE_AFTER as secs (secs)}
                  <button
                    class="rot"
                    class:checked={alerts.settings.done_after_s === secs}
                    role="radio"
                    aria-checked={alerts.settings.done_after_s === secs}
                    disabled={!alerts.settings.done}
                    title="Tell me when a task took at least this long"
                    onclick={() => setAlerts({ done_after_s: secs })}>{secs < 60 ? `${secs} s` : `${secs / 60} min`}</button
                  >
                {/each}
              </div>
            </div>
            <div class="rotation">
              <span class="rlabel">Language</span>
              <div class="rots langs" role="radiogroup" aria-label="Language of the alerts">
                {#each VOICE_LANGUAGES as [code, label] (code)}
                  <button
                    class="rot"
                    class:checked={alerts.settings.language === code}
                    role="radio"
                    aria-checked={alerts.settings.language === code}
                    onclick={() => setAlerts({ language: code })}>{label}</button
                  >
                {/each}
              </div>
            </div>
            {#if alerts.settings.speak && !alerts.can_speak}
              <p class="hint">Speaking needs voice on, with “Answer out loud” and a voice, in the Voice tab; until then they're only shown.</p>
            {/if}
            <div class="actions">
              {#if alerts.hooks.connected}
                <button class="btn" disabled={alertsBusy || monitor.link !== "connected"} onclick={testAlert}>Try it</button>
                <button class="btn" disabled={alertsBusy} onclick={() => linkHooks(false)}>Disconnect</button>
              {:else}
                <button class="btn primary" disabled={alertsBusy} onclick={() => linkHooks(true)}>Connect hooks</button>
              {/if}
            </div>
          {/if}
          {#if alertsError}
            <p class="hint error">{alertsError}</p>
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
      {:else if tab === "timers"}
        {@const info = monitor.timers}
        <p class="hint">
          The board counts the first timer down on a screen and rings when it's up; saying "Alexa" silences it, and with spoken replies
          on it says what the timer was for. By voice: "Alexa, timer 10 minuti", "ricordami alle 17 di chiamare Marco", "start a pomodoro",
          "quanto manca?". Timers keep running with the window closed, and reminders even across a restart of the app.
        </p>
        <section class="timers">
          <h3>Running</h3>
          {#each info?.timers ?? [] as t (t.id)}
            {@const [kind, color] = TIMER_KINDS[t.kind]}
            <div class="trow" class:ringing={t.state === "ring"} style:--tcolor={color}>
              <span class="tdot"></span>
              <div class="tname">
                <b>{t.label ?? kind}</b>
                <small>{t.label ? `${kind} · ` : ""}{timerSub(t)}</small>
              </div>
              <span class="tleft" class:paused={t.state === "pause"}>{clockText(t.left_s)}</span>
              <div class="tbtns">
                {#if t.state === "run"}<button class="btn small" onclick={() => controlTimer("pause", t)}>Pause</button>{/if}
                {#if t.state === "pause"}<button class="btn small" onclick={() => controlTimer("resume", t)}>Resume</button>{/if}
                <button class="btn small" onclick={() => controlTimer("cancel", t)}>{t.state === "ring" ? "Stop" : "Cancel"}</button>
              </div>
            </div>
          {:else}
            <p class="empty">No timers running.</p>
          {/each}
          {#if timerError}<p class="hint error">{timerError}</p>{/if}
        </section>
        <section class="timers">
          <h3>New timer</h3>
          <div class="rots quick">
            {#each [1, 3, 5, 10, 15, 30] as m (m)}
              <button class="rot" onclick={() => startTimer(m * 60)}>{m} min</button>
            {/each}
          </div>
          <div class="tform">
            <input class="tnum" type="number" min="1" max="1440" bind:value={timerMinutes} aria-label="Minutes" />
            <span class="tunit">min</span>
            <input class="ttext" type="text" maxlength="40" placeholder="Label (optional)" bind:value={timerLabel} />
            <button class="btn primary" disabled={!(timerMinutes > 0)} onclick={() => startTimer(Math.round(timerMinutes * 60), timerLabel).then(() => (timerLabel = ""))}
              >Start</button
            >
          </div>
        </section>
        <section class="timers">
          <h3>Reminder</h3>
          <div class="tform">
            <input class="ttext" type="text" maxlength="80" placeholder="What to remind you of" bind:value={remindText} />
            <input class="ttime" type="time" bind:value={remindAt} aria-label="At" />
            <button class="btn primary" disabled={!remindText.trim() || !remindAt} onclick={remind}>Set</button>
          </div>
        </section>
        <section class="timers">
          <h3>Pomodoro</h3>
          {#if info?.pomodoro}
            {@const p = info.pomodoro}
            <p class="hint">Round {p.round} of {p.rounds}: {p.work_s / 60} minutes of focus, {p.break_s / 60}-minute breaks.</p>
            <div class="actions"><button class="btn" onclick={() => timerTool("pomodoro", { action: "stop" })}>Stop the pomodoro</button></div>
          {:else}
            <div class="tform">
              <input class="tnum" type="number" min="1" max="120" bind:value={pomo.work} aria-label="Focus minutes" />
              <span class="tunit">min focus</span>
              <input class="tnum" type="number" min="1" max="60" bind:value={pomo.rest} aria-label="Break minutes" />
              <span class="tunit">min break</span>
              <input class="tnum" type="number" min="1" max="12" bind:value={pomo.rounds} aria-label="Rounds" />
              <span class="tunit">rounds</span>
              <button
                class="btn primary"
                onclick={() => timerTool("pomodoro", { action: "start", work_minutes: pomo.work, break_minutes: pomo.rest, rounds: pomo.rounds })}>Start</button
              >
            </div>
          {/if}
        </section>
        <section class="timers">
          <h3>On the board</h3>
          <div class="rotation">
            <span class="rlabel">Takes over</span>
            <div class="rots three" role="radiogroup" aria-label="Screen a running timer takes over">
              {#each SHOW_ON as [value, label] (value)}
                <button
                  class="rot"
                  class:checked={info?.show_on === value}
                  role="radio"
                  aria-checked={info?.show_on === value}
                  onclick={() => monitor.setTimerScreen(value)}>{label}</button
                >
              {/each}
            </div>
          </div>
          <p class="hint" style:margin-top="10px">
            While a timer runs, that screen shows it instead of its face, and goes back when it's done. The Timer face (click a screen
            on the main window) shows it on any screen you put it on.{info && !info.can_speak ? " Turn on spoken replies in the Voice tab to hear what each timer was for." : ""}
          </p>
        </section>
      {:else if tab === "device"}
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
            <p class="hint">More than one Espressif device is plugged in. Pin the DualEye's port above first.</p>
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
      {:else if tab === "diagnostics"}
        <div class="rots sub" role="tablist" aria-label="Diagnostics">
          <button class="rot" class:checked={diag === "sensors"} role="tab" aria-selected={diag === "sensors"} onclick={() => (diag = "sensors")}>Sensors</button>
          <button class="rot" class:checked={diag === "console"} role="tab" aria-selected={diag === "console"} onclick={() => (diag = "console")}>Board console</button>
        </div>
        {#if diag === "sensors"}
        <p class="hint">Every raw value the host can read. The board gets the CPU average, the first GPU, the fastest fan of each kind, RAM and VRAM.</p>
        {#if isMac && powerHelper}
          <section>
            <h3>Exact CPU power</h3>
            {#if powerHelper.state === "unavailable"}
              <p class="hint">Only in the installed app (from the .dmg), on macOS 13 or newer.</p>
            {:else}
              <label class="switch">
                <input
                  type="checkbox"
                  checked={powerHelper.state !== "off"}
                  disabled={powerBusy}
                  onchange={(e) => togglePowerHelper(e.currentTarget.checked)}
                />
                <span class="track"><span class="knob"></span></span>
                <span class="slabel">Read it with powermetrics, through a system helper</span>
              </label>
              <p class="hint">
                macOS 27 hides the CPU's energy counters from apps, so without the helper the CPU power is estimated from the SoC's
                power minus the GPU's. The helper runs Apple's <code>powermetrics</code> as root while DualEye is open, and only
                answers apps signed like DualEye.
              </p>
              {#if powerHelper.state === "needs_approval"}
                <p class="hint">
                  Allow <b>DualEye</b> in System Settings → General → Login Items &amp; Extensions, under "Allow in the Background".
                </p>
                <div class="actions">
                  <button class="btn" onclick={() => monitor.openPowerHelperSettings()}>Open System Settings</button>
                </div>
              {/if}
            {/if}
            {#if powerHelper.error}
              <p class="hint error">{powerHelper.error}</p>
            {/if}
          </section>
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
          <p class="empty">Reading sensors…</p>
        {/each}
        <section>
          <h3>Stream</h3>
          <dl class="facts">
            <div><dt>Rate</dt><dd>1 Hz</dd></div>
            <div><dt>Baud</dt><dd>115200</dd></div>
            <div><dt>Format</dt><dd>JSON line · v1</dd></div>
            <div><dt>Stale after</dt><dd>3 s</dd></div>
          </dl>
        </section>
        {:else}
        <div class="console" bind:this={consoleEl} onscroll={scrolled}>
          {#each monitor.logs as line, i (i)}
            <div class="log" class:warn={line.startsWith("W ")} class:err={line.startsWith("E ")}>{line}</div>
          {:else}
            <p class="empty">Nothing logged by the board yet.</p>
          {/each}
        </div>
        {/if}
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
    width: min(var(--w, 460px), calc(100vw - 20px));
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

  .drawer.resizing {
    user-select: none;
    cursor: ew-resize;
  }
  .grip {
    position: absolute;
    z-index: 2;
    top: 0;
    bottom: 0;
    left: 0;
    width: 8px;
    cursor: ew-resize;
    touch-action: none;
  }
  .grip::after {
    content: "";
    position: absolute;
    top: 50%;
    left: 3px;
    width: 3px;
    height: 36px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.18);
    transform: translateY(-50%);
    opacity: 0;
    transition: opacity 160ms;
  }
  .grip:hover::after,
  .resizing .grip::after {
    opacity: 1;
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
    padding: 0;
    border: 0;
    background: none;
    color: var(--dim);
    font: 550 11px/1 var(--sans);
    letter-spacing: -0.01em;
    white-space: nowrap;
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
  .timers {
    --accent: #f8a639;
  }
  .trow {
    display: grid;
    grid-template-columns: 8px 1fr auto auto;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    margin-bottom: 6px;
    border: 1px solid var(--line);
    border-radius: 10px;
    background: rgba(255, 255, 255, 0.02);
  }
  .trow.ringing {
    border-color: color-mix(in srgb, #f05354 60%, transparent);
    background: color-mix(in srgb, #f05354 10%, transparent);
  }
  .tdot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--tcolor);
  }
  .tname {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .tname b {
    font: 600 12.5px/1.2 var(--sans);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tname small {
    font: 450 11px/1.2 var(--sans);
    color: var(--faint);
  }
  .tleft {
    font: 600 15px/1 var(--mono);
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .tleft.paused {
    color: var(--dim);
  }
  .tbtns {
    display: flex;
    gap: 4px;
  }
  .rots.quick {
    grid-template-columns: repeat(6, 1fr);
    margin-bottom: 10px;
  }
  .rots.three {
    grid-template-columns: repeat(3, 1fr);
  }
  .rots.quick .rot {
    justify-content: center;
  }
  .tform {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .tform input {
    height: 30px;
    padding: 0 9px;
    border-radius: 9px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.04);
    color: var(--text);
    font: 500 12px/1 var(--sans);
    color-scheme: dark;
  }
  .tform input:focus-visible {
    outline: 1.5px solid var(--accent);
    outline-offset: -1px;
  }
  .tform .tnum {
    width: 58px;
    font-family: var(--mono);
  }
  .tform .ttext {
    flex: 1;
    min-width: 120px;
  }
  .tunit {
    font: 500 11.5px/1 var(--sans);
    color: var(--faint);
    margin-right: 4px;
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
  .voice.dimmed .persona {
    opacity: 0.5;
  }
  .switch.master {
    align-items: flex-start;
    margin-bottom: 14px;
  }
  .switch.master .track {
    flex: none;
    margin-top: 1px;
  }
  .switch.master .slabel {
    font-size: 14px;
  }
  .pills {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: -2px 0 12px 42px;
  }
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 9px 4px 8px;
    border-radius: 999px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.025);
    font: 550 11px/1 var(--sans);
    color: var(--dim);
  }
  .pill .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--faint);
  }
  .pill.ready .dot {
    background: #5ee38a;
    box-shadow: 0 0 6px #5ee38a;
  }
  .pill.starting .dot {
    background: var(--warm);
    animation: blink 1s ease-in-out infinite;
  }
  .pill.error {
    color: var(--hot);
  }
  .pill.error .dot {
    background: var(--hot);
  }
  .pill.board {
    color: var(--faint);
    font-family: var(--mono);
    font-size: 10.5px;
  }
  @keyframes blink {
    50% {
      opacity: 0.3;
    }
  }

  .persona {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(118px, 1fr));
    gap: 6px;
    margin-bottom: 10px;
  }
  .pcard {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 9px 10px;
    border-radius: 11px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.02);
    color: var(--text);
    text-align: left;
    cursor: pointer;
    transition:
      border-color 180ms,
      background 180ms;
  }
  .pcard:hover {
    background: rgba(255, 255, 255, 0.045);
  }
  .pcard.checked {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .pcard:focus-visible {
    outline: 1.5px solid var(--accent);
  }
  .pcard b {
    font: 600 12.5px/1.2 var(--sans);
  }
  .pcard small {
    font: 450 11px/1.3 var(--sans);
    color: var(--faint);
  }
  .custom {
    position: relative;
    margin-bottom: 10px;
  }
  .custom textarea {
    width: 100%;
    resize: vertical;
    padding: 9px 11px 20px;
    border-radius: 10px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.04);
    color: var(--text);
    font: 450 12.5px/1.45 var(--sans);
    user-select: text;
  }
  .custom textarea:focus-visible {
    outline: 1.5px solid var(--accent);
    outline-offset: -1px;
  }
  .custom .pmeta {
    position: absolute;
    right: 10px;
    bottom: 9px;
  }

  .recbox {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    margin-bottom: 12px;
    padding: 10px 12px;
    border-radius: 12px;
    border: 1px dashed color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .recbox .hint {
    margin: 0;
  }
  .mrow {
    padding: 12px 0;
    border-top: 1px solid var(--line);
  }
  .mrow:first-of-type {
    border-top: 0;
    padding-top: 2px;
  }
  .mrow.off {
    opacity: 0.45;
  }
  .mhead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-bottom: 8px;
  }
  .select {
    position: relative;
  }
  .select select {
    width: 100%;
    height: 34px;
    padding: 0 32px 0 11px;
    appearance: none;
    -webkit-appearance: none;
    border-radius: 10px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.045);
    color: var(--text);
    font: 500 12.5px/1 var(--sans);
    cursor: pointer;
    transition:
      border-color 180ms,
      background 180ms;
  }
  .select select:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.07);
  }
  .select select:focus-visible {
    outline: 1.5px solid var(--accent);
    outline-offset: -1px;
  }
  .select select:disabled {
    cursor: default;
  }
  .select svg {
    position: absolute;
    right: 10px;
    top: 50%;
    width: 14px;
    height: 14px;
    translate: 0 -50%;
    color: var(--dim);
    pointer-events: none;
  }
  .mstatus {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-top: 8px;
  }
  .chip {
    padding: 4px 8px;
    border-radius: 999px;
    font: 600 10.5px/1 var(--sans);
    white-space: nowrap;
  }
  .chip.ready {
    color: #5ee38a;
    background: rgba(94, 227, 138, 0.1);
  }
  .chip.online {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .hint.small {
    margin: 6px 0 0;
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .more {
    margin-top: 6px;
    padding-top: 12px;
    border-top: 1px solid var(--line);
  }
  .more summary {
    display: flex;
    align-items: center;
    cursor: pointer;
    font: 550 12.5px/1 var(--sans);
    color: var(--dim);
    list-style: none;
  }
  .more summary::-webkit-details-marker {
    display: none;
  }
  .more summary::before {
    content: "›";
    display: inline-block;
    width: 14px;
    margin-right: 4px;
    transition: rotate 180ms;
  }
  .more[open] summary::before {
    rotate: 90deg;
  }
  .more summary .pmeta {
    margin-left: auto;
  }
  .more[open] summary {
    margin-bottom: 12px;
    color: var(--text);
  }
  .provider {
    padding: 11px 12px;
    margin-bottom: 6px;
    border-radius: 12px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.02);
  }
  .phead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-bottom: 8px;
  }
  .tform .key {
    font-family: var(--mono);
  }
  .rots.sub {
    grid-template-columns: 1fr 1fr;
  }
  .rots.sub .rot {
    justify-content: center;
    font-family: var(--sans);
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
  .mtext {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
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

  .claude.dimmed {
    opacity: 0.7;
  }
  .claude .facts {
    margin-bottom: 12px;
  }
  .hint.error {
    color: var(--hot);
  }
  .mcp .facts,
  .alerts .facts {
    margin-bottom: 12px;
  }
  .alerts {
    /* Claude's orange, as on the claude face. */
    --accent: #d97757;
  }
  .alerts .rotation {
    margin-bottom: 12px;
  }
  .actions.inline {
    margin: 0;
  }
  .alerts .rot:disabled {
    opacity: 0.45;
    cursor: default;
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
