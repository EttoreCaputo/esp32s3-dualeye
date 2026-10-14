// The app's language. `en/` holds every text the app shows; each other
// language mirrors it key for key, and `npm run check` fails on a missing one.
// The voice's and the Claude alerts' language is a separate setting.

import { invoke, isTauri } from "@tauri-apps/api/core";
import en, { type Messages } from "./en";
import it from "./it";

/** Each language named in itself, for the picker. */
export const LANGUAGES = [
  { id: "en", name: "English" },
  { id: "it", name: "Italiano" },
] as const;
export type Lang = (typeof LANGUAGES)[number]["id"];
export type { Messages };

const DICTS: Record<Lang, Messages> = { en, it };
/** The last choice, so the first frame is already in it; the app's settings keep the real one. */
const CACHE_KEY = "dualeye.lang";

const isLang = (id: unknown): id is Lang => typeof id === "string" && id in DICTS;

/** The first of the system's languages the app speaks, English otherwise. */
function systemLang(): Lang {
  for (const tag of navigator.languages?.length ? navigator.languages : [navigator.language]) {
    const base = tag?.toLowerCase().split("-")[0];
    if (isLang(base)) return base;
  }
  return "en";
}

function cached(): Lang | null {
  try {
    const id = localStorage.getItem(CACHE_KEY);
    return isLang(id) ? id : null;
  } catch {
    return null;
  }
}

class I18n {
  /** `null` follows the system. */
  choice = $state<Lang | null>(cached());
  readonly system = systemLang();
  readonly lang = $derived<Lang>(this.choice ?? this.system);
  readonly t = $derived(DICTS[this.lang]);

  /** Read the saved choice and put the tray menu in the language. */
  async start() {
    if (!isTauri()) return;
    const saved = await invoke<string | null>("ui_language").catch(() => null);
    this.choice = isLang(saved) ? saved : null;
    this.#remember();
    await this.#apply();
  }

  async set(choice: Lang | null) {
    this.choice = choice;
    this.#remember();
    await this.#apply();
  }

  #remember() {
    try {
      if (this.choice) localStorage.setItem(CACHE_KEY, this.choice);
      else localStorage.removeItem(CACHE_KEY);
    } catch {}
  }

  async #apply() {
    document.documentElement.lang = this.lang;
    if (!isTauri()) return;
    const tray = { show: this.t.main.tray.show, quit: this.t.main.tray.quit };
    await invoke("set_ui_language", { language: this.choice, tray }).catch(() => {});
  }

  // Numbers, dates and times in the app's language.
  number(n: number, opts?: Intl.NumberFormatOptions) {
    return n.toLocaleString(this.lang, opts);
  }
  time(at: number | Date, seconds = false) {
    return new Date(at).toLocaleTimeString(this.lang, { hour: "2-digit", minute: "2-digit", ...(seconds ? { second: "2-digit" } : {}) });
  }
  date(at: number | Date, opts: Intl.DateTimeFormatOptions = { day: "numeric", month: "short" }) {
    return new Date(at).toLocaleDateString(this.lang, opts);
  }
  /** `one` or `other`, by the language's plural rule for `n`. */
  plural(n: number, one: string, other: string) {
    return new Intl.PluralRules(this.lang).select(n) === "one" ? one : other;
  }
}

export const i18n = new I18n();
document.documentElement.lang = i18n.lang;
