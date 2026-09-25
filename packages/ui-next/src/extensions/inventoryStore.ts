import { useSyncExternalStore } from "react";
import {
  EXTENSIONS_CHANGED,
  isTauri,
  listExtensions,
  onExtensionInventoryChanged,
  type ExtensionInventory,
  type InstalledExtension,
} from "@srelens/core";

/** What to call an installed app on screen. A quarantined app's stored name is one the
 *  host no longer accepts, and it may hold characters that display as another app's name,
 *  so such an app is named by its ID until it is reinstalled or removed. */
export function extensionLabel(plugin: InstalledExtension): string {
  return plugin.quarantined ? plugin.manifest.id : plugin.manifest.name;
}

/**
 * How the list learns of a change made elsewhere (#566): `live` when the host
 * announces every inventory write, `polling` when that channel could not be
 * listened to (with why) and the list is read every five seconds instead, and
 * `none` on the web. The web server keeps each user's inventory (#515) but
 * announces nothing, so the list is read when this window changes it and when
 * the window gains focus, which is where a change made in another tab shows.
 */
export type InventoryUpdates =
  | { mode: "live" }
  | { mode: "polling"; reason: string }
  | { mode: "none" };

type Snapshot = {
  status: "loading" | "ready" | "error";
  data?: ExtensionInventory;
  error?: string;
  updates?: InventoryUpdates;
};
const loading: Snapshot = { status: "loading" };
let snapshot = loading;
const listeners = new Set<() => void>();
let feed: { refresh(force?: boolean): Promise<void>; stop(): void } | undefined;

/** How often the fallback reads, when the host's announcements cannot be heard. */
const FALLBACK_POLL_MS = 5000;

function publish(next: Snapshot) {
  if (JSON.stringify(snapshot) === JSON.stringify(next)) return;
  snapshot = next;
  for (const listener of listeners) listener();
}

/** One feed per window, independent of how many retained tabs consume it. */
function start() {
  let active = true;
  let pending = false;
  let queued = false;
  let generation = 0;
  // Unknown until the host's channel answers: the list does not claim to be live before it is.
  let updates: InventoryUpdates | undefined = isTauri() ? undefined : { mode: "none" };
  let latest: Omit<Snapshot, "updates"> = loading;
  const show = () => publish({ ...latest, ...(latest.status === "loading" || !updates ? {} : { updates }) });
  async function refresh(force = false) {
    if (pending) {
      // A lifecycle change must not be lost behind an older in-flight read.
      if (force) { queued = true; generation++; }
      return;
    }
    pending = true;
    const mine = generation;
    try {
      const data = await listExtensions();
      if (active && mine === generation) { latest = { status: "ready", data }; show(); }
    } catch (error) {
      if (active && mine === generation) {
        latest = { status: "error", error: error instanceof Error ? error.message : String(error) };
        show();
      }
    } finally {
      pending = false;
      if (active && queued) {
        queued = false;
        void refresh();
      }
    }
  }
  const onChange = () => void refresh(true);
  window.addEventListener(EXTENSIONS_CHANGED, onChange);
  // Focus still reads: it is what catches a write by another process, which
  // this host cannot announce.
  window.addEventListener("focus", onChange);
  let timer: number | undefined;
  let unlisten: (() => void) | undefined;
  if (isTauri()) {
    onExtensionInventoryChanged(onChange).then(
      (stop) => {
        if (!active) { stop(); return; }
        unlisten = stop;
        updates = { mode: "live" };
        show();
      },
      (error: unknown) => {
        if (!active) return;
        // Say so rather than look live: the list is now only as fresh as the last poll.
        updates = { mode: "polling", reason: error instanceof Error ? error.message : String(error) };
        timer = window.setInterval(() => void refresh(), FALLBACK_POLL_MS);
        show();
      },
    );
  }
  void refresh();
  return {
    refresh,
    stop() {
      active = false;
      unlisten?.();
      window.clearInterval(timer);
      window.removeEventListener(EXTENSIONS_CHANGED, onChange);
      window.removeEventListener("focus", onChange);
    },
  };
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) feed = start();
  return () => {
    listeners.delete(listener);
    if (!listeners.size) {
      feed?.stop();
      feed = undefined;
      snapshot = loading;
    }
  };
}
const getSnapshot = () => snapshot;
const reload = () => { void feed?.refresh(true); };
export function useExtensions() {
  return { ...useSyncExternalStore(subscribe, getSnapshot, getSnapshot), reload };
}
