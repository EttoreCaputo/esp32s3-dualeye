<!-- Find a setting by what it's called or what its ⓘ says (⌘F / Ctrl+F), and go there. -->
<script lang="ts">
  import { i18n } from "../i18n/index.svelte";
  import { settingsNav } from "./nav.svelte";
  import { search, type Entry } from "./search";

  const tx = $derived(i18n.t.settings);
  const isMac = typeof navigator !== "undefined" && /Mac/.test(navigator.userAgent);
  let query = $state("");
  let active = $state(0);
  let focused = $state(false);
  let input = $state<HTMLInputElement>();
  const results = $derived(search(query, tx).slice(0, 8));
  const id = "settings-search-results";

  // ⌘F asks for the cursor here, also before the panel is open.
  $effect(() => {
    if (!settingsNav.wantSearch || !input) return;
    settingsNav.wantSearch = false;
    input.focus();
    input.select();
  });

  function pick(e: Entry) {
    settingsNav.go(e);
    query = "";
    input?.blur();
  }

  function key(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (results.length) active = (active + (e.key === "ArrowDown" ? 1 : results.length - 1)) % results.length;
    } else if (e.key === "Enter" && results[active]) {
      pick(results[active]);
    } else if (e.key === "Escape" && (query || focused)) {
      // Clears the search, not the panel.
      e.stopPropagation();
      query ? (query = "") : input?.blur();
    }
  }
</script>

<div class="search">
  <svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="7" cy="7" r="4.5" /><path d="M10.4 10.4 14 14" /></svg>
  <input
    bind:this={input}
    type="search"
    role="combobox"
    aria-expanded={focused && !!query}
    aria-controls={id}
    aria-activedescendant={results.length ? `${id}-${active}` : undefined}
    aria-label={tx.common.search}
    placeholder={tx.common.search}
    spellcheck="false"
    bind:value={query}
    oninput={() => (active = 0)}
    onfocus={() => (focused = true)}
    onblur={() => setTimeout(() => (focused = false), 150)}
    onkeydown={key}
  />
  {#if !query}<kbd>{isMac ? "⌘F" : "Ctrl+F"}</kbd>{/if}
  {#if focused && query}
    <ul class="results" {id} role="listbox">
      {#each results as r, i (r.tab + r.id)}
        <li role="option" id="{id}-{i}" aria-selected={i === active}>
          <button class:on={i === active} tabindex="-1" onmousedown={(e) => e.preventDefault()} onclick={() => pick(r)} onmouseenter={() => (active = i)}>
            <span>{r.label(tx)}</span>
            <small>{tx.tabs[r.tab]}{r.advanced ? ` · ${tx.common.advanced}` : ""}</small>
          </button>
        </li>
      {:else}
        <li class="none">{tx.common.searchNone}</li>
      {/each}
    </ul>
  {/if}
</div>
