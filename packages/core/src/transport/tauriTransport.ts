import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { getCurrentWebview } from "@tauri-apps/api/webview";
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
