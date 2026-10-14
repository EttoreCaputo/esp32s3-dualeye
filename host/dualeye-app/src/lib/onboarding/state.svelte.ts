// The first-run setup and the tour: whether they're open, and where they
// stand. The app's settings keep both (`onboarding_done`, `tour_version`), so
// someone upgrading with a board already set up skips the setup and only gets
// the tour's invitation; a newer tour offers just its new steps.

import { invoke, isTauri } from "@tauri-apps/api/core";
import { TOUR, TOUR_VERSION, type TourStep } from "./tour";

/** Outside the app (the browser preview) it lives in localStorage. */
const PREVIEW_KEY = "dualeye.onboarding";

type Saved = { done: boolean; tour_version: number };

function previewLoad(): Saved {
  try {
    return { done: false, tour_version: 0, ...JSON.parse(localStorage.getItem(PREVIEW_KEY) ?? "{}") };
  } catch {
    return { done: false, tour_version: 0 };
  }
}

class Onboarding {
  /** Not known until read: neither the setup nor the invitation shows before. */
  loaded = $state(false);
  done = $state(true);
  tourVersion = $state(TOUR_VERSION);
  wizardOpen = $state(false);
  /** The tour's steps while it runs. */
  tour = $state<TourStep[] | null>(null);
  /** The invitation was put off until the next start. */
  later = $state(false);

  /** A newer tour than the one seen, for someone who's past the setup. */
  readonly invite = $derived(this.loaded && this.done && !this.wizardOpen && !this.tour && !this.later && this.tourVersion < TOUR_VERSION);

  async start() {
    const saved: Saved = isTauri() ? await invoke<Saved>("onboarding").catch(() => ({ done: true, tour_version: TOUR_VERSION })) : previewLoad();
    this.done = saved.done;
    this.tourVersion = saved.tour_version;
    this.loaded = true;
    if (!this.done) this.wizardOpen = true;
  }

  async #save(change: Partial<Saved>) {
    if (change.done !== undefined) this.done = change.done;
    if (change.tour_version !== undefined) this.tourVersion = change.tour_version;
    if (!isTauri()) {
      try {
        localStorage.setItem(PREVIEW_KEY, JSON.stringify({ done: this.done, tour_version: this.tourVersion }));
      } catch {}
      return;
    }
    await invoke("set_onboarding", { done: change.done ?? null, tourVersion: change.tour_version ?? null }).catch(() => {});
  }

  /** The setup again, from the settings. */
  redo() {
    this.tour = null;
    this.wizardOpen = true;
  }

  /** Done or skipped: either way it doesn't come back by itself. */
  finishWizard() {
    this.wizardOpen = false;
    void this.#save({ done: true });
  }

  /** The tour: every step, or only those newer than the version seen. */
  startTour(onlyNew = false) {
    const from = onlyNew ? this.tourVersion : 0;
    const steps = TOUR.filter((s) => s.since > from);
    this.wizardOpen = false;
    this.tour = steps.length ? steps : TOUR;
  }

  /** Seen or declined: this version doesn't ask again. */
  endTour() {
    this.tour = null;
    void this.#save({ tour_version: TOUR_VERSION });
  }

  declineTour() {
    void this.#save({ tour_version: TOUR_VERSION });
  }
}

export const onboarding = new Onboarding();
