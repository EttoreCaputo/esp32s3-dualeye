// Where the settings panel is: open or not, which tab, which "Advanced" parts
// are open, and a setting to bring into view (a search result, a "Set up"
// button). Every setting a link can point at has `data-anchor="<id>"`.

export type Tab = "voice" | "pet" | "timers" | "claude" | "device" | "diagnostics";
export const TABS: Tab[] = ["voice", "pet", "timers", "claude", "device", "diagnostics"];

/** A setting: its tab, its `data-anchor`, and whether it sits under "Advanced". */
export type Anchor = { tab: Tab; id: string; advanced?: boolean };

const ADVANCED_KEY = "dualeye.advanced";

function loadAdvanced(): Partial<Record<Tab, boolean>> {
  try {
    return JSON.parse(localStorage.getItem(ADVANCED_KEY) ?? "{}") ?? {};
  } catch {
    return {};
  }
}

class SettingsNav {
  open = $state(false);
  tab = $state<Tab>("voice");
  /** "Advanced", by tab: kept across launches. */
  advanced = $state<Partial<Record<Tab, boolean>>>(loadAdvanced());
  /** The setting to scroll to and light up; `seq` makes the same one count twice. */
  target = $state<{ id: string; seq: number } | null>(null);
  /** The search field should take the cursor (it clears this once it has). */
  wantSearch = $state(false);
  #seq = 0;

  show(tab?: Tab) {
    if (tab) this.tab = tab;
    this.open = true;
  }

  go(anchor: Anchor) {
    this.tab = anchor.tab;
    if (anchor.advanced) this.setAdvanced(anchor.tab, true);
    this.open = true;
    this.target = { id: anchor.id, seq: ++this.#seq };
  }

  setAdvanced(tab: Tab, open: boolean) {
    if (!!this.advanced[tab] === open) return;
    this.advanced[tab] = open;
    try {
      localStorage.setItem(ADVANCED_KEY, JSON.stringify(this.advanced));
    } catch {}
  }

  search() {
    this.open = true;
    this.wantSearch = true;
  }
}

export const settingsNav = new SettingsNav();
