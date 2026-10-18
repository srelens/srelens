import { useSyncExternalStore } from "react";
import { settingsStorage } from "@srelens/core";
import type { Storage } from "./tabsPersist";

/**
 * The terminal panel a page can show along its foot (#942): a shell opened
 * from the Pods list stays under the Pods list instead of taking the reader to
 * another tab.
 *
 * **A panel is its tab's.** Each tab has at most one, holding one session, and
 * keeps it while the reader is on another tab — `TabSurface` keeps hidden tabs
 * mounted, and this store is what they read on the way back. A tab that is
 * closed takes nothing with it but the entry: the session is
 * `lib/sessions.ts`'s, still listed in Terminals, where it is ended.
 *
 * **Closing the panel is not ending the shell.** The tab remembers the session
 * it last showed, so opening its terminal again shows the same shell rather
 * than starting another — the way a docked terminal in an editor behaves.
 *
 * **The size is the reader's, not the tab's.** One share of the page, kept
 * between launches: half the page until it is dragged. A share rather than
 * pixels, so it means the same thing on a short window and a tall one.
 */
export interface TerminalPanelState {
  /**
   * The `lib/sessions.ts` row the panel shows: its in-process counter, not a
   * secret of any kind. Not called `sessionId`, which code scanning reads as
   * a security token and then traces the tab id's `Math.random()` into.
   */
  shellId: number;
  /** Whether the panel is showing; a closed one remembers its session. */
  open: boolean;
  /** Filling the page, with the page's own content behind it. */
  maximized: boolean;
}

export const TERMINAL_PANEL_SHARE_KEY = "srelens.next.terminalPanelShare";
export const DEFAULT_TERMINAL_PANEL_SHARE = 0.5;
export const MIN_TERMINAL_PANEL_SHARE = 0.15;
export const MAX_TERMINAL_PANEL_SHARE = 0.85;
/** Neither the panel nor the page above it gets shorter than this. */
export const MIN_TERMINAL_PANEL_PX = 96;

const clampShare = (share: number) =>
  Math.max(MIN_TERMINAL_PANEL_SHARE, Math.min(MAX_TERMINAL_PANEL_SHARE, share));

let panels: Readonly<Record<string, TerminalPanelState>> = {};
let share: number | null = null;
/** A session the Terminals tab is asked to show once it is on screen. */
let focusRequest: number | null = null;
const listeners = new Set<() => void>();

function emit() {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function put(tabId: string, next: TerminalPanelState | undefined) {
  const { [tabId]: _, ...rest } = panels;
  panels = next ? { ...rest, [tabId]: next } : rest;
  emit();
}

/** The tab's panel, open or remembered, or nothing. */
export function terminalPanel(tabId: string): TerminalPanelState | undefined {
  return panels[tabId];
}

export function useTerminalPanel(tabId: string): TerminalPanelState | undefined {
  return useSyncExternalStore(subscribe, () => panels[tabId], () => panels[tabId]);
}

/** Show `shellId` in the tab's panel, opening it at the reader's size. */
export function showTerminalPanel(tabId: string, shellId: number): void {
  const current = panels[tabId];
  put(tabId, { shellId, open: true, maximized: current?.shellId === shellId ? current.maximized : false });
}

/** Hide the tab's panel. The shell keeps running and is shown again next time. */
export function closeTerminalPanel(tabId: string): void {
  const current = panels[tabId];
  if (current?.open) put(tabId, { ...current, open: false, maximized: false });
}

/** Forget the tab's panel: its session has ended, or moved to the Terminals tab. */
export function forgetTerminalPanel(tabId: string): void {
  if (panels[tabId]) put(tabId, undefined);
}

export function setTerminalPanelMaximized(tabId: string, maximized: boolean): void {
  const current = panels[tabId];
  if (current && current.maximized !== maximized) put(tabId, { ...current, maximized });
}

export function parseStoredTerminalPanelShare(raw: string | null): number | null {
  if (!raw) return null;
  let doc: unknown;
  try {
    doc = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof doc !== "number" || !Number.isFinite(doc) || doc <= 0 || doc >= 1) return null;
  return clampShare(doc);
}

/**
 * The share of the page the panel takes, read once from storage. Every
 * accessor is wrapped, as `peekWidth.ts`'s are: a refusing storage costs the
 * memory and nothing else.
 */
export function terminalPanelShare(storage: Storage = settingsStorage): number {
  if (share === null) {
    let stored: number | null = null;
    try {
      stored = parseStoredTerminalPanelShare(storage.getItem(TERMINAL_PANEL_SHARE_KEY));
    } catch (error) {
      console.error("could not read the saved terminal panel size", error);
    }
    share = stored ?? DEFAULT_TERMINAL_PANEL_SHARE;
  }
  return share;
}

export function useTerminalPanelShare(): number {
  return useSyncExternalStore(subscribe, () => terminalPanelShare(), () => terminalPanelShare());
}

/** Every step of a drag: shown, not yet written. */
export function setTerminalPanelShare(next: number): void {
  share = clampShare(next);
  emit();
}

/** Once, when the drag settles. */
export function saveTerminalPanelShare(next: number, storage: Storage = settingsStorage): void {
  setTerminalPanelShare(next);
  try {
    storage.setItem(TERMINAL_PANEL_SHARE_KEY, JSON.stringify(share));
  } catch (error) {
    console.error("could not persist the terminal panel size", error);
  }
}

/** Ask the Terminals tab to show `shellId` — the panel's "Open in tab". */
export function requestTerminalsFocus(shellId: number): void {
  focusRequest = shellId;
  emit();
}

export function terminalsFocusRequest(): number | null {
  return focusRequest;
}

export function useTerminalsFocusRequest(): number | null {
  return useSyncExternalStore(subscribe, () => focusRequest, () => focusRequest);
}

/** The request has been acted on. */
export function clearTerminalsFocusRequest(): void {
  if (focusRequest === null) return;
  focusRequest = null;
  emit();
}

export function __resetTerminalPanelsForTests(): void {
  panels = {};
  share = null;
  focusRequest = null;
  emit();
}
