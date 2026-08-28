import type { CrdRef } from "./crds";
import { settingsStorage } from "./settingsStorage";

/** A recently-opened palette target, persisted across sessions. */
export type RecentItem =
  | { type: "view"; kind: string; label: string }
  | { type: "resource"; kind: string; namespace: string | null; name: string; label: string }
  | { type: "crd"; crd: CrdRef; label: string };

const KEY = "srelens.recents";
const LEGACY_KEY = "free" + "lens.recents";
const MAX = 8;

/** Stable identity so re-opening an item de-dupes instead of piling up. */
export function recentId(r: RecentItem): string {
  if (r.type === "view") return `view:${r.kind}`;
  if (r.type === "crd") return `crd:${r.crd.name}`;
  return `res:${r.kind}:${r.namespace ?? ""}:${r.name}`;
}

export function getRecents(): RecentItem[] {
  try {
    const raw = settingsStorage.getItem(KEY) ?? settingsStorage.getItem(LEGACY_KEY);
    return raw ? (JSON.parse(raw) as RecentItem[]) : [];
  } catch {
    return [];
  }
}

/** Record an opened item, moving it to the front (most-recent-first, capped). */
export function pushRecent(item: RecentItem): void {
  const id = recentId(item);
  const next = [item, ...getRecents().filter((r) => recentId(r) !== id)].slice(0, MAX);
  try {
    settingsStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    // ignore quota / unavailable storage
  }
}
