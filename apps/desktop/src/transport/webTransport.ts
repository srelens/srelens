// Web-mode transport: capability/command RPC over HTTP to the srelens server.
// Streaming (on/subscribe) is backed by the multiplexed socket in ./wsClient.

import wsClient from "./wsClient";
import { parseClusterLoginRequired, requestClusterLogin } from "../lib/clusterLogin";

const CSRF = typeof crypto !== "undefined" && "randomUUID" in crypto
  ? crypto.randomUUID()
  : "srelens-web";

/** The CSRF header every /api request must carry (presence, not value, is checked). */
export function csrfHeader(): Record<string, string> {
  return { "X-Srelens-Csrf": CSRF };
}

async function post<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(path, {
    method: "POST",
    credentials: "include",
    headers: { "Content-Type": "application/json", ...csrfHeader() },
    body: JSON.stringify(body ?? null),
  });
  const text = await res.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = null;
  }
  if (res.status === 401) {
    const login = parseClusterLoginRequired(data);
    if (login) {
      requestClusterLogin(login);
      throw new Error("cluster_login_required");
    }
    throw new Error("unauthenticated"); // app-session expired, not a cluster
  }
  if (!res.ok) {
    const message = data && typeof data === "object" && "error" in data ? String((data as Record<string, unknown>).error) : res.statusText;
    throw new Error(message);
  }
  return data as T;
}

export async function invokeCapability<T>(id: string, input: unknown = null): Promise<T> {
  return post<T>(`/api/capability/${encodeURIComponent(id)}`, input);
}

export async function invokeCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return post<T>(`/api/command/${encodeURIComponent(command)}`, args ?? {});
}

export async function relaunchApp(): Promise<void> {
  // No relaunch in a browser; a reload is the closest equivalent.
  if (typeof location !== "undefined") location.reload();
}

export async function appVersion(): Promise<string> {
  return (import.meta.env.VITE_APP_VERSION as string | undefined) ?? "web";
}

export function on(channel: string, handler: (payload: unknown) => void): () => void {
  let dispose = () => {};
  let disposed = false;
  // Fire-and-forget: exec/port-forward subscribe by a server-assigned id and
  // don't await the ack (parity with the desktop's synchronous `on`).
  void wsClient.subscribeChannel(channel, handler).then((d) => {
    if (disposed) d();
    else dispose = d;
  });
  return () => {
    disposed = true;
    dispose();
  };
}

export async function subscribe(channel: string, handler: (payload: unknown) => void): Promise<() => void> {
  // Await the `subbed` ack before resolving, so the caller can safely start
  // the producer without losing the first frame (watch/logs/terminal/helm).
  return wsClient.subscribeChannel(channel, handler, { awaitAck: true });
}
