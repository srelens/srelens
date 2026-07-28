// Detects the "this OIDC cluster needs an interactive sign-in" signal — which
// arrives either as an HTTP 401 body or as an async WebSocket stream event —
// and prompts the user to sign in, on both web (full-page redirect to the
// server's OIDC login route) and desktop (a Tauri command that opens the
// browser and captures the loopback callback).
import { isWeb } from "../transport/platform";
import { invokeCommand } from "../transport/transport";
import { notify } from "./notify";

export const CLUSTER_LOGIN_MARKER = "NEEDS_CLUSTER_LOGIN";

export interface ClusterLoginInfo {
  key: string;
  context: string;
  loginUrl: string;
}

function loginUrlForKey(key: string): string {
  return `/auth/cluster/login?key=${encodeURIComponent(key)}`;
}

/** Pull `(key, context)` out of a raw `NEEDS_CLUSTER_LOGIN:<key>:<context>`
 * marker string. The key is hex (no colon), so the first two colon-separated
 * segments are the prefix + key and the remainder is the context. */
function fromMarker(text: string): ClusterLoginInfo | null {
  const idx = text.indexOf(`${CLUSTER_LOGIN_MARKER}:`);
  if (idx < 0) return null;
  const rest = text.slice(idx + CLUSTER_LOGIN_MARKER.length + 1);
  const sep = rest.indexOf(":");
  if (sep < 0) return null;
  const key = rest.slice(0, sep);
  const context = rest.slice(sep + 1);
  if (!key || !context) return null;
  return { key, context, loginUrl: loginUrlForKey(key) };
}

/** Recognize the needs-login signal in any shape: the HTTP 401 JSON body, a
 * raw marker string, or an object carrying the marker in an `error` field. */
export function parseClusterLoginRequired(value: unknown): ClusterLoginInfo | null {
  if (typeof value === "string") return fromMarker(value);
  if (value && typeof value === "object") {
    const obj = value as Record<string, unknown>;
    if (obj.error === "cluster_login_required" && typeof obj.key === "string") {
      const key = obj.key;
      const context = typeof obj.context === "string" ? obj.context : "";
      // Always DERIVE the login URL from the (opaque hex) key rather than
      // trusting a `loginUrl` string from the response body — the redirect
      // target is then a fixed same-origin path (`/auth/cluster/login?key=…`)
      // that can't be steered to `javascript:` or an external host.
      return { key, context, loginUrl: loginUrlForKey(key) };
    }
    // A stream event may nest the marker in an `error` string.
    if (typeof obj.error === "string") return fromMarker(obj.error);
  }
  return null;
}

// One prompt per key within a short window, so a burst of failing calls (a
// dashboard polling many resources) doesn't stack dozens of identical toasts.
const promptedAt = new Map<string, number>();
const PROMPT_COOLDOWN_MS = 15_000;

export function requestClusterLogin(info: ClusterLoginInfo): void {
  const now = Date.now();
  const last = promptedAt.get(info.key);
  if (last !== undefined && now - last < PROMPT_COOLDOWN_MS) return;
  promptedAt.set(info.key, now);
  const where = info.context ? `“${info.context}”` : "this cluster";
  if (!isWeb) {
    // Desktop: the same managed flow, driven by a Tauri command that opens
    // the browser and captures the loopback callback.
    notify.clusterSignIn(
      `Sign in to ${where}`,
      "This cluster uses OIDC and needs you to sign in.",
      () => {
        void invokeCommand("cluster_login", { key: info.key })
          .then(() => window.location.reload())
          .catch((e) => notify.error("Sign-in failed", String(e)));
      },
    );
    return;
  }
  notify.clusterSignIn(
    `Sign in to ${where}`,
    "This cluster uses OIDC and needs you to sign in.",
    () => {
      window.location.href = info.loginUrl;
    },
  );
}

/** Test-only: clear the per-key dedupe window. */
export function __resetClusterLoginDedupeForTests(): void {
  promptedAt.clear();
}
