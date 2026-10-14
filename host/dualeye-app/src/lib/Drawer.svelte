<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { tick } from "svelte";
  import { i18n, LANGUAGES, type Lang } from "./i18n/index.svelte";
  import "./settings/settings.css";
  import { onboarding } from "./onboarding/state.svelte";
  import ClaudeTab from "./settings/ClaudeTab.svelte";
  import DeviceTab from "./settings/DeviceTab.svelte";
  import DiagnosticsTab from "./settings/DiagnosticsTab.svelte";
  import { TAB_ICONS } from "./settings/icons";
  import { settingsNav, TABS } from "./settings/nav.svelte";
  import PetTab from "./settings/PetTab.svelte";
  import { issues, level } from "./settings/requirements";
  import Search from "./settings/Search.svelte";
  import TimersTab from "./settings/TimersTab.svelte";
  import VoiceTab from "./settings/VoiceTab.svelte";

  const tx = $derived(i18n.t.settings);
  const tc = $derived(tx.common);
  const nav = settingsNav;
  const PANES = { voice: VoiceTab, pet: PetTab, timers: TimersTab, claude: ClaudeTab, device: DeviceTab, diagnostics: DiagnosticsTab };
  const Pane = $derived(PANES[nav.tab]);

  /** Each tab's issues, for its dot and its tooltip. */
  const tabIssues = $derived(Object.fromEntries(TABS.map((t) => [t, issues(t)])));

  function onKey(e: KeyboardEvent) {
    // The setup and the tour are on top: the keys are theirs.
    if (onboarding.wizardOpen || onboarding.tour) return;
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      nav.search();
    } else if (nav.open && e.key === "Escape") nav.open = false;
  }

  // Another tab starts at its top.
  $effect(() => {
    void nav.tab;
    if (content) content.scrollTop = 0;
  });

  // A setting asked for (a search result, a "Set up" button): scroll to it, light it up, focus it.
  let content = $state<HTMLElement>();
  $effect(() => {
    const target = nav.target;
    if (!target || !nav.open) return;
    let frame = 0;
    let tries = 0;
    const find = () => {
      const el = content?.querySelector<HTMLElement>(`[data-anchor="${target.id}"]`);
      if (!el) {
        if (tries++ < 90) frame = requestAnimationFrame(find);
        return;
      }
      nav.target = null;
      el.scrollIntoView({ block: "center", behavior: "smooth" });
      el.classList.remove("flash");
      void el.offsetWidth;
      el.classList.add("flash");
      setTimeout(() => el.classList.remove("flash"), 2000);
      const control = el.matches("input, select, textarea, button") ? el : el.querySelector<HTMLElement>(":is(input, select, textarea, button):not(.info button):not(:disabled)");
      control?.focus({ preventScroll: true });
    };
    tick().then(() => (frame = requestAnimationFrame(find)));
    return () => cancelAnimationFrame(frame);
  });

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

  /** The app's language: a language, or `system` to follow the computer's. */
  const langChoice = $derived(i18n.choice ?? "system");
  const systemName = $derived(LANGUAGES.find((l) => l.id === i18n.system)?.name ?? "English");
  const pickLang = (value: string) => i18n.set(value === "system" ? null : (value as Lang));
</script>

<svelte:window onkeydown={onKey} />

{#if nav.open}
  <button class="settings-scrim" transition:fade={{ duration: 200 }} onclick={() => (nav.open = false)} aria-label={tc.close}></button>
  <aside class="drawer settings-ui" class:resizing style:--w="{width}px" transition:fly={{ x: 40, duration: 320, easing: cubicOut, opacity: 0 }} aria-label={tc.title}>
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
      <button class="dclose" onclick={() => (nav.open = false)} aria-label={tc.close} title={tc.close}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" /></svg>
      </button>
    </header>
    <div class="dsearch"><Search /></div>
    <nav aria-label={tc.title}>
      {#each TABS as key (key)}
        {@const list = tabIssues[key]}
        {@const dot = level(list)}
        <button
          class:active={nav.tab === key}
          aria-current={nav.tab === key ? "page" : undefined}
          aria-label={dot ? `${tx.tabs[key]}: ${list.map((i) => i.text).join(", ")}` : tx.tabs[key]}
          title={[tx.tabs[key], ...list.map((i) => `• ${i.text}`)].join("\n")}
          onclick={() => (nav.tab = key)}
        >
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d={TAB_ICONS[key]} /></svg>
          <span class="tlabel">{tx.tabs[key]}</span>
          {#if dot}<span class="ndot {dot}"></span>{/if}
        </button>
      {/each}
      <span class="indicator" style:--i={TABS.indexOf(nav.tab)}></span>
    </nav>
    <div class="content" data-tab={nav.tab} bind:this={content}>
      {#key nav.tab}
        <Pane />
      {/key}
    </div>
  </aside>
{/if}
