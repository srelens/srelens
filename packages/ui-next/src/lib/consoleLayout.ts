import { useSyncExternalStore } from "react";
import { settingsStorage } from "@srelens/core";
import type { Storage } from "./tabsPersist";

/**
 * How the agent console docked along the window's foot is laid out (#944):
 * the height the reader dragged its conversation to, and whether it fills
 * the content area.
 *
 * **The height is kept between launches; maximized is not.** A size is a
 * preference. A console that opened filling the window would hide the page
 * the reader came back to, which is a state, not a preference.
 *
 * Every storage accessor is wrapped, as `peekWidth.ts`'s are: a refusing
 * storage costs the memory and nothing else.
 */
export const CONSOLE_HEIGHT_KEY = "srelens.next.consoleHeight";
export const MIN_CONSOLE_HEIGHT = 96;
/** What the console leaves of the window for the page, tabs and chrome above it. */
export const CONSOLE_PAGE_ROOM = 240;
/**
 * The least the content area above the console keeps while the console is
 * dragged taller (#948): a page's header, its filter row and a few rows, or
 * those and a terminal panel folded to its head. The window bound above
 * cannot know what is drawn there; this is counted from the area itself.
 */
export const MIN_PAGE_ROOM = 160;

/** What the content area above can give the console, measured at `bandHeight`. */
export function consoleSpareAbove(bandHeight: number): number {
  return Math.round(bandHeight - MIN_PAGE_ROOM);
}

let height: number | null | undefined;
let maximized = false;
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

/** The drag's bounds on a window this tall. */
export function consoleHeightBounds(windowHeight: number): { min: number; max: number } {
  return { min: MIN_CONSOLE_HEIGHT, max: Math.max(MIN_CONSOLE_HEIGHT, Math.round(windowHeight - CONSOLE_PAGE_ROOM)) };
}

export function parseStoredConsoleHeight(raw: string | null): number | null {
  if (!raw) return null;
  let doc: unknown;
  try {
    doc = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof doc !== "number" || !Number.isFinite(doc) || doc < MIN_CONSOLE_HEIGHT) return null;
  return Math.round(doc);
}

/** The dragged height, or none: the console is then as tall as its content. */
export function consoleHeight(storage: Storage = settingsStorage): number | null {
  if (height === undefined) {
    try {
      height = parseStoredConsoleHeight(storage.getItem(CONSOLE_HEIGHT_KEY));
    } catch (error) {
      console.error("could not read the saved console height", error);
      height = null;
    }
  }
  return height;
}

export function useConsoleHeight(): number | null {
  return useSyncExternalStore(subscribe, () => consoleHeight(), () => consoleHeight());
}

/** Every step of a drag: shown, not yet written. */
export function setConsoleHeight(next: number): void {
  height = Math.max(MIN_CONSOLE_HEIGHT, Math.round(next));
  emit();
}

/** Once, when the drag settles. */
export function saveConsoleHeight(next: number, storage: Storage = settingsStorage): void {
  setConsoleHeight(next);
  try {
    storage.setItem(CONSOLE_HEIGHT_KEY, JSON.stringify(height));
  } catch (error) {
    console.error("could not persist the console height", error);
  }
}

export function consoleMaximized(): boolean {
  return maximized;
}

export function useConsoleMaximized(): boolean {
  return useSyncExternalStore(subscribe, () => maximized, () => maximized);
}

export function setConsoleMaximized(next: boolean): void {
  if (maximized === next) return;
  maximized = next;
  emit();
}

export function __resetConsoleLayoutForTests(): void {
  height = undefined;
  maximized = false;
  emit();
}
