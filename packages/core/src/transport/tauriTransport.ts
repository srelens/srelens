import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { relaunch } from "@tauri-apps/plugin-process";
import { parseClusterLoginRequired, requestClusterLogin } from "../lib/clusterLogin";

/** Request/response to a backend capability. */
export async function invokeCapability<T>(id: string, input: unknown = null): Promise<T> {
  try {
    return await invoke<T>("invoke_capability", { id, input });
  } catch (e) {
    const login = parseClusterLoginRequired(e);
    if (login) {
      requestClusterLogin(login);
      throw new Error("cluster_login_required");
    }
    throw e;
  }
}

/** Invoke a raw Tauri command (for streaming primitives like watches). */
export async function invokeCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(command, args);
}

/** Subscribe to a broadcast event (mirrors ipcRendererOn / broadcastMessage). */
export function on(channel: string, handler: (payload: unknown) => void): () => void {
  const unlistenPromise = listen(channel, (event) => handler(event.payload));
  let disposed = false;
  unlistenPromise.then((un) => {
    if (disposed) un();
  });
  return () => {
    disposed = true;
    unlistenPromise.then((un) => un());
  };
}

/**
 * Subscribe and await registration before resolving. Use this (not `on`) when
 * the backend starts emitting as soon as it's invoked: subscribe first, then
 * start the producer, so the initial emission can't race ahead of the listener.
 */
export async function subscribe(
  channel: string,
  handler: (payload: unknown) => void,
): Promise<() => void> {
  return listen(channel, (event) => handler(event.payload));
}

/** Restart the app (used after an update is installed). */
export async function relaunchApp(): Promise<void> {
  return relaunch();
}

/** The running app's bundle version. */
export async function appVersion(): Promise<string> {
  return getVersion();
}

/** Set the webview's native zoom level (1 = 100%) — the #237 interface scale. */
export async function setWebviewZoom(factor: number): Promise<void> {
  await getCurrentWebview().setZoom(factor);
}

/**
 * Intercept window close request, run an async cleanup handler (e.g. flushing
 * state writes to disk), then destroy the window.
 *
 * The timeout is a *stall* guard, not a success: if the handler has not settled
 * by then, the window stays up and a later close can retry. Destroying over an
 * in-flight flush drops the settings write with the webview.
 */
export function onWindowCloseRequested(
  handler: () => Promise<void> | void,
  timeoutMs = 500,
): () => void {
  const win = getCurrentWindow();
  if (typeof win?.onCloseRequested !== "function") return () => {};
  let unlisten: (() => void) | undefined;
  let disposed = false;
  let closing = false;
  // Only the intentional `win.close()` fallback may proceed without our
  // preventDefault — a second user click while a flush is in flight must not.
  let allowClose = false;
  void win
    .onCloseRequested(async (event) => {
      if (allowClose) return;
      event.preventDefault();
      if (closing) return;
      closing = true;
      let settled = false;
      try {
        await Promise.race([
          Promise.resolve(handler()).then(() => {
            settled = true;
          }),
          new Promise<void>((resolve) => setTimeout(resolve, timeoutMs)),
        ]);
      } catch {
        closing = false;
        return;
      }
      if (!settled) {
        closing = false;
        return;
      }
      try {
        await win.destroy();
      } catch {
        allowClose = true;
        try {
          await win.close();
        } finally {
          allowClose = false;
          closing = false;
        }
      }
    })
    .then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    })
    .catch(() => {});
  return () => {
    disposed = true;
    unlisten?.();
  };
}

/** The window label assigned by Tauri ("main", "ctx-...", etc.). */
export function currentWindowLabel(): string {
  try {
    return getCurrentWindow().label;
  } catch {
    return "main";
  }
}

