// Whether the board's firmware is older than the one bundled with the app, and
// what flashing the bundled one changes, from FIRMWARE_CHANGELOG.md.

import changelogText from "../../../../FIRMWARE_CHANGELOG.md?raw";
import type { BoardFirmware } from "./monitor.svelte";

export type Release = { version: string; notes: string[] };
export type Update = {
  /** `null` for firmware from before versioning, or none at all. */
  from: string | null;
  to: string;
  /** Newest first: every release after `from`, up to `to`. */
  changes: Release[];
};

/** A changelog line in pieces, `**bold**` ones marked. */
export function boldParts(line: string): { text: string; bold: boolean }[] {
  return line.split(/\*\*(.+?)\*\*/).map((text, i) => ({ text, bold: i % 2 === 1 })).filter((p) => p.text);
}

/** `## 0.2.0` headings, each followed by `- ` bullets. */
export function parseChangelog(text: string): Release[] {
  const releases: Release[] = [];
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    const heading = /^##\s+v?(\d+(?:\.\d+)*)/.exec(line);
    if (heading) releases.push({ version: heading[1], notes: [] });
    else if (releases.length && /^[-*]\s+/.test(line)) releases[releases.length - 1].notes.push(line.replace(/^[-*]\s+/, ""));
    else if (releases.length && line && releases[releases.length - 1].notes.length) {
      // A bullet wrapped over several lines.
      const notes = releases[releases.length - 1].notes;
      notes[notes.length - 1] += ` ${line}`;
    }
  }
  return releases;
}

export const CHANGELOG = parseChangelog(changelogText);
/** Firmware that can't report its version predates 0.2.0, the first that could. */
const LEGACY = "0.1.0";

/** Compare dotted versions numerically; a suffix like `-dirty` is ignored. */
export function compareVersions(a: string, b: string): number {
  const parts = (v: string) => (/^v?(\d+(?:\.\d+)*)/.exec(v.trim())?.[1] ?? "0").split(".").map(Number);
  const [x, y] = [parts(a), parts(b)];
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d) return Math.sign(d);
  }
  return 0;
}

/** The update to offer, if the board runs DualEye firmware older than `bundled`. */
export function updateFor(board: BoardFirmware | null, bundled: string | undefined, changelog = CHANGELOG): Update | null {
  if (!board || !bundled || board.state === "missing") return null;
  const from = board.state === "version" ? board.version : null;
  if (from !== null && compareVersions(from, bundled) >= 0) return null;
  const changes = changelog.filter((r) => compareVersions(r.version, from ?? LEGACY) > 0 && compareVersions(r.version, bundled) <= 0);
  return { from, to: bundled, changes };
}
