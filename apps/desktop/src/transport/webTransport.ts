// Web-mode transport: capability/command RPC over HTTP to the srelens server.
// Streaming (on/subscribe) is provided by ./wsClient (wired in the next task).

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
  if (res.status === 401) throw new Error("unauthenticated");
  const text = await res.text();
  const data = text ? JSON.parse(text) : null;
  if (!res.ok) {
    const message = data && typeof data === "object" && "error" in data ? String(data.error) : res.statusText;
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

// Placeholder streaming exports; replaced by ./wsClient wiring in the next task.
export function on(_channel: string, _handler: (payload: unknown) => void): () => void {
  throw new Error("web streaming not wired yet");
}
export async function subscribe(_channel: string, _handler: (payload: unknown) => void): Promise<() => void> {
  throw new Error("web streaming not wired yet");
}
