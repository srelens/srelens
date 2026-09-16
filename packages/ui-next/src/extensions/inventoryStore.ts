import { useSyncExternalStore } from "react";
import {
  EXTENSIONS_CHANGED,
  isTauri,
  listExtensions,
  type ExtensionInventory,
  type InstalledExtension,
} from "@srelens/core";

/** What to call an installed app on screen. A quarantined app's stored name is one the
 *  host no longer accepts, and it may hold characters that display as another app's name,
 *  so such an app is named by its ID until it is reinstalled or removed. */
export function extensionLabel(plugin: InstalledExtension): string {
  return plugin.quarantined ? plugin.manifest.id : plugin.manifest.name;
}

type Snapshot = {
  status: "loading" | "ready" | "error";
  data?: ExtensionInventory;
  error?: string;
};
const loading: Snapshot = { status: "loading" };
let snapshot = loading;
const listeners = new Set<() => void>();
let poll: { refresh(force?: boolean): Promise<void>; stop(): void } | undefined;

function publish(next: Snapshot) {
  if (JSON.stringify(snapshot) === JSON.stringify(next)) return;
  snapshot = next;
  for (const listener of listeners) listener();
}

/** One poll per window, independent of how many retained tabs consume it. */
function start() {
  let active = true;
  let pending = false;
  let queued = false;
  let generation = 0;
  async function refresh(force = false) {
    if (pending) {
      // A lifecycle change must not be lost behind an older in-flight read.
      if (force) { queued = true; generation++; }
      return;
    }
    pending = true;
    const mine = generation;
    try {
      const data = isTauri() ? await listExtensions() : {
        schemaVersion: 1, nextRevision: 1, plugins: [],
      };
      if (active && mine === generation) publish({ status: "ready", data });
    } catch (error) {
      if (active && mine === generation) {
        publish({ status: "error", error: error instanceof Error ? error.message : String(error) });
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
  window.addEventListener("focus", onChange);
  const timer = isTauri() ? window.setInterval(() => void refresh(), 5000) : undefined;
  void refresh();
  return {
    refresh,
    stop() {
      active = false;
      window.clearInterval(timer);
      window.removeEventListener(EXTENSIONS_CHANGED, onChange);
      window.removeEventListener("focus", onChange);
    },
  };
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) poll = start();
  return () => {
    listeners.delete(listener);
    if (!listeners.size) {
      poll?.stop();
      poll = undefined;
      snapshot = loading;
    }
  };
}
const getSnapshot = () => snapshot;
const reload = () => { void poll?.refresh(true); };
export function useExtensions() {
  return { ...useSyncExternalStore(subscribe, getSnapshot, getSnapshot), reload };
}
