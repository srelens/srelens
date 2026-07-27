// Web-mode persistence of the open workspace tabs. In the desktop shell the
// window stays open, so tab state lives only in memory; in a browser a reload
// wipes React state and would dump the user back on the landing page. To avoid
// that, web mode serializes the open tabs + active tab to localStorage and
// rehydrates them on load. Desktop is a no-op (isTauri guard).

import { isTauri } from "../transport/platform";
import type { ViewTab } from "../App";

const KEY = "srelens.openTabs";

export interface PersistedWorkspace {
  tabs: ViewTab[];
  activeTabId: number | null;
}

/** Transient editor tabs carry unsaved content that lives outside App state,
 * so restoring them would show a blank editor — drop them from persistence. */
function isRestorable(t: ViewTab): boolean {
  return !t.create && !t.edit;
}

/**
 * Restore the open tabs + active tab from a prior web session. Web-only.
 * Deep-link `focus` (a session-scoped nonce) is stripped so a reload doesn't
 * re-trigger a stale deep-link. Returns null when there is nothing valid to
 * restore (no storage, empty, or a parse/shape error) so the caller falls back
 * to the landing page.
 */
export function loadOpenTabs(): PersistedWorkspace | null {
  if (isTauri()) return null;
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as PersistedWorkspace;
    if (!parsed || !Array.isArray(parsed.tabs)) return null;
    const tabs = parsed.tabs
      .filter(
        (t): t is ViewTab =>
          !!t && typeof t.id === "number" && typeof t.kind === "string",
      )
      .filter(isRestorable)
      .map((t) => {
        const { focus: _focus, ...rest } = t;
        return rest as ViewTab;
      });
    if (tabs.length === 0) return null;
    const activeTabId = tabs.some((t) => t.id === parsed.activeTabId)
      ? parsed.activeTabId
      : tabs[0].id;
    return { tabs, activeTabId };
  } catch {
    return null;
  }
}

/** The next tab id to hand out, so restored ids are never reused. */
export function nextTabId(tabs: ViewTab[]): number {
  return tabs.reduce((max, t) => Math.max(max, t.id), 0) + 1;
}

/** Persist the open tabs + active tab (web-only, best-effort). */
export function saveOpenTabs(tabs: ViewTab[], activeTabId: number | null): void {
  if (isTauri()) return;
  try {
    const persist = tabs.filter(isRestorable);
    if (persist.length === 0) {
      localStorage.removeItem(KEY);
      return;
    }
    const active = persist.some((t) => t.id === activeTabId)
      ? activeTabId
      : persist[0].id;
    localStorage.setItem(KEY, JSON.stringify({ tabs: persist, activeTabId: active }));
  } catch {
    // localStorage full or disabled — best-effort, ignore.
  }
}
