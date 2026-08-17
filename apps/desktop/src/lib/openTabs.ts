// Session restore: the open workspace tabs + active tab survive a restart.
//
// This began as web-only (a browser reload wipes React state and dumps the
// user on the landing page). Issue #159 extends it to the desktop, which used
// to start blank every launch. Both now go through `settingsStorage`, so the
// desktop writes into the durable settings file (#34) while web keeps using
// localStorage — one code path, no platform branch.

import { loadRestoreSession } from "./settings";
import { settingsStorage } from "./settingsStorage";
import type { CrdRef } from "./crds";
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
  // Opting out starts fresh but deliberately leaves the stored snapshot
  // alone, so turning the setting back on restores the last real session
  // instead of nothing.
  if (!loadRestoreSession()) return null;
  try {
    const raw = settingsStorage.getItem(KEY);
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
  if (!loadRestoreSession()) return;
  try {
    const persist = tabs.filter(isRestorable);
    if (persist.length === 0) {
      settingsStorage.removeItem(KEY);
      return;
    }
    const active = persist.some((t) => t.id === activeTabId)
      ? activeTabId
      : persist[0].id;
    settingsStorage.setItem(KEY, JSON.stringify({ tabs: persist, activeTabId: active }));
  } catch {
    // Storage full, disabled, or the settings write failed — best-effort.
  }
}

/**
 * How long to gather tab changes before writing the session.
 *
 * Search text lives on the tab (#254), so every keystroke mutates `tabs`.
 * Writing per keystroke means a full read-modify-rewrite of settings.json —
 * under an interprocess lock, ending in `sync_all` — for each character, which
 * queues fsyncs behind each other and delays unrelated settings writes.
 */
const SAVE_DEBOUNCE_MS = 400;

let pendingSave: { tabs: ViewTab[]; activeTabId: number | null } | null = null;
let saveTimer: ReturnType<typeof setTimeout> | null = null;

/**
 * Persist the session, coalescing bursts. The in-memory state is untouched —
 * only the durable write is delayed — so the UI stays instant.
 *
 * The timer is NOT restarted by later changes: the write lands a fixed delay
 * after the first change of a burst, carrying the newest snapshot. A restarting
 * debounce would starve during continuous typing and never write at all.
 */
export function scheduleSaveOpenTabs(tabs: ViewTab[], activeTabId: number | null): void {
  pendingSave = { tabs, activeTabId };
  if (saveTimer) return;
  saveTimer = setTimeout(() => {
    saveTimer = null;
    const snapshot = pendingSave;
    pendingSave = null;
    if (snapshot) saveOpenTabs(snapshot.tabs, snapshot.activeTabId);
  }, SAVE_DEBOUNCE_MS);
}

/**
 * Write any coalesced session immediately. Called when the window is going
 * away, so the last keystrokes before a quit are not lost with the timer.
 */
export function flushSaveOpenTabs(): void {
  if (saveTimer) {
    clearTimeout(saveTimer);
    saveTimer = null;
  }
  const snapshot = pendingSave;
  pendingSave = null;
  if (snapshot) saveOpenTabs(snapshot.tabs, snapshot.activeTabId);
}

/**
 * Drop restored tabs whose cluster is no longer among the discovered
 * contexts, returning the survivors and how many were dropped. Cluster-less
 * tabs (Settings, Toolbox, the landing view) are always kept — they do not
 * depend on a context existing.
 */
/**
 * The active tab id after a prune: keep it when it survived, otherwise fall
 * back to the first remaining tab, or null when nothing is left. A stale id
 * would leave the workspace blank behind a populated tab strip, and would
 * also stop the native close command taking its no-tabs path.
 */
export function reconcileActiveTab(tabs: ViewTab[], activeTabId: number | null): number | null {
  if (tabs.length === 0) return null;
  return tabs.some((t) => t.id === activeTabId) ? activeTabId : tabs[0].id;
}

/**
 * Reconcile restored CRD tabs for one context against the CRDs actually
 * discovered there. A CRD that is gone drops its tab; a CRD that still exists
 * under a different served version (or plural) has its stale `CrdRef` replaced
 * with the current one, so the tab keeps working instead of querying a version
 * the cluster no longer serves.
 *
 * Only ever called with a SUCCESSFUL discovery result: an unreachable cluster
 * must not be read as "this CRD is gone" and silently delete the workspace.
 */
export function reconcileCrdTabs(
  tabs: ViewTab[],
  context: string,
  crds: readonly CrdRef[],
): { tabs: ViewTab[]; dropped: number } {
  const current = new Map(crds.map((c) => [`${c.group}/${c.kind}`, c]));
  let dropped = 0;
  const kept: ViewTab[] = [];
  for (const t of tabs) {
    if (t.cluster !== context || !t.crd) {
      kept.push(t);
      continue;
    }
    const live = current.get(`${t.crd.group}/${t.crd.kind}`);
    if (!live) {
      dropped += 1;
      continue;
    }
    const stale = live.version !== t.crd.version || live.plural !== t.crd.plural;
    kept.push(stale ? { ...t, crd: live } : t);
  }
  return { tabs: kept, dropped };
}

export function pruneMissingContexts(
  tabs: ViewTab[],
  contexts: readonly string[],
): { tabs: ViewTab[]; dropped: number } {
  const available = new Set(contexts);
  const kept = tabs.filter((t) => !t.cluster || available.has(t.cluster));
  return { tabs: kept, dropped: tabs.length - kept.length };
}
