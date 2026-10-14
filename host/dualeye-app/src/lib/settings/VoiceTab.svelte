<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import InfoTip from "../InfoTip.svelte";
  import { monitor, type AccountVoice, type Hardware, type ModelInfo, type Personality, type SttLanguage, type TtsEngine, type VoiceInfo } from "../monitor.svelte";
  import { VOICE_PROMPTS } from "../voicePrompts";
  import Advanced from "./Advanced.svelte";
  import { settingsData } from "./data.svelte";
  import KeyField from "./KeyField.svelte";
  import { settingsNav } from "./nav.svelte";
  import NeedRow from "./NeedRow.svelte";
  import { need } from "./requirements";
  import Section from "./Section.svelte";
  import Toggle from "./Toggle.svelte";
  import VoiceStatus from "./VoiceStatus.svelte";

  const tx = $derived(i18n.t.settings);
  const tv = $derived(tx.voice);
  const tc = $derived(tx.common);
  const voice = $derived(settingsData.voice);
  const connected = $derived(monitor.link === "connected");

  // Faster while the tab is open, for a download's progress bar.
  $effect(() => {
    const load = () => settingsData.loadVoice().catch(() => {});
    load();
    const id = setInterval(load, 700);
    return () => clearInterval(id);
  });

  let voiceError = $state("");
  let engineError = $state("");
  let customLength = $state<number | null>(null);

  async function setVoice(change: Parameters<typeof settingsData.setVoice>[0]) {
    voiceError = "";
    try {
      await settingsData.setVoice(change);
    } catch (e) {
      voiceError = String(e);
    }
  }

  // The speaker's volume and the eyes live on the board: read them when the tab opens.
  let volume = $state<number | null>(null);
  let eyes = $state<boolean | null>(null);
  let wakeSound = $state<boolean | null>(null);
  $effect(() => {
    if (!connected) return;
    let alive = true;
    monitor
      .boardVoice()
      .then((b) => alive && ((volume = b.volume), (eyes = b.eyes), (wakeSound = b.wake_sound)))
      .catch(() => alive && (volume = eyes = wakeSound = null));
    return () => {
      alive = false;
    };
  });

  /** Set something on the board, and put it back if the board refuses. */
  async function onBoard<T>(set: (v: T) => void, value: T, previous: T, send: () => Promise<unknown>) {
    set(value);
    try {
      await send();
    } catch (e) {
      set(previous);
      voiceError = String(e);
    }
  }

  // ── A voice's test, next to it in the list.
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

  // ── ElevenLabs: the account's voices, once read.
  let accountVoices = $state<AccountVoice[] | null>(null);
  let elevenError = $state("");
  const hasElevenKey = $derived(!!voice?.providers.find((p) => p.id === "elevenlabs")?.key);
  $effect(() => {
    if (hasElevenKey) loadAccountVoices();
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
      note: a.category === "premade" ? tv.elevenOwn : a.description || tv.elevenAccount,
      license: "",
      installed: true,
      provider: "elevenlabs",
    })),
  );
  /** An ElevenLabs voice speaks both languages: it answers in either. */
  const useAccountVoice = (v: VoiceInfo, id: string) => setVoice({ voices: { ...v.settings.voices, it: id, en: id } });

  // ── The personalities' prompts for ElevenLabs: shown while hovered, kept by a click.
  const PERSONALITIES: Personality[] = ["cute", "playful", "calm", "sassy", "butler", "minimal", "custom"];
  /** `agent::MAX_PERSONALITY`. */
  const MAX_PERSONALITY = 300;
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

  // ── Models.
  const ENGINE_NAMES: Record<TtsEngine, string> = { piper: "Piper" };
  const ENGINES: [TtsEngine, string, () => string][] = [["piper", "Piper", () => tv.piperNote]];
  const STT_LANGUAGES: [SttLanguage, string][] = [
    ["auto", "Auto"],
    ["it", "Italiano"],
    ["en", "English"],
  ];
  const VOICE_LANGUAGES: [string, string][] = [
    ["it", "Italiano"],
    ["en", "English"],
  ];
  /** A note from the backend, in the app's language when there's a translation. */
  const note = (text: string) => tv.notes[text] ?? text;
  const providerName = (v: VoiceInfo, id: string | null) => v.providers.find((p) => p.id === id)?.name ?? id ?? "";
  const mb = (n: number) => (n >= 1e9 ? `${i18n.number(n / 1e9, { maximumFractionDigits: 1 })} GB` : `${Math.round(n / 1_000_000)} MB`);
  const gb = (mbytes: number) => `${Math.round(mbytes / 1024)} GB`;
  const hardwareLine = (hw: Hardware) =>
    tv.hardware(hw.cpu, hw.cores, gb(hw.memory_mb), hw.gpu.kind === "apple" ? tv.gpuApple : hw.gpu.kind === "nvidia" ? `${hw.gpu.name} (${gb(hw.gpu.memory_mb)})` : tv.gpuNone);

  /** A model as its list shows it: size and whether it's here, or whether its service has a key. */
  function optionLabel(v: VoiceInfo, m: ModelInfo): string {
    if (m.provider) {
      const name = accountVoices?.find((a) => m.id === accountId(a))?.name ?? m.id.slice(m.id.indexOf(":") + 1);
      return `${name}${m.installed ? "" : tv.needsKey(providerName(v, m.provider))}`;
    }
    return `${m.id} · ${mb(m.bytes)}${m.installed ? tv.downloaded : ""}${m.id === v.recommendation.whisper || m.id === v.recommendation.llm ? tv.recommended : ""}`;
  }

  function usesRecommended(v: VoiceInfo): boolean {
    const r = v.recommendation;
    return v.settings.model === r.whisper && (r.llm ? v.settings.llm && v.settings.llm_model === r.llm : !v.settings.llm);
  }

  /** Pick the models recommended for this computer, and download those missing. */
  async function useRecommended(v: VoiceInfo) {
    const r = v.recommendation;
    await setVoice(r.llm ? { model: r.whisper, llm: true, llm_model: r.llm } : { model: r.whisper, llm: false });
    for (const id of [r.whisper, r.llm]) if (id && !v.models.find((m) => m.id === id)?.installed) await download(id);
  }

  async function download(id: string) {
    voiceError = "";
    await settingsData.download(id).catch((e) => (voiceError = String(e)));
  }

  async function installEngine(engine: TtsEngine) {
    engineError = "";
    await settingsData.installEngine(engine).catch((e) => (engineError = String(e)));
  }

  async function removeModel(id: string) {
    voiceError = "";
    try {
      settingsData.voice = await monitor.deleteModel(id);
    } catch (e) {
      voiceError = String(e);
    }
  }

  // ── Transcripts.
  const newestFirst = $derived([...monitor.transcripts].reverse());
  const clock = (ms: number) => i18n.time(ms, true);
  const seconds = (ms: number) => `${i18n.number(ms / 1000, { minimumFractionDigits: 1, maximumFractionDigits: 1 })} s`;
</script>

<!-- One model to pick, from this computer's or an online service's, and what it still needs. -->
{#snippet picker(v: VoiceInfo, anchor: string, label: string, what: string, kind: ModelInfo["kind"], lang: string | null, chosen: string, onpick: (id: string) => void, off: boolean)}
  {@const list = [...v.models, ...accountModels].filter((m) => m.kind === kind && (lang === null || m.language === lang || m.language === null))}
  {@const m = list.find((x) => x.id === chosen)}
  {@const downloading = !!m && v.download?.[0] === m.id}
  {@const installing = m?.engine ? v[`${m.engine}_install`] : null}
  <div class="mrow" class:off data-anchor={anchor}>
    <div class="mhead">
      <span class="mtext">
        <span class="pname">{label}</span>
        <span class="mnote">{what}</span>
      </span>
      {#if kind === "voice" && lang}
        <button class="btn small" disabled={v.tts !== "ready" || testing !== null || !connected || !m?.installed} title={tv.testTitle} onclick={() => testVoice(lang)}
          >{testing === lang ? tv.testing : tv.test}</button
        >
      {/if}
    </div>
    <div class="select">
      <select value={chosen} disabled={off} aria-label={label} onchange={(e) => onpick(e.currentTarget.value)}>
        {#if !m}<option value={chosen} disabled>{tv.choose}</option>{/if}
        <optgroup label={tv.onComputer}>
          {#each list.filter((o) => !o.provider) as o (o.id)}
            <option value={o.id}>{optionLabel(v, o)}</option>
          {/each}
        </optgroup>
        {#each v.providers as p (p.id)}
          {@const cloud = list.filter((o) => o.provider === p.id)}
          {#if cloud.length}
            <optgroup label={tv.onlineGroup(p.name)}>
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
        <span class="mnote">{note(m.note)}</span>
        <span class="mside">
          {#if m.provider}
            {#if m.installed}
              <span class="chip online" title={tv.onlineTitle}>{tc.online}</span>
            {:else}
              {@const provider = m.provider}
              <button class="btn small primary" onclick={() => settingsNav.go({ tab: "voice", id: `key-${provider}` })}>{tv.addKey(providerName(v, provider))}</button>
            {/if}
          {:else if downloading}
            <button class="btn small" onclick={() => monitor.cancelDownload()}>{tv.stop(Math.round(v.download?.[1] ?? 0))}</button>
          {:else if !m.installed}
            <button class="btn small primary" disabled={!!v.download} onclick={() => download(m.id)}>{tv.download(mb(m.bytes))}</button>
          {:else if m.engine && !v[m.engine]}
            {@const engine = m.engine}
            <button class="btn small primary" disabled={!!installing} onclick={() => installEngine(engine)}>{installing ? tc.installing : tv.installEngine(ENGINE_NAMES[engine])}</button>
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

{#if !voice}
  <p class="empty">{tc.loading}</p>
{:else}
  {@const v = voice}
  {@const s = v.settings}
  <section class="voice">
    <Toggle master anchor="assistant" checked={s.enabled} label={tv.assistant} tip={tv.assistantTip} onchange={(on) => setVoice({ enabled: on })} />
    {#if s.enabled}<VoiceStatus />{/if}
    <div class="volume" data-anchor="volume">
      <span class="rlabel">{tv.volume}</span>
      <input type="range" min="0" max="100" step="5" value={volume ?? 60} disabled={volume === null} aria-label={tv.volumeAria} onchange={(e) => onBoard((x) => (volume = x), Number(e.currentTarget.value), volume, () => monitor.setBoardVolume(Number(e.currentTarget.value)))} />
      <span class="pmeta">{volume === null ? "—" : `${volume}%`}</span>
    </div>
    <div class="rotation" data-anchor="language">
      <span class="rlabel">{tv.language}&nbsp;<InfoTip text={tv.languageTip} /></span>
      <div class="rots langs" role="radiogroup" aria-label={tv.language}>
        {#each STT_LANGUAGES as [id, label] (id)}
          <button class="rot" class:checked={s.language === id} role="radio" aria-checked={s.language === id} title={id === "auto" ? tv.autoTitle : tv.always(label)} onclick={() => setVoice({ language: id })}
            >{id === "auto" ? tv.auto : label}</button
          >
        {/each}
      </div>
    </div>
  </section>

  <Section class="voice" title={tv.behaviour}>
    <Toggle anchor="speak" checked={s.speak} label={tv.speak} tip={tv.speakTip} onchange={(on) => setVoice({ speak: on })} />
    <Toggle anchor="llm" checked={s.llm} label={tv.llm} tip={tv.llmTip} onchange={(on) => setVoice({ llm: on })} />
  </Section>

  <Section class="voice" anchor="personality" title={tv.personality} tip={tv.personalityTip} dimmed={!s.llm}>
    <NeedRow need={need("llm")} />
    <div class="persona" role="radiogroup" aria-label={tv.personality}>
      {#each PERSONALITIES as id (id)}
        {@const [name, blurb] = tv.personalities[id]}
        <div class="pslot">
          <button class="pcard" class:checked={s.personality === id} role="radio" aria-checked={s.personality === id} onclick={() => setVoice({ personality: id })}>
            <b>{name}</b>
            <small>{blurb}</small>
          </button>
          {#if VOICE_PROMPTS[id]}
            <button
              class="vicon"
              class:on={shownPrompt === id}
              aria-label={tv.promptAria(name)}
              aria-expanded={pinnedPrompt === id}
              title={tv.promptTitle}
              onmouseenter={() => (hoverPrompt = id)}
              onmouseleave={() => (hoverPrompt = null)}
              onfocus={() => (hoverPrompt = id)}
              onblur={() => (hoverPrompt = null)}
              onclick={() => (pinnedPrompt = pinnedPrompt === id ? null : id)}
            >
              <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2 6.5v3M5 4v8M8 2.5v11M11 4.5v7M14 6.5v3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" /></svg>
            </button>
          {/if}
        </div>
      {/each}
    </div>
    {#if shownPrompt && VOICE_PROMPTS[shownPrompt]}
      {@const p = VOICE_PROMPTS[shownPrompt]}
      <div class="vprompt">
        <div class="vphead">
          <b>{tv.promptHead(tv.personalities[shownPrompt][0])}</b>
          {#if pinnedPrompt}<button class="btn small" onclick={() => (pinnedPrompt = null)}>{tc.closeBtn}</button>{/if}
        </div>
        <p class="hint">{tv.promptSteps(p.name)}</p>
        <div class="vpfield">
          <span class="rlabel">{tv.prompt}</span>
          <button class="btn small" onclick={() => copyPrompt(p.description, "description")}>{promptCopied === "description" ? tc.copied : tc.copy}</button>
        </div>
        <p class="vptext">{p.description}</p>
        <div class="vpfield">
          <span class="rlabel">{tv.previewText}</span>
          <button class="btn small" onclick={() => copyPrompt(p.sample, "sample")}>{promptCopied === "sample" ? tc.copied : tc.copy}</button>
        </div>
        <p class="vptext">{p.sample}</p>
      </div>
    {/if}
    {#if s.personality === "custom"}
      <div class="custom">
        <textarea
          rows="3"
          maxlength={MAX_PERSONALITY}
          placeholder={tv.customPlaceholder}
          value={s.personality_custom}
          oninput={(e) => (customLength = e.currentTarget.value.length)}
          onchange={(e) => setVoice({ personality_custom: e.currentTarget.value })}
        ></textarea>
        <span class="pmeta">{customLength ?? s.personality_custom.length}/{MAX_PERSONALITY}</span>
      </div>
    {/if}
  </Section>

  <Section class="voice" anchor="services" title={tv.services} tip={tv.servicesTip}>
    {#each v.providers as p (p.id)}
      <KeyField provider={p} />
    {/each}
  </Section>

  <Section class="voice" anchor="eleven" title={tv.eleven} tip={tv.elevenTip} dimmed={!s.speak}>
    {#if !hasElevenKey}
      <NeedRow need={need("key:elevenlabs")} />
    {:else}
      {#if accountVoices === null}
        <p class="hint small">{tv.elevenReading}</p>
      {:else if !ownVoices.length}
        <p class="hint">{tv.elevenNone}</p>
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
                  <span class="pmeta">{tv.speakingNow}</span>
                {:else}
                  <button class="btn small" onclick={() => useAccountVoice(v, accountId(a))}>{tv.use}</button>
                {/if}
              </span>
            </div>
          {/each}
        </div>
      {/if}
      <div class="vpfield">
        <p class="hint">{tv.bothLanguages}</p>
        <button class="btn small" onclick={loadAccountVoices}>{tc.refresh}</button>
      </div>
    {/if}
    {#if elevenError}<p class="hint error">{elevenError}</p>{/if}
  </Section>

  <Section class="voice" anchor="models" title={tv.models}>
    {#if !usesRecommended(v)}
      <div class="recbox">
        <p class="hint">{v.recommendation.why}</p>
        <button class="btn small" onclick={() => useRecommended(v)}>{tv.useRecommended(v.recommendation.whisper, v.recommendation.llm)}</button>
      </div>
    {/if}
    {@render picker(v, "model-hearing", tv.hearing, tv.hearingWhat, "whisper", null, s.model, (id) => setVoice({ model: id }), false)}
    {@render picker(v, "model-llm", tv.understanding, tv.understandingWhat, "llm", null, s.llm_model, (id) => setVoice({ llm_model: id }), !s.llm)}
    <div data-anchor="voices">
      {#each VOICE_LANGUAGES as [lang, label] (lang)}
        {@render picker(v, `voice-${lang}`, tv.voiceFor(label), tv.voiceWhat, "voice", lang, s.voices[lang] ?? "", (id) => setVoice({ voices: { ...s.voices, [lang]: id } }), !s.speak)}
      {/each}
    </div>
    {#if voiceError}<p class="hint error">{voiceError}</p>{/if}
    {#if engineError}<p class="hint error">{engineError}</p>{/if}
  </Section>

  <Advanced tab="voice">
    <Toggle anchor="follow-up" checked={s.follow_up} disabled={!s.speak} label={tv.followUp} tip={tv.followUpTip} onchange={(on) => setVoice({ follow_up: on })} />
    <Toggle anchor="pause-music" checked={s.pause_music} label={tv.pauseMusic} onchange={(on) => setVoice({ pause_music: on })} />
    <Toggle anchor="keep-warm" checked={s.keep_warm} label={tv.keepWarm} tip={tv.keepWarmTip} onchange={(on) => setVoice({ keep_warm: on })} />
    <Toggle
      anchor="wake-sound"
      checked={wakeSound ?? true}
      disabled={wakeSound === null}
      title={wakeSound === null ? tc.needsFirmware("1.3.3") : ""}
      label={tv.wakeSound}
      onchange={(on) => onBoard((x) => (wakeSound = x), on, !on, () => monitor.setBoardWakeSound(on))}
    />
    <Toggle
      anchor="eyes"
      checked={eyes ?? true}
      disabled={eyes === null}
      title={eyes === null ? tc.needsFirmware("1.0.1") : ""}
      label={tv.eyes}
      tip={tv.eyesTip}
      onchange={(on) => onBoard((x) => (eyes = x), on, !on, () => monitor.setBoardEyes(on))}
    />
    <div data-anchor="downloads">
      <p class="hint">
        {hardwareLine(v.hardware)}. whisper-server: <code>{v.server ?? tv.notFound}</code> · llama-server: <code>{v.llm_server ?? tv.notFound}</code>
      </p>
      {#if !v.server}
        {@const w = tv.whisperMissing}
        <p class="hint error">{w[0]}<code>tools/build_sidecars.sh</code>{w[1]}<code>brew install whisper-cpp</code>{w[2]}</p>
      {/if}
      {#if !v.llm_server}
        {@const w = tv.llamaMissing}
        <p class="hint error">{w[0]}<code>tools/build_sidecars.sh</code>{w[1]}<code>brew install llama.cpp</code>{w[2]}</p>
      {/if}
      <div class="ports">
        {#each ENGINES as [engine, name, engineNote] (engine)}
          {@const installing = v[`${engine}_install`]}
          <div class="port model" class:checked={!!v[engine]}>
            <span class="mtext">
              <span class="pname">{name}</span>
              <span class="mnote">{installing ? tv.installingWith(installing) : engineNote()}</span>
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
          {@const used = [s.model, s.llm_model, ...Object.values(s.voices)].includes(m.id)}
          <div class="port model">
            <span class="mtext">
              <span class="pname">{m.id}</span>
              <span class="mnote">{tv.kinds[m.kind]} · {mb(m.bytes)}{used ? tv.inUse : ""}</span>
            </span>
            <span class="mside">
              <button class="btn small" disabled={used && s.enabled} title={tv.deleteTitle} onclick={() => removeModel(m.id)}>{tc.delete}</button>
            </span>
          </div>
        {/each}
      </div>
    </div>
    <label class="check" data-anchor="recordings">
      <input type="checkbox" checked={s.keep_recordings} onchange={(e) => setVoice({ keep_recordings: e.currentTarget.checked })} />
      <span>{tv.keepRecordings}</span>
    </label>
  </Advanced>

  <Section anchor="transcripts" title={tv.transcripts}>
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
              {#if entry.reply.by === "rules" && s.llm}
                <span class="taction" title={tv.fixedTitle}>{tv.fixedPhrase}</span>
              {/if}
              {#each entry.reply.actions as action, i (i)}
                <span class="taction">{action}</span>
              {/each}
              {#if entry.spoken?.reason === "barge_in"}
                <span class="taction" title={tv.interruptedTitle}>{tv.interrupted}</span>
              {/if}
            </span>
            <span class="tmeta">{entry.spoken ? seconds(entry.spoken.first_audio_ms) : ""}</span>
          {/if}
        {:else}
          <span class="ttext muted">{tv.noWords}</span>
        {/if}
      </div>
    {:else}
      <p class="empty">{s.enabled ? tv.emptyOn : tv.emptyOff}</p>
    {/each}
  </Section>
{/if}
