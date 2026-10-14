// The tabs' icons: 16×16 strokes.
import type { Tab } from "./nav.svelte";

export const TAB_ICONS: Record<Tab, string> = {
  voice: "M8 2.2a2.1 2.1 0 0 1 2.1 2.1v3.6a2.1 2.1 0 0 1-4.2 0V4.3A2.1 2.1 0 0 1 8 2.2zM4.3 7.6a3.7 3.7 0 0 0 7.4 0M8 11.3v2.5M6 13.8h4",
  pet: "M3.2 2.8l2.3 2.6h5l2.3-2.6v6.4a4.8 4.4 0 0 1-9.6 0zM6.3 8.6v.6M9.7 8.6v.6M7.3 10.8h1.4",
  timers: "M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11zM8 5v3.2l2.1 1.4",
  claude: "M8 1.8v12.4M2.6 4.9l10.8 6.2M2.6 11.1l10.8-6.2",
  device: "M4.5 4.5h7v7h-7zM6.5 2v2.5M9.5 2v2.5M6.5 11.5V14M9.5 11.5V14M2 6.5h2.5M2 9.5h2.5M11.5 6.5H14M11.5 9.5H14",
  diagnostics: "M1.5 8.5h3l1.6-4.2 2.6 8 2-6 1.1 2.2h2.7",
};
