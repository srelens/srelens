import { currentWorkspace, openTab } from "./tabsStore";

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
  /**
   * The Settings tab `openTab` brought to the front. Two Settings tabs can be
   * open at once (a duplicate), and `openTab` focuses the first in strip
   * order, which need not be the screen that subscribed first.
   */
  tabId: string;
}

let held: SettingsRequest | null = null;
const listeners = new Set<() => void>();

/** Show Settings on `section` (and `tab` within it), opening the tab if it is not open. */
export function openSettings(section: string, tab?: string): void {
  openTab("/settings");
  const tabId = currentWorkspace().activeId;
  held = tab === undefined ? { section, tabId } : { section, tab, tabId };
  for (const listener of [...listeners]) listener();
}

/**
 * The request waiting for the Settings screen in tab `tabId`, once; `null`
 * when there is none or it is another tab's. A screen rendered outside any tab
 * (`null`) takes whatever is waiting.
 */
export function takeSettingsRequest(tabId: string | null): SettingsRequest | null {
  if (!held || (tabId !== null && held.tabId !== tabId)) return null;
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
