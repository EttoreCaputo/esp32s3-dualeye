<script module lang="ts">
  export type Tab = "voice" | "pet" | "timers" | "claude" | "device" | "diagnostics";
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
    type Transport,
    type WifiInfo,
    type WifiNetwork,
    type WifiStatus,
    type PowerHelper,
    type Personality,
    type QuipLevel,
    type QuipsInfo,
    type Reading,
    type SttLanguage,
    type ShowOn,
    type TimerInfo,
    type TimerKind,
    type TtsEngine,
    type ModelInfo,
    type AccountVoice,
    type VoiceInfo,
    type VoiceSettings,
  } from "./monitor.svelte";
  import { VOICE_PROMPTS } from "./voicePrompts";
  import PetGallery from "./PetGallery.svelte";
  import PetMood from "./PetMood.svelte";
  import InfoTip from "./InfoTip.svelte";
  import { i18n, LANGUAGES, type Lang } from "./i18n/index.svelte";

  // `tx` and not `t`: timers are `t` here.
  const tx = $derived(i18n.t.settings);
  const tc = $derived(tx.common);
  const TABS: Tab[] = ["voice", "pet", "timers", "claude", "device", "diagnostics"];

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

  // The Device tab's connection: USB, WiFi or both, and the board's WiFi.
  const TRANSPORTS: Transport[] = ["auto", "usb", "wifi"];
  let wifi = $state<WifiInfo | null>(null);
  let wifiBusy = $state<"" | "scan" | "join" | "toggle" | "forget" | "pair" | "transport">("");
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
    if (!open || tab !== "device") return;
    loadWifi();
    const id = setInterval(loadWifi, 3000);
    return () => clearInterval(id);
  });

  const transport = $derived(wifi?.transport ?? "auto");
  const onWifi = $derived(monitor.link === "connected" && !!monitor.port?.startsWith("WiFi"));
  const pairedHere = $derived(!!wifi?.board && wifi.paired.some((p) => p.id === wifi?.board?.id));

  function wifiLabel(b: WifiStatus): string {
    const net = b.ssid ? `“${b.ssid}”` : "";
    const say = tx.device.state;
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

  const setTransport = (t: Transport) => wifiDo("transport", () => monitor.setTransport(t));

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
  const TIMER_COLORS: Record<TimerKind, string> = {
    timer: "#f8a639",
    work: "#ff6347",
    break: "#40e080",
    reminder: "#3ae7ed",
  };
  const clockText = (secs: number) => {
    const two = (n: number) => String(n).padStart(2, "0");
    return secs >= 3600 ? `${Math.trunc(secs / 3600)}:${two(Math.trunc((secs % 3600) / 60))}:${two(secs % 60)}` : `${Math.trunc(secs / 60)}:${two(secs % 60)}`;
  };
  const timerSub = (t: TimerInfo) => {
    if (t.state === "ring") return tx.timers.ringing;
    if (t.state === "pause") return tx.timers.paused;
    return t.ends_at ? tx.timers.until(t.ends_at) : "";
  };
  const SHOW_ON: ShowOn[] = ["left", "right", "none"];

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

  const PERSONALITIES: Personality[] = ["cute", "playful", "calm", "sassy", "butler", "minimal", "custom"];
  /** `agent::MAX_PERSONALITY`. */
  const MAX_PERSONALITY = 300;
  const ENGINE_NAMES: Record<TtsEngine, string> = { piper: "Piper" };
  /** A note from the backend, in the app's language when there's a translation. */
  const note = (text: string) => tx.voice.notes[text] ?? text;

  /** A model as its list shows it: size and whether it's here, or whether its service has a key. */
  function optionLabel(v: VoiceInfo, m: ModelInfo): string {
    if (m.provider) {
      // ElevenLabs' voices by their names.
      const name = accountVoices?.find((a) => m.id === accountId(a))?.name ?? m.id.slice(m.id.indexOf(":") + 1);
      return `${name}${m.installed ? "" : tx.voice.needsKey(providerName(v, m.provider))}`;
    }
    return `${m.id} · ${mb(m.bytes)}${m.installed ? tx.voice.downloaded : ""}${m.id === v.recommendation.whisper || m.id === v.recommendation.llm ? tx.voice.recommended : ""}`;
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

  /** The ElevenLabs account's voices, once read; null before. */
  let accountVoices = $state<AccountVoice[] | null>(null);
  let elevenError = $state("");
  const hasElevenKey = $derived(!!voice?.providers.find((p) => p.id === "elevenlabs")?.key);

  $effect(() => {
    if (!open || tab !== "voice" || !hasElevenKey) return;
    loadAccountVoices();
  });

  async function loadAccountVoices() {
    elevenError = "";
    try {
      accountVoices = await monitor.elevenlabsVoices();
    } catch (e) {
      elevenError = String(e);
      accountVoices = [];
    }
  }

  const accountId = (a: AccountVoice) => `elevenlabs:voice/${a.voice_id}`;
  const speaking = (v: VoiceInfo, id: string) => Object.values(v.settings.voices).includes(id);
  /** The account's own voices, not ElevenLabs' stock ones. */
  const ownVoices = $derived((accountVoices ?? []).filter((a) => a.category !== "premade"));

  /** The account's voices, for the voice lists next to the downloaded ones. */
  const accountModels = $derived<ModelInfo[]>(
    (accountVoices ?? []).map((a) => ({
      id: accountId(a),
      kind: "voice",
      language: null,
      engine: null,
      bytes: 0,
      note: a.category === "premade" ? tx.voice.elevenOwn : a.description || tx.voice.elevenAccount,
      license: "",
      installed: true,
      provider: "elevenlabs",
    })),
  );

  /** An ElevenLabs voice speaks both languages: it answers in either. */
  async function useAccountVoice(id: string) {
    if (!voice) return;
    elevenError = "";
    await setVoice({ voices: { ...voice.settings.voices, it: id, en: id } });
  }

  // The ElevenLabs prompt of a personality: shown while its icon is hovered, kept open by a click.
  let hoverPrompt = $state<Personality | null>(null);
  let pinnedPrompt = $state<Personality | null>(null);
  const shownPrompt = $derived(hoverPrompt ?? pinnedPrompt);
  let promptCopied = $state<string | null>(null);
  async function copyPrompt(text: string, what: string) {
    try {
      await navigator.clipboard.writeText(text);
      promptCopied = what;
      setTimeout(() => promptCopied === what && (promptCopied = null), 1500);
    } catch {
      promptCopied = null;
    }
  }

  // The speaker's volume and the eyes live on the board: read them when the tab opens.
  let volume = $state<number | null>(null);
  let eyes = $state<boolean | null>(null);
  let idleEyes = $state<boolean | null>(null);
  let wakeSound = $state<boolean | null>(null);
  $effect(() => {
    if (!open || (tab !== "voice" && tab !== "pet") || monitor.link !== "connected") return;
    let alive = true;
    monitor
      .boardVoice()
      .then((b) => {
        if (!alive) return;
        volume = b.volume;
        eyes = b.eyes;
        idleEyes = b.idle_eyes;
        wakeSound = b.wake_sound;
      })
      .catch(() => {
        if (!alive) return;
        volume = null;
        eyes = null;
        idleEyes = null;
        wakeSound = null;
      });
    return () => {
      alive = false;
    };
  });

  async function setWakeSound(on: boolean) {
    wakeSound = on;
    try {
      await monitor.setBoardWakeSound(on);
    } catch (e) {
      wakeSound = !on;
      voiceError = String(e);
    }
  }

  // The pet's mood: the monitor reads it every few seconds.
  const pet = $derived(monitor.pet);
  let petError = $state("");

  async function setPet(key: "sounds" | "reactions" | "hearing", on: boolean) {
    if (!monitor.pet) return;
    monitor.pet[key] = on;
    petError = "";
    try {
      await monitor.setBoardPet(key === "sounds" ? { sounds: on } : key === "reactions" ? { react: on } : { hear: on });
    } catch (e) {
      if (monitor.pet) monitor.pet[key] = !on;
      petError = String(e);
    }
  }

  // Its comments on your day, kept by the app.
  let quips = $state<QuipsInfo | null>(null);
  const QUIP_LEVELS: QuipLevel[] = ["off", "rare", "often"];

  $effect(() => {
    if (!open || tab !== "pet") return;
    let alive = true;
    // Whether it can speak: its comments need spoken replies on.
    monitor.voiceInfo().then((v) => alive && (voice = v));
    const load = () => monitor.quipsInfo().then((q) => alive && (quips = q));
    load();
    const id = setInterval(load, 5000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  });

  async function setQuips(level: QuipLevel) {
    petError = "";
    try {
      quips = await monitor.setQuips(level);
    } catch (e) {
      petError = String(e);
    }
  }

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
  const ENGINES: [TtsEngine, string, () => string][] = [["piper", "Piper", () => tx.voice.piperNote]];

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
    const gpu = hw.gpu.kind === "apple" ? tx.voice.gpuApple : hw.gpu.kind === "nvidia" ? `${hw.gpu.name} (${gb(hw.gpu.memory_mb)})` : tx.voice.gpuNone;
    return tx.voice.hardware(hw.cpu, hw.cores, gb(hw.memory_mb), gpu);
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

  const STT_LANGUAGES: [SttLanguage, string][] = [
    ["auto", "Auto"],
    ["it", "Italiano"],
    ["en", "English"],
  ];
  const mb = (n: number) => `${Math.round(n / 1_000_000)} MB`;
  const clock = (ms: number) => i18n.time(ms, true);
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
  const ago = (s: number) => tc.ago(s);

  // The port esptool will use: the pinned one, else the only Espressif device plugged in.
  const boards = $derived(ports.filter((p) => p.is_board));
  const target = $derived(monitor.portSetting ?? (boards.length === 1 ? boards[0].name : null));
  const busy = $derived(monitor.job !== "idle");

  const onBoard = $derived.by(() => {
    const fw = monitor.boardFirmware;
    const d = tx.device;
    if (monitor.link !== "connected") return { text: "—", note: tc.offline };
    if (!fw) return { text: "—", note: d.asking2 };
    if (fw.state === "version") return { text: `v${fw.version}`, note: fw.idf ? `IDF ${fw.idf}` : "" };
    if (fw.state === "legacy") return { text: d.before020, note: d.unversioned };
    return { text: d.none, note: d.blank };
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
  const kb = (n: number) => `${i18n.number(Math.round(n / 1024))} KB`;
  const seconds = (ms: number) => `${i18n.number(ms / 1000, { minimumFractionDigits: 1, maximumFractionDigits: 1 })} s`;

  /** The app's language: a language, or `system` to follow the computer's. */
  const langChoice = $derived(i18n.choice ?? "system");
  const systemName = $derived(LANGUAGES.find((l) => l.id === i18n.system)?.name ?? "English");
  const pickLang = (value: string) => i18n.set(value === "system" ? null : (value as Lang));
  const fmt = (r: Reading) => (r.unit === "RPM" || r.unit === "%" || r.unit === "MB" ? r.value.toFixed(0) : r.value.toFixed(1));
</script>

<!-- One model to pick, from this computer's or an online service's, and what it still needs. -->
{#snippet picker(v: VoiceInfo, label: string, what: string, kind: ModelInfo["kind"], lang: string | null, chosen: string, onpick: (id: string) => void, off: boolean)}
  {@const list = [...v.models, ...accountModels].filter((m) => m.kind === kind && (lang === null || m.language === lang || m.language === null))}
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
          title={tx.voice.testTitle}
          onclick={() => testVoice(lang)}>{testing === lang ? tx.voice.testing : tx.voice.test}</button
        >
      {/if}
    </div>
    <div class="select">
      <select value={chosen} disabled={off} aria-label={label} onchange={(e) => onpick(e.currentTarget.value)}>
        {#if !m}<option value={chosen} disabled>{tx.voice.choose}</option>{/if}
        <optgroup label={tx.voice.onComputer}>
          {#each list.filter((o) => !o.provider) as o (o.id)}
            <option value={o.id}>{optionLabel(v, o)}</option>
          {/each}
        </optgroup>
        {#each v.providers as p (p.id)}
          {@const cloud = list.filter((o) => o.provider === p.id)}
          {#if cloud.length}
            <optgroup label={tx.voice.onlineGroup(p.name)}>
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
          {note(m.note)}
        </span>
        <span class="mside">
          {#if m.provider}
            {#if m.installed}
              <span class="chip online" title={tx.voice.onlineTitle}>{tc.online}</span>
            {:else}
              <button class="btn small primary" onclick={() => (keysOpen = true)}>{tx.voice.addKey(providerName(v, m.provider))}</button>
            {/if}
          {:else if downloading}
            <button class="btn small" onclick={() => monitor.cancelDownload()}>{tx.voice.stop(Math.round(v.download?.[1] ?? 0))}</button>
          {:else if !m.installed}
            <button class="btn small primary" disabled={!!v.download} onclick={() => download(m.id)}>{tx.voice.download(mb(m.bytes))}</button>
          {:else if m.engine && !v[m.engine]}
            <button class="btn small primary" disabled={!!installing} onclick={() => m.engine && installEngine(m.engine)}
              >{installing ? tc.installing : tx.voice.installEngine(ENGINE_NAMES[m.engine])}</button
            >
          {:else}
            <span class="chip ready">{tc.ready}</span>
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
  <button class="scrim" transition:fade={{ duration: 200 }} onclick={() => (open = false)} aria-label={tc.close}></button>
  <aside
    class="drawer"
    class:resizing
    style:--w="{width}px"
    transition:fly={{ x: 40, duration: 320, easing: cubicOut, opacity: 0 }}
    aria-label={tc.title}
  >
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="grip" onpointerdown={startResize} ondblclick={resetWidth} title={tc.resize}></div>
    <header class="dhead">
      <h2>{tc.title}</h2>
      <label class="lang" title={tc.language}>
        <svg viewBox="0 0 16 16" aria-hidden="true"
          ><circle cx="8" cy="8" r="6.3" /><path d="M1.8 8h12.4M8 1.7c-1.9 1.8-2.8 3.9-2.8 6.3s.9 4.5 2.8 6.3M8 1.7c1.9 1.8 2.8 3.9 2.8 6.3s-.9 4.5-2.8 6.3" /></svg
        >
        <select value={langChoice} aria-label={tc.language} onchange={(e) => pickLang(e.currentTarget.value)}>
          <option value="system">{tc.system(systemName)}</option>
          {#each LANGUAGES as l (l.id)}
            <option value={l.id}>{l.name}</option>
          {/each}
        </select>
      </label>
      <button class="dclose" onclick={() => (open = false)} aria-label={tc.close} title={tc.close}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" /></svg>
      </button>
    </header>
    <nav>
      {#each TABS as key (key)}
        <button class:active={tab === key} onclick={() => (tab = key)}>{tx.tabs[key]}</button>
      {/each}
      <span class="indicator" style:--i={TABS.indexOf(tab)}></span>
    </nav>

    <div class="content">
      {#if tab === "voice"}
        {#if !voice}
          <p class="empty">{tc.loading}</p>
        {:else}
          {@const v = voice}
          <section class="voice">
            <label class="switch master">
              <input type="checkbox" checked={v.settings.enabled} onchange={(e) => setVoice({ enabled: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="mtext">
                <span class="slabel">{tx.voice.assistant} <InfoTip text={tx.voice.assistantTip} /></span>
              </span>
            </label>
            {#if v.settings.enabled}
              <div class="pills">
                {#each [[tx.voice.hearing, v.stt], [tx.voice.understanding, v.settings.llm ? v.llm : "off"], [tx.voice.speaking, v.tts]] as [name, state] (name)}
                  <span class="pill {state}" title={tx.voice.status[state as keyof typeof tx.voice.status]}><span class="dot"></span>{name}</span>
                {/each}
                <span class="pill board" title={tx.voice.boardStateTitle}>{tx.voice.board(monitor.link === "connected" ? monitor.voice : tc.offline)}</span>
              </div>
              {#each [v.stt === "error" && v.stt_error, v.tts === "error" && v.tts_error, v.settings.llm && v.llm === "error" && v.llm_error && tx.voice.llmFallback(v.llm_error)] as err, i (i)}
                {#if err}<p class="hint error">{err}</p>{/if}
              {/each}
              {#if v.tts_fallback}
                <p class="hint">{tx.voice.ttsFallback(v.tts_fallback)}</p>
              {/if}
            {/if}
            <div class="volume">
              <span class="rlabel">{tx.voice.volume}</span>
              <input
                type="range"
                min="0"
                max="100"
                step="5"
                value={volume ?? 60}
                disabled={volume === null}
                aria-label={tx.voice.volumeAria}
                onchange={(e) => setVolume(Number(e.currentTarget.value))}
              />
              <span class="pmeta">{volume === null ? "—" : `${volume}%`}</span>
            </div>
            <div class="rotation">
              <span class="rlabel">{tx.voice.language} <InfoTip text={tx.voice.languageTip} /></span>
              <div class="rots langs" role="radiogroup" aria-label={tx.voice.language}>
                {#each STT_LANGUAGES as [id, label] (id)}
                  <button
                    class="rot"
                    class:checked={v.settings.language === id}
                    role="radio"
                    aria-checked={v.settings.language === id}
                    title={id === "auto" ? tx.voice.autoTitle : tx.voice.always(label)}
                    onclick={() => setVoice({ language: id })}>{id === "auto" ? tx.voice.auto : label}</button
                  >
                {/each}
              </div>
            </div>
          </section>

          <section class="voice" class:dimmed={!v.settings.llm}>
            <h3>{tx.voice.personality} <InfoTip text={tx.voice.personalityTip} /></h3>
            {#if !v.settings.llm}<p class="hint need">{tx.voice.personalityNeedsLlm}</p>{/if}
            <div class="persona" role="radiogroup" aria-label={tx.voice.personality}>
              {#each PERSONALITIES as id (id)}
                {@const [name, blurb] = tx.voice.personalities[id]}
                <div class="pslot">
                  <button class="pcard" class:checked={v.settings.personality === id} role="radio" aria-checked={v.settings.personality === id} onclick={() => setVoice({ personality: id })}>
                    <b>{name}</b>
                    <small>{blurb}</small>
                  </button>
                  {#if VOICE_PROMPTS[id]}
                    <button
                      class="vicon"
                      class:on={shownPrompt === id}
                      aria-label={tx.voice.promptAria(name)}
                      aria-expanded={pinnedPrompt === id}
                      title={tx.voice.promptTitle}
                      onmouseenter={() => (hoverPrompt = id)}
                      onmouseleave={() => (hoverPrompt = null)}
                      onfocus={() => (hoverPrompt = id)}
                      onblur={() => (hoverPrompt = null)}
                      onclick={() => (pinnedPrompt = pinnedPrompt === id ? null : id)}
                    >
                      <svg viewBox="0 0 16 16" aria-hidden="true"
                        ><path d="M2 6.5v3M5 4v8M8 2.5v11M11 4.5v7M14 6.5v3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" /></svg
                      >
                    </button>
                  {/if}
                </div>
              {/each}
            </div>
            {#if shownPrompt && VOICE_PROMPTS[shownPrompt]}
              {@const p = VOICE_PROMPTS[shownPrompt]}
              {@const label = tx.voice.personalities[shownPrompt][0]}
              <div class="vprompt">
                <div class="vphead">
                  <b>{tx.voice.promptHead(label)}</b>
                  {#if pinnedPrompt}<button class="btn small" onclick={() => (pinnedPrompt = null)}>{tc.closeBtn}</button>{/if}
                </div>
                <p class="hint">{tx.voice.promptSteps(p.name)}</p>
                <div class="vpfield">
                  <span class="rlabel">{tx.voice.prompt}</span>
                  <button class="btn small" onclick={() => copyPrompt(p.description, "description")}>{promptCopied === "description" ? tc.copied : tc.copy}</button>
                </div>
                <p class="vptext">{p.description}</p>
                <div class="vpfield">
                  <span class="rlabel">{tx.voice.previewText}</span>
                  <button class="btn small" onclick={() => copyPrompt(p.sample, "sample")}>{promptCopied === "sample" ? tc.copied : tc.copy}</button>
                </div>
                <p class="vptext">{p.sample}</p>
              </div>
            {/if}
            {#if v.settings.personality === "custom"}
              <div class="custom">
                <textarea
                  rows="3"
                  maxlength={MAX_PERSONALITY}
                  placeholder={tx.voice.customPlaceholder}
                  value={v.settings.personality_custom}
                  oninput={(e) => (customLength = e.currentTarget.value.length)}
                  onchange={(e) => setVoice({ personality_custom: e.currentTarget.value })}
                ></textarea>
                <span class="pmeta">{customLength ?? v.settings.personality_custom.length}/{MAX_PERSONALITY}</span>
              </div>
            {/if}
          </section>

          <section class="voice" class:dimmed={!v.settings.speak}>
            <h3>{tx.voice.eleven} <InfoTip text={tx.voice.elevenTip} /></h3>
            {#if !hasElevenKey}
              <div class="actions">
                <button class="btn small primary" onclick={() => (keysOpen = true)}>{tx.voice.elevenAdd}</button>
                <InfoTip text={tx.voice.elevenKeyTip} />
              </div>
            {:else}
              {#if accountVoices === null}
                <p class="hint small">{tx.voice.elevenReading}</p>
              {:else if !ownVoices.length}
                <p class="hint">{tx.voice.elevenNone}</p>
              {:else}
                <div class="ports">
                  {#each ownVoices as a (a.voice_id)}
                    {@const inUse = speaking(v, accountId(a))}
                    <div class="port model" class:checked={inUse}>
                      <span class="mtext">
                        <span class="pname">{a.name}</span>
                        <span class="mnote">{a.description || a.category}</span>
                      </span>
                      <span class="mside">
                        {#if inUse}
                          <span class="pmeta">{tx.voice.speakingNow}</span>
                        {:else}
                          <button class="btn small" onclick={() => useAccountVoice(accountId(a))}>{tx.voice.use}</button>
                        {/if}
                      </span>
                    </div>
                  {/each}
                </div>
              {/if}
              <div class="vpfield">
                <p class="hint">{tx.voice.bothLanguages}</p>
                <button class="btn small" onclick={loadAccountVoices}>{tc.refresh}</button>
              </div>
            {/if}
            {#if elevenError}<p class="hint error">{elevenError}</p>{/if}
          </section>

          <section class="voice">
            <h3>{tx.voice.behaviour}</h3>
            <label class="switch">
              <input type="checkbox" checked={v.settings.speak} onchange={(e) => setVoice({ speak: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">{tx.voice.speak}</span>
            </label>
            <label class="switch">
              <input type="checkbox" checked={v.settings.follow_up} disabled={!v.settings.speak} onchange={(e) => setVoice({ follow_up: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">{tx.voice.followUp} <InfoTip text={tx.voice.followUpTip} /></span>
            </label>
            <label class="switch">
              <input type="checkbox" checked={v.settings.pause_music} onchange={(e) => setVoice({ pause_music: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">{tx.voice.pauseMusic}</span>
            </label>
            <label class="switch">
              <input type="checkbox" checked={v.settings.keep_warm} onchange={(e) => setVoice({ keep_warm: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">{tx.voice.keepWarm} <InfoTip text={tx.voice.keepWarmTip} /></span>
            </label>
            <label class="switch" title={wakeSound === null ? tc.needsFirmware("1.3.3") : ""}>
              <input type="checkbox" checked={wakeSound ?? true} disabled={wakeSound === null} onchange={(e) => setWakeSound(e.currentTarget.checked)} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">{tx.voice.wakeSound}</span>
            </label>
            <label class="switch">
              <input type="checkbox" checked={v.settings.llm} onchange={(e) => setVoice({ llm: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">{tx.voice.llm} <InfoTip text={tx.voice.llmTip} /></span>
            </label>
            <label class="switch" title={eyes === null ? tc.needsFirmware("1.0.1") : ""}>
              <input type="checkbox" checked={eyes ?? true} disabled={eyes === null} onchange={(e) => setEyes(e.currentTarget.checked)} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">{tx.voice.eyes} <InfoTip text={tx.voice.eyesTip} /></span>
            </label>
          </section>

          <section class="voice">
            <h3>{tx.voice.models}</h3>
            {#if !usesRecommended(v)}
              <div class="recbox">
                <p class="hint">{v.recommendation.why}</p>
                <button class="btn small" onclick={useRecommended}>
                  {tx.voice.useRecommended(v.recommendation.whisper, v.recommendation.llm)}
                </button>
              </div>
            {/if}
            {@render picker(v, tx.voice.hearing, tx.voice.hearingWhat, "whisper", null, v.settings.model, (id) => setVoice({ model: id }), false)}
            {@render picker(v, tx.voice.understanding, tx.voice.understandingWhat, "llm", null, v.settings.llm_model, (id) => setVoice({ llm_model: id }), !v.settings.llm)}
            {#each VOICE_LANGUAGES as [lang, label] (lang)}
              {@render picker(v, tx.voice.voiceFor(label), tx.voice.voiceWhat, "voice", lang, v.settings.voices[lang] ?? "", (id) => setVoice({ voices: { ...v.settings.voices, [lang]: id } }), !v.settings.speak)}
            {/each}
            {#if voiceError}<p class="hint error">{voiceError}</p>{/if}
            {#if engineError}<p class="hint error">{engineError}</p>{/if}

            <details class="more" bind:open={keysOpen}>
              <summary>{tx.voice.services} <InfoTip text={tx.voice.servicesTip} /> <span class="pmeta">{v.providers.filter((p) => p.key).length ? tx.voice.keySaved : tx.voice.noKey}</span></summary>
              {#each v.providers as p (p.id)}
                <div class="provider">
                  <div class="phead">
                    <span class="mtext"><span class="pname">{p.name}</span><span class="mnote">{note(p.note)}</span></span>
                    {#if p.key === "env"}
                      <span class="pmeta" title={tx.voice.envTitle}>{tx.voice.envKey(p.key_env)}</span>
                    {:else if p.key === "saved"}
                      <button class="btn small" disabled={savingKey === p.id} onclick={() => saveKey(p.id, null)}>{tx.voice.forgetKey}</button>
                    {/if}
                  </div>
                  {#if p.key !== "env"}
                    <div class="tform">
                      <input
                        class="ttext key"
                        type="password"
                        autocomplete="off"
                        spellcheck="false"
                        placeholder={p.key ? tx.voice.replaceKey : tx.voice.keyFrom(p.keys_url)}
                        aria-label={tx.voice.keyAria(p.name)}
                        bind:value={keyDrafts[p.id]}
                      />
                      <button class="btn small primary" disabled={!keyDrafts[p.id]?.trim() || savingKey === p.id} onclick={() => saveKey(p.id, keyDrafts[p.id])}
                        >{savingKey === p.id ? tx.voice.checking : tc.save}</button
                      >
                    </div>
                  {/if}
                </div>
              {/each}
              {#if keyError}<p class="hint error">{keyError}</p>{/if}
            </details>

            <details class="more">
              <summary>{tx.voice.advanced}</summary>
              <p class="hint">
                {hardwareLine(v.hardware)}. whisper-server: <code>{v.server ?? tx.voice.notFound}</code> · llama-server: <code>{v.llm_server ?? tx.voice.notFound}</code>
              </p>
              {#if !v.server}
                {@const w = tx.voice.whisperMissing}
                <p class="hint error">{w[0]}<code>tools/build_sidecars.sh</code>{w[1]}<code>brew install whisper-cpp</code>{w[2]}</p>
              {/if}
              {#if !v.llm_server}
                {@const w = tx.voice.llamaMissing}
                <p class="hint error">{w[0]}<code>tools/build_sidecars.sh</code>{w[1]}<code>brew install llama.cpp</code>{w[2]}</p>
              {/if}
              <div class="ports">
                {#each ENGINES as [engine, name, note] (engine)}
                  {@const installing = v[`${engine}_install`]}
                  <div class="port model" class:checked={!!v[engine]}>
                    <span class="mtext">
                      <span class="pname">{name}</span>
                      <span class="mnote">{installing ? tx.voice.installingWith(installing) : note()}</span>
                    </span>
                    <span class="mside">
                      {#if v[engine]}
                        <span class="pmeta">{tc.installed}</span>
                      {:else}
                        <button class="btn small primary" disabled={!!installing} onclick={() => installEngine(engine)}>{installing ? tc.installing : tc.install}</button>
                      {/if}
                    </span>
                  </div>
                {/each}
                {#each v.models.filter((m) => !m.provider && m.installed) as m (m.id)}
                  {@const used = [v.settings.model, v.settings.llm_model, ...Object.values(v.settings.voices)].includes(m.id)}
                  <div class="port model">
                    <span class="mtext">
                      <span class="pname">{m.id}</span>
                      <span class="mnote">{tx.voice.kinds[m.kind]} · {mb(m.bytes)}{used ? tx.voice.inUse : ""}</span>
                    </span>
                    <span class="mside">
                      <button class="btn small" disabled={used && v.settings.enabled} title={tx.voice.deleteTitle} onclick={() => removeModel(m.id)}>{tc.delete}</button>
                    </span>
                  </div>
                {/each}
              </div>
              <label class="check">
                <input type="checkbox" checked={v.settings.keep_recordings} onchange={(e) => setVoice({ keep_recordings: e.currentTarget.checked })} />
                <span>{tx.voice.keepRecordings}</span>
              </label>
            </details>
          </section>

          <section>
            <h3>{tx.voice.transcripts}</h3>
            {#each newestFirst as entry (entry.at + "-" + entry.id)}
              <div class="transcript">
                <span class="tmeta">{clock(entry.at)}</span>
                {#if entry.transcript}
                  <span class="tlang">{entry.transcript.language}</span>
                  <span class="ttext">{entry.transcript.text}</span>
                  <span class="tmeta">{seconds(entry.transcript.elapsed_ms)}</span>
                  {#if entry.reply}
                    <span class="treply" class:muted={!entry.reply.understood}>
                      → {entry.reply.text}
                      {#if entry.reply.by === "rules" && v.settings.llm}
                        <span class="taction" title={tx.voice.fixedTitle}>{tx.voice.fixedPhrase}</span>
                      {/if}
                      {#each entry.reply.actions as action, i (i)}
                        <span class="taction">{action}</span>
                      {/each}
                      {#if entry.spoken?.reason === "barge_in"}
                        <span class="taction" title={tx.voice.interruptedTitle}>{tx.voice.interrupted}</span>
                      {/if}
                    </span>
                    <span class="tmeta">{entry.spoken ? seconds(entry.spoken.first_audio_ms) : ""}</span>
                  {/if}
                {:else}
                  <span class="ttext muted">{tx.voice.noWords}</span>
                {/if}
              </div>
            {:else}
              <p class="empty">{v.settings.enabled ? tx.voice.emptyOn : tx.voice.emptyOff}</p>
            {/each}
          </section>
        {/if}
      {:else if tab === "claude"}
        <section class="claude" class:dimmed={!usesClaude}>
          <h3>{tx.claude.code} <InfoTip text={tx.claude.codeTip} /></h3>
          <p class="hint">
            {claudeUsage ? tx.claude.usage(formatTokens(claudeUsage.tok), formatTokens(claudeUsage.today)) : tx.claude.noUsage}
          </p>
          {#if claudeLink?.connected}
            <dl class="facts">
              <div><dt>{tx.claude.statusLine}</dt><dd>{tc.connected}</dd></div>
              <div>
                <dt>{tx.claude.lastUpdate}</dt>
                <dd>{claudeLink.last_update_s === null ? tc.waiting : ago(claudeLink.last_update_s)}</dd>
              </div>
              {#if claudeLink.chained}
                <div class="wide"><dt>{tx.claude.chained}</dt><dd class="small">{claudeLink.chained}</dd></div>
              {/if}
            </dl>
            {#if claudeLink.last_update_s === null}
              <p class="hint">{tx.claude.limitsLater}</p>
            {/if}
            <div class="actions">
              <button class="btn" disabled={claudeBusy} onclick={() => linkClaude(false)}>{tc.disconnect}</button>
            </div>
          {:else}
            <div class="actions">
              <button class="btn primary" disabled={claudeBusy || !claudeLink} onclick={() => linkClaude(true)}>{tx.claude.connect}</button>
              <InfoTip text={tx.claude.connectTip(claudeLink?.settings_path ?? "~/.claude/settings.json")} />
            </div>
          {/if}
          {#if claudeError}
            <p class="hint error">{claudeError}</p>
          {/if}
        </section>

        <section class="alerts">
          <h3>{tx.claude.alerts} <InfoTip text={tx.claude.alertsTip} /></h3>
          {#if alerts}
            {#if alerts.hooks.connected}
              <dl class="facts">
                <div><dt>{tx.claude.hooks}</dt><dd>{tc.connected}</dd></div>
                <div>
                  <dt>{tx.claude.lastEvent}</dt>
                  <dd>{alerts.hooks.last_event_s === null ? tc.waiting : ago(alerts.hooks.last_event_s)}</dd>
                </div>
                {#if alerts.last}
                  <div class="wide">
                    <dt>{tx.claude.lastAlert(ago(alerts.last.age_s))}</dt>
                    <dd class="small">{alerts.last.text}{alerts.last.error ? tx.claude.notGiven(alerts.last.error) : ""}</dd>
                  </div>
                {/if}
              </dl>
              {#if alerts.hooks.last_event_s === null}
                <p class="hint">{tx.claude.restart}</p>
              {/if}
            {/if}
            <label class="switch">
              <input type="checkbox" checked={alerts.settings.needs_you} onchange={(e) => setAlerts({ needs_you: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">{tx.claude.needsYou}</span>
            </label>
            <label class="switch">
              <input type="checkbox" checked={alerts.settings.done} onchange={(e) => setAlerts({ done: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">{tx.claude.done}</span>
            </label>
            <label class="switch">
              <input type="checkbox" checked={alerts.settings.usage} onchange={(e) => setAlerts({ usage: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">{tx.claude.usageAlert}</span>
            </label>
            <label class="switch">
              <input type="checkbox" checked={alerts.settings.speak} onchange={(e) => setAlerts({ speak: e.currentTarget.checked })} />
              <span class="track"><span class="knob"></span></span>
              <span class="slabel">{tx.claude.speak}</span>
            </label>
            <div class="rotation">
              <span class="rlabel">{tx.claude.longTask}</span>
              <div class="rots" role="radiogroup" aria-label={tx.claude.longTask}>
                {#each DONE_AFTER as secs (secs)}
                  <button
                    class="rot"
                    class:checked={alerts.settings.done_after_s === secs}
                    role="radio"
                    aria-checked={alerts.settings.done_after_s === secs}
                    disabled={!alerts.settings.done}
                    title={tx.claude.longTaskTitle}
                    onclick={() => setAlerts({ done_after_s: secs })}>{secs < 60 ? `${secs} s` : `${secs / 60} min`}</button
                  >
                {/each}
              </div>
            </div>
            <div class="rotation">
              <span class="rlabel">{tx.claude.language}</span>
              <div class="rots langs" role="radiogroup" aria-label={tx.claude.languageAria}>
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
              <p class="hint need">{tx.claude.cantSpeak}</p>
            {/if}
            <div class="actions">
              {#if alerts.hooks.connected}
                <button class="btn" disabled={alertsBusy || monitor.link !== "connected"} onclick={testAlert}>{tc.tryIt}</button>
                <button class="btn" disabled={alertsBusy} onclick={() => linkHooks(false)}>{tc.disconnect}</button>
              {:else}
                <button class="btn primary" disabled={alertsBusy} onclick={() => linkHooks(true)}>{tx.claude.connectHooks}</button>
                <InfoTip text={tx.claude.hooksTip(alerts.hooks.settings_path ?? "~/.claude/settings.json")} />
              {/if}
            </div>
          {/if}
          {#if alertsError}
            <p class="hint error">{alertsError}</p>
          {/if}
        </section>

        <section class="mcp">
          <h3>{tx.claude.mcp} <InfoTip text={tx.claude.mcpTip} /></h3>
          {#if mcp?.hub}
            <dl class="facts">
              <div>
                <dt>{tx.claude.clients}</dt>
                <dd>{tx.claude.clientsN(mcp.hub.clients)}</dd>
              </div>
              <div>
                <dt>{tx.claude.lastCall}</dt>
                <dd>{mcp.hub.last_tool === null || mcp.hub.last_call_age_ms === null ? tx.claude.noneYet : `${mcp.hub.last_tool}, ${ago(Math.round(mcp.hub.last_call_age_ms / 1000))}`}</dd>
              </div>
            </dl>
          {:else if mcp?.hub_error}
            <p class="hint error">{tx.claude.hubError(mcp.hub_error)}</p>
          {/if}
          {#if mcpCommand}
            <p class="hint">{tx.claude.forCode}</p>
            <div class="snippet">
              <pre>{mcpCommand}</pre>
              <button class="btn" onclick={() => copy("code")}>{copied === "code" ? tc.copied : tc.copy}</button>
            </div>
            <p class="hint">{tx.claude.forDesktop[0]}<code>claude_desktop_config.json</code>{tx.claude.forDesktop[1]}</p>
            <div class="snippet">
              <pre>{mcpDesktop}</pre>
              <button class="btn" onclick={() => copy("desktop")}>{copied === "desktop" ? tc.copied : tc.copy}</button>
            </div>
          {/if}
        </section>
      {:else if tab === "pet"}
        {#if pet}<PetMood {pet} />{/if}
        <section class="petset">
          <h3>{tx.pet.behaviour} <InfoTip text={tx.pet.behaviourTip} /></h3>
          <label class="switch" title={idleEyes === null ? tc.needsFirmware("1.0.2") : ""}>
            <input type="checkbox" checked={idleEyes ?? true} disabled={idleEyes === null} onchange={(e) => setIdleEyes(e.currentTarget.checked)} />
            <span class="track"><span class="knob"></span></span>
            <span class="slabel">{tx.pet.idleEyes}</span>
          </label>
          <label class="switch" title={!pet ? tc.needsFirmware("1.4.0") : ""}>
            <input type="checkbox" checked={pet?.reactions ?? true} disabled={!pet} onchange={(e) => setPet("reactions", e.currentTarget.checked)} />
            <span class="track"><span class="knob"></span></span>
            <span class="slabel">{tx.pet.reactions} <InfoTip text={tx.pet.reactionsTip} /></span>
          </label>
          <label class="switch" title={!pet ? tc.needsFirmware("1.4.0") : ""}>
            <input type="checkbox" checked={pet?.sounds ?? true} disabled={!pet} onchange={(e) => setPet("sounds", e.currentTarget.checked)} />
            <span class="track"><span class="knob"></span></span>
            <span class="slabel">{tx.pet.sounds} <InfoTip text={tx.pet.soundsTip} /></span>
          </label>
          <label class="switch" title={pet?.hearing === undefined ? tc.needsFirmware("1.4.2") : ""}>
            <input type="checkbox" checked={pet?.hearing ?? true} disabled={pet?.hearing === undefined} onchange={(e) => setPet("hearing", e.currentTarget.checked)} />
            <span class="track"><span class="knob"></span></span>
            <span class="slabel">{tx.pet.hearing} <InfoTip text={tx.pet.hearingTip} /></span>
          </label>
        </section>
        <section class="petset">
          <h3>{tx.pet.quips} <InfoTip text={tx.pet.quipsTip} /></h3>
          <div class="rotation">
            <span class="rlabel">{tx.pet.howOften}</span>
            <div class="rots" role="radiogroup" aria-label={tx.pet.howOftenAria}>
              {#each QUIP_LEVELS as level (level)}
                {@const [label, title] = tx.pet.levels[level]}
                <button class="rot" class:checked={quips?.level === level} role="radio" aria-checked={quips?.level === level} {title} disabled={!quips} onclick={() => setQuips(level)}
                  >{label}</button
                >
              {/each}
            </div>
          </div>
          {#if quips?.last}
            <p class="hint">{tx.pet.last(i18n.time(quips.last.at), quips.last.text)}{quips.last.error ? tx.pet.notSaid(quips.last.error) : ""}</p>
          {/if}
          {#if quips && quips.level !== "off" && voice && !voice.settings.speak}<p class="hint need">{tx.pet.needsSpeak}</p>{/if}
          {#if petError}<p class="hint error">{petError}</p>{/if}
        </section>
        <PetGallery connected={monitor.link === "connected"} petSounds={pet?.sounds ?? true} oldFirmware={monitor.link === "connected" && pet === null} />
      {:else if tab === "timers"}
        {@const info = monitor.timers}
        <section class="timers">
          <h3>{tx.timers.running} <InfoTip text={tx.timers.tip} /></h3>
          {#each info?.timers ?? [] as t (t.id)}
            {@const kind = tx.timers.kinds[t.kind]}
            {@const color = TIMER_COLORS[t.kind]}
            <div class="trow" class:ringing={t.state === "ring"} style:--tcolor={color}>
              <span class="tdot"></span>
              <div class="tname">
                <b>{t.label ?? kind}</b>
                <small>{t.label ? `${kind} · ` : ""}{timerSub(t)}</small>
              </div>
              <span class="tleft" class:paused={t.state === "pause"}>{clockText(t.left_s)}</span>
              <div class="tbtns">
                {#if t.state === "run"}<button class="btn small" onclick={() => controlTimer("pause", t)}>{tx.timers.pause}</button>{/if}
                {#if t.state === "pause"}<button class="btn small" onclick={() => controlTimer("resume", t)}>{tx.timers.resume}</button>{/if}
                <button class="btn small" onclick={() => controlTimer("cancel", t)}>{t.state === "ring" ? tx.timers.stop : tx.timers.cancel}</button>
              </div>
            </div>
          {:else}
            <p class="empty">{tx.timers.none}</p>
          {/each}
          {#if timerError}<p class="hint error">{timerError}</p>{/if}
        </section>
        <section class="timers">
          <h3>{tx.timers.new}</h3>
          <div class="rots quick">
            {#each [1, 3, 5, 10, 15, 30] as m (m)}
              <button class="rot" onclick={() => startTimer(m * 60)}>{m} {tx.timers.min}</button>
            {/each}
          </div>
          <div class="tform">
            <input class="tnum" type="number" min="1" max="1440" bind:value={timerMinutes} aria-label={tx.timers.minutes} />
            <span class="tunit">{tx.timers.min}</span>
            <input class="ttext" type="text" maxlength="40" placeholder={tx.timers.labelPlaceholder} bind:value={timerLabel} />
            <button class="btn primary" disabled={!(timerMinutes > 0)} onclick={() => startTimer(Math.round(timerMinutes * 60), timerLabel).then(() => (timerLabel = ""))}
              >{tc.start}</button
            >
          </div>
        </section>
        <section class="timers">
          <h3>{tx.timers.reminder}</h3>
          <div class="tform">
            <input class="ttext" type="text" maxlength="80" placeholder={tx.timers.remindPlaceholder} bind:value={remindText} />
            <input class="ttime" type="time" bind:value={remindAt} aria-label={tx.timers.at} />
            <button class="btn primary" disabled={!remindText.trim() || !remindAt} onclick={remind}>{tc.set}</button>
          </div>
        </section>
        <section class="timers">
          <h3>{tx.timers.pomodoro}</h3>
          {#if info?.pomodoro}
            {@const p = info.pomodoro}
            <p class="hint">{tx.timers.round(p.round, p.rounds, p.work_s / 60, p.break_s / 60)}</p>
            <div class="actions"><button class="btn" onclick={() => timerTool("pomodoro", { action: "stop" })}>{tx.timers.stopPomodoro}</button></div>
          {:else}
            <div class="tform">
              <input class="tnum" type="number" min="1" max="120" bind:value={pomo.work} aria-label={tx.timers.focusAria} />
              <span class="tunit">{tx.timers.minFocus}</span>
              <input class="tnum" type="number" min="1" max="60" bind:value={pomo.rest} aria-label={tx.timers.breakAria} />
              <span class="tunit">{tx.timers.minBreak}</span>
              <input class="tnum" type="number" min="1" max="12" bind:value={pomo.rounds} aria-label={tx.timers.roundsAria} />
              <span class="tunit">{tx.timers.rounds}</span>
              <button
                class="btn primary"
                onclick={() => timerTool("pomodoro", { action: "start", work_minutes: pomo.work, break_minutes: pomo.rest, rounds: pomo.rounds })}>{tc.start}</button
              >
            </div>
          {/if}
        </section>
        <section class="timers">
          <h3>{tx.timers.onBoard}</h3>
          <div class="rotation">
            <span class="rlabel">{tx.timers.takesOver} <InfoTip text={tx.timers.takesOverTip} /></span>
            <div class="rots three" role="radiogroup" aria-label={tx.timers.takesOverAria}>
              {#each SHOW_ON as value (value)}
                <button
                  class="rot"
                  class:checked={info?.show_on === value}
                  role="radio"
                  aria-checked={info?.show_on === value}
                  onclick={() => monitor.setTimerScreen(value)}>{tx.timers.showOn[value]}</button
                >
              {/each}
            </div>
          </div>
          {#if info && !info.can_speak}<p class="hint need" style:margin-top="10px">{tx.timers.needsSpeak}</p>{/if}
        </section>
      {:else if tab === "device"}
        <section>
          <h3>{tx.device.connection} <InfoTip text={tx.device.connectionTip} /></h3>
          <div class="ports">
            {#each TRANSPORTS as key (key)}
              {@const [name, note] = tx.device.transports[key]}
              <label class="port" class:checked={transport === key}>
                <input type="radio" name="transport" checked={transport === key} disabled={wifiBusy === "transport"} onchange={() => setTransport(key)} />
                <span class="radio"></span>
                <span class="pname">{name}</span>
                <span class="pmeta">{note}</span>
              </label>
            {/each}
          </div>
          {#if monitor.link === "connected" && monitor.port}
            <p class="hint conn">{onWifi ? tx.device.connectedWifi(monitor.port) : tx.device.connectedUsb(monitor.port)}</p>
          {/if}
          {#if transport !== "usb" && wifi && !wifi.paired.length}
            <p class="hint">{tx.device.noPaired}</p>
          {/if}
        </section>

        <section class="wifi">
          <h3>{tx.device.wifi} <InfoTip text={tx.device.setupTip} /></h3>
          {#if !wifi}
            <p class="empty">{tx.device.asking}</p>
          {:else if !wifi.board}
            <p class="empty">{wifi.error ?? tx.device.noAnswer}</p>
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
                <span class="slabel">{tx.device.wifiSwitch}</span>
              </label>
            {/if}
            {#if b.paired && !pairedHere}
              <p class="hint">{tx.device.otherComputer}</p>
              <div class="actions">
                <button class="btn" disabled={!!wifiBusy || onWifi} onclick={() => wifiDo("pair", () => monitor.wifiPair())}>
                  {wifiBusy === "pair" ? tx.device.pairing : tx.device.pair}
                </button>
              </div>
            {/if}
            {#if wifiForm || !b.ssid}
              {#if onWifi}
                <p class="hint error">{tx.device.plugUsb}</p>
              {:else}
                <div class="actions">
                  <button class="btn" disabled={!!wifiBusy} onclick={scanWifi}>{wifiBusy === "scan" ? tx.device.looking : networks ? tx.device.lookAgain : tx.device.find}</button>
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
                      <p class="empty">{tx.device.noNetwork}</p>
                    {/each}
                  </div>
                {/if}
                <div class="tform">
                  <input class="ttext" type="text" maxlength="32" spellcheck="false" placeholder={tx.device.networkName} aria-label={tx.device.networkName} bind:value={wifiSsid} />
                  <input
                    class="ttext"
                    type="password"
                    maxlength="64"
                    autocomplete="off"
                    placeholder={tx.device.password}
                    aria-label={tx.device.passwordAria}
                    bind:value={wifiPassword}
                  />
                  <button
                    class="btn small primary"
                    disabled={!!wifiBusy || !wifiSsid.trim() || (wifiPassword.length > 0 && wifiPassword.length < 8)}
                    onclick={joinWifi}>{wifiBusy === "join" ? tx.device.sending : tx.device.connect}</button
                  >
                </div>
              {/if}
            {:else if forgetting}
              <p class="hint">{tx.device.forgetConfirm(b.ssid)}</p>
              <div class="actions">
                <button class="btn primary" onclick={() => ((forgetting = false), wifiDo("forget", () => monitor.wifiForget()))}>{tx.device.forget}</button>
                <button class="btn" onclick={() => (forgetting = false)}>{tc.cancel}</button>
              </div>
            {:else}
              <div class="actions">
                <button class="btn" disabled={!!wifiBusy || onWifi} onclick={() => (wifiForm = true)}>{tx.device.change}</button>
                <button class="btn" disabled={!!wifiBusy || onWifi} onclick={() => (forgetting = true)}>{tx.device.forgetNetwork}</button>
              </div>
            {/if}
          {/if}
          {#if wifiError}<p class="hint error">{wifiError}</p>{/if}
        </section>

        {#if transport !== "wifi"}
        <section>
          <h3>{tx.device.serial} <InfoTip text={tx.device.serialTip} /></h3>
          <div class="ports">
            <label class="port" class:checked={monitor.portSetting === null}>
              <input type="radio" name="port" checked={monitor.portSetting === null} onchange={() => monitor.setPort(null)} />
              <span class="radio"></span>
              <span class="pname">{tx.device.automatic}</span>
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
              <p class="empty">{tx.device.noPorts}</p>
            {/each}
          </div>
        </section>
        {/if}

        {#if monitor.message && monitor.link !== "connected"}
          <section class="alert" class:error={monitor.link === "offline"}>
            <h3>{monitor.link === "offline" ? tx.device.lost : tx.device.waiting}</h3>
            <p class="mono">{monitor.message}</p>
            {#if monitor.permissionDenied}
              <p class="hint">{tx.device.linux[0]}<code>dialout</code>{tx.device.linux[1]}</p>
              <pre>sudo usermod -aG dialout "$USER"</pre>
            {/if}
          </section>
        {/if}

        <section>
          <h3>{tx.device.board} <InfoTip text={tx.device.boardTip} /></h3>
          {#if target}
            <div class="target">
              <span class="chipdot" class:found={monitor.chip?.port === target}></span>
              <span class="pname">{target}</span>
              <button class="btn" disabled={busy || !firmware} onclick={() => identify()}>
                {monitor.job === "identify" ? tx.device.reading : tx.device.identify}
              </button>
            </div>
          {:else if boards.length > 1}
            <p class="hint">{tx.device.several}</p>
          {:else}
            <p class="empty">{tx.device.noBoard}</p>
          {/if}
          {#if monitor.chip && monitor.chip.port === target}
            <dl class="facts chipinfo">
              <div class="wide"><dt>{tx.device.chip}</dt><dd>{monitor.chip.chip ?? "—"}</dd></div>
              <div><dt>{tx.device.flash}</dt><dd>{monitor.chip.flash_size ?? "—"}</dd></div>
              <div><dt>{tx.device.crystal}</dt><dd>{monitor.chip.crystal ?? "—"}</dd></div>
              <div class="wide"><dt>MAC</dt><dd>{monitor.chip.mac ?? "—"}</dd></div>
              {#if monitor.chip.features}<div class="wide"><dt>{tx.device.features}</dt><dd class="small">{monitor.chip.features}</dd></div>{/if}
            </dl>
          {/if}
        </section>

        <section>
          <h3>{tx.device.firmware}</h3>
          {#if !firmware}
            <p class="empty">{tx.device.checking}</p>
          {:else}
            <dl class="facts">
              <div><dt>{tx.device.onBoard}</dt><dd>{onBoard.text} {#if onBoard.note}<span class="muted sub">{onBoard.note}</span>{/if}</dd></div>
              <div>
                <dt>{tx.device.inApp}</dt>
                <dd>
                  {firmware.bundled ? `v${firmware.bundled.version}` : "—"}
                  {#if monitor.update}<span class="muted sub new">{tx.device.newer}</span>{:else if firmware.bundled}<span class="muted sub">IDF {firmware.bundled.idf}</span>{/if}
                </dd>
              </div>
              <div><dt>{tx.device.image}</dt><dd>merged-binary.bin</dd></div>
              <div><dt>{tx.device.size}</dt><dd>{kb(firmware.size)} · {tx.device.at0}</dd></div>
              <div class="wide">
                <dt>esptool</dt>
                {#if firmware.esptool}
                  <dd>v{firmware.esptool.version} <span class="muted">{tx.device.esptoolReady}</span></dd>
                {:else}
                  <dd class="small">{tx.device.esptoolSetup}</dd>
                {/if}
              </div>
            </dl>
            {#if monitor.update && monitor.update.changes.length && monitor.job !== "flash"}
              <div class="changes">
                <p class="hint">{tx.device.brings(monitor.update.to)}</p>
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
              <p class="hint setup">{tx.device.preparing(monitor.setup.message)}{monitor.setup.percent !== null ? ` ${Math.round(monitor.setup.percent)}%` : ""}</p>
            {:else if monitor.job === "flash"}
              <div class="progress" role="progressbar" aria-valuenow={Math.round(monitor.flashPercent)} aria-valuemin={0} aria-valuemax={100}>
                <span style:width="{monitor.flashPercent}%"></span>
              </div>
              <p class="hint">{tx.device.writing(Math.round(monitor.flashPercent))}</p>
            {:else if confirming}
              <p class="hint">{tx.device.confirm(target ?? "")}</p>
              <div class="actions">
                <button class="btn primary" onclick={flash}>{tx.device.flashNow}</button>
                <button class="btn" onclick={() => (confirming = false)}>{tc.cancel}</button>
              </div>
            {:else}
              <div class="actions">
                <button class="btn primary" disabled={busy || !target} onclick={() => (confirming = true)}>
                  {monitor.update ? tx.device.updateTo(monitor.update.to) : tx.device.flashFirmware}
                </button>
              </div>
            {/if}
          {/if}
          {#if monitor.jobError}
            <section class="alert error">
              <h3>{tx.device.failed}</h3>
              <p class="mono">{monitor.jobError}</p>
              {#if /busy|Errno 16/i.test(monitor.jobError)}
                <p class="hint">{tx.device.busyPort[0]}<code>idf.py monitor</code>{tx.device.busyPort[1]}<code>dualeye</code>{tx.device.busyPort[2]}</p>
              {/if}
            </section>
          {:else if monitor.flashedAt && monitor.job === "idle"}
            <p class="ok">{tx.device.flashed}</p>
          {/if}
          {#if monitor.jobLog.length}
            <details open={busy || !!monitor.jobError}>
              <summary>{tx.device.output}</summary>
              <div class="console joblog">
                {#each monitor.jobLog as line, i (i)}<div class="log">{line}</div>{/each}
              </div>
            </details>
          {/if}
        </section>
      {:else if tab === "diagnostics"}
        <div class="rots sub" role="tablist" aria-label={tx.diag.aria}>
          <button class="rot" class:checked={diag === "sensors"} role="tab" aria-selected={diag === "sensors"} onclick={() => (diag = "sensors")}>{tx.diag.sensors}</button>
          <button class="rot" class:checked={diag === "console"} role="tab" aria-selected={diag === "console"} onclick={() => (diag = "console")}>{tx.diag.console}</button>
        </div>
        {#if diag === "sensors"}
        <h3 class="flat">{tx.diag.allReadings} <InfoTip text={tx.diag.sensorsTip} /></h3>
        {#if isMac && powerHelper}
          <section>
            <h3>{tx.diag.power}</h3>
            {#if powerHelper.state === "unavailable"}
              <p class="hint">{tx.diag.powerUnavailable}</p>
            {:else}
              <label class="switch">
                <input
                  type="checkbox"
                  checked={powerHelper.state !== "off"}
                  disabled={powerBusy}
                  onchange={(e) => togglePowerHelper(e.currentTarget.checked)}
                />
                <span class="track"><span class="knob"></span></span>
                <span class="slabel">{tx.diag.powerSwitch} <InfoTip text={tx.diag.powerTip} /></span>
              </label>
              {#if powerHelper.state === "needs_approval"}
                <p class="hint">{tx.diag.approve}</p>
                <div class="actions">
                  <button class="btn" onclick={() => monitor.openPowerHelperSettings()}>{tx.diag.openSettings}</button>
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
          <p class="empty">{tx.diag.readingSensors}</p>
        {/each}
        <section>
          <h3>{tx.diag.stream}</h3>
          <dl class="facts">
            <div><dt>{tx.diag.rate}</dt><dd>1 Hz</dd></div>
            <div><dt>{tx.diag.baud}</dt><dd>115200</dd></div>
            <div><dt>{tx.diag.format}</dt><dd>{tx.diag.jsonLine}</dd></div>
            <div><dt>{tx.diag.staleAfter}</dt><dd>3 s</dd></div>
          </dl>
        </section>
        {:else}
        <div class="console" bind:this={consoleEl} onscroll={scrolled}>
          {#each monitor.logs as line, i (i)}
            <div class="log" class:warn={line.startsWith("W ")} class:err={line.startsWith("E ")}>{line}</div>
          {:else}
            <p class="empty">{tx.diag.consoleEmpty}</p>
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

  .dhead {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 14px 14px 0 18px;
  }
  .dhead h2 {
    flex: 1;
    margin: 0;
    font: 650 15px/1 var(--sans);
    letter-spacing: -0.01em;
  }
  .lang {
    position: relative;
    display: flex;
    align-items: center;
  }
  .lang svg {
    position: absolute;
    left: 9px;
    width: 14px;
    height: 14px;
    fill: none;
    stroke: var(--dim);
    stroke-width: 1.2;
    pointer-events: none;
  }
  .lang select {
    height: 28px;
    max-width: 170px;
    padding: 0 10px 0 29px;
    appearance: none;
    -webkit-appearance: none;
    border-radius: 9px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.045);
    color: var(--text);
    font: 500 12px/1 var(--sans);
    cursor: pointer;
    text-overflow: ellipsis;
  }
  .lang select:hover {
    background: rgba(255, 255, 255, 0.08);
  }
  .dclose {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    border-radius: 8px;
    background: none;
    color: var(--dim);
    cursor: pointer;
  }
  .dclose:hover {
    background: rgba(255, 255, 255, 0.07);
    color: var(--text);
  }
  .dclose svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
  }
  /* What a setting still needs before it works. */
  .hint.need {
    padding: 8px 10px;
    border-radius: 9px;
    border: 1px solid color-mix(in srgb, var(--warm) 35%, transparent);
    background: color-mix(in srgb, var(--warm) 8%, transparent);
    color: color-mix(in srgb, var(--warm) 55%, var(--text));
  }
  h3.flat {
    margin: 0;
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

  .wstatus {
    display: grid;
    grid-template-columns: 10px 1fr auto;
    align-items: center;
    gap: 10px;
    padding: 11px 12px;
    margin-bottom: 10px;
    border-radius: 12px;
    border: 1px solid var(--line);
    background: rgba(255, 255, 255, 0.02);
  }
  .wdot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--faint);
  }
  .wdot.connected {
    background: #5ee38a;
    box-shadow: 0 0 6px #5ee38a;
  }
  .wdot.connecting,
  .wdot.disconnected {
    background: var(--warm);
    animation: blink 1s ease-in-out infinite;
  }
  .wdot.no_network,
  .wdot.wrong_password {
    background: var(--hot);
  }
  .wifi .switch {
    margin-bottom: 10px;
  }
  .wifi .tform {
    margin-top: 10px;
  }
  .ports.nets {
    margin-top: 10px;
    max-height: 220px;
    overflow-y: auto;
  }
  .hint.conn {
    margin-top: 8px;
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
  .petset {
    --accent: #40e080;
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
  .pslot {
    position: relative;
    display: grid;
  }
  .vicon {
    position: absolute;
    top: 6px;
    right: 6px;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--faint);
    cursor: pointer;
  }
  .vicon svg {
    width: 13px;
    height: 13px;
  }
  .vicon:hover,
  .vicon.on {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .vicon:focus-visible {
    outline: 1.5px solid var(--accent);
  }
  .vinline {
    color: var(--accent);
  }
  .vprompt {
    margin-bottom: 10px;
    padding: 10px 12px;
    border-radius: 11px;
    border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
    background: color-mix(in srgb, var(--accent) 6%, transparent);
  }
  .vphead,
  .vpfield {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .vphead b {
    font: 600 12.5px/1.3 var(--sans);
  }
  .vpfield {
    margin-top: 8px;
  }
  .vptext {
    margin: 4px 0 0;
    font: 450 11.5px/1.45 var(--sans);
    color: var(--dim);
    user-select: text;
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
    align-items: center;
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
