import { csrfHeader } from "../transport/webTransport";

export interface Me {
  userId: number;
  email: string;
  displayName: string;
}

/** GET /api/me → the signed-in identity, or null when unauthenticated. */
export async function fetchMe(): Promise<Me | null> {
  const res = await fetch("/api/me", { credentials: "include", headers: { ...csrfHeader() } });
  if (res.status === 401) return null;
  if (!res.ok) throw new Error(`session check failed: ${res.status}`);
  const data = await res.json();
  return { userId: data.user_id, email: data.email, displayName: data.display_name };
}

export const loginUrl = "/auth/login";

/** POST /auth/dev-login (only succeeds when the server enables dev login). */
export async function devLogin(): Promise<void> {
  const res = await fetch("/auth/dev-login", { method: "POST", credentials: "include", headers: { ...csrfHeader() } });
  if (!res.ok && res.status !== 302) throw new Error("dev login is not enabled");
}

/** POST /auth/logout, then reload to the login screen. */
export async function logout(): Promise<void> {
  await fetch("/auth/logout", { method: "POST", credentials: "include", headers: { ...csrfHeader() } });
  if (typeof location !== "undefined") location.reload();
}
