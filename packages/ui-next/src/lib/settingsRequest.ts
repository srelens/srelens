import { openTab } from "./tabsStore";

/**
 * Another screen asking Settings to show one of its sections — Home's "Browse
 * catalog" asks for Apps with its Catalog tab.
 *
 * Settings keeps its section in component state, and `/settings` is one tab,
 * so the request is held as well as announced: a Settings tab that mounts
 * takes it on mount, and one already open hears it. Same shape as
 * `extensions/actionRequests.ts`.
 */
export interface SettingsRequest {
  section: string;
  /** A tab within the section; the Apps section's `catalog` is the one asked for today. */
  tab?: string;
}

let held: SettingsRequest | null = null;
const listeners = new Set<() => void>();

/** Show Settings on `section` (and `tab` within it), opening the tab if it is not open. */
export function openSettings(section: string, tab?: string): void {
  held = tab === undefined ? { section } : { section, tab };
  for (const listener of [...listeners]) listener();
  openTab("/settings");
}

/** The request waiting for Settings, once; `null` when there is none. */
export function takeSettingsRequest(): SettingsRequest | null {
  const request = held;
  held = null;
  return request;
}

export function onSettingsRequested(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
