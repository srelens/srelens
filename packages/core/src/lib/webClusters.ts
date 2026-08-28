// Web-mode cluster management: define an OIDC cluster from a form, list the
// user's OIDC clusters + sign-in status, and sign out of one. Raw fetch (like
// lib/webKubeconfigs.ts), not the capability transport.
import { isTauri } from "../transport/platform";
import { invokeCommand } from "../transport/transport";
import { csrfHeader } from "../transport/webTransport";

export interface OidcClusterRow {
  key: string;
  issuer: string;
  clientId: string;
  contexts: string[];
  signedIn: boolean;
  expiresAt: number | null;
}

export interface CreateClusterInput {
  name: string;
  server: string;
  caCertPem?: string;
  insecureSkipTlsVerify: boolean;
  oidc?: { issuer: string; clientId: string; clientSecret?: string; extraScopes?: string[] };
}

async function parseError(res: Response): Promise<never> {
  const data = await res.json().catch(() => ({}));
  throw new Error((data as { error?: string }).error ?? `request failed: ${res.status}`);
}

/** GET /api/clusters — the caller's defined OIDC clusters and sign-in status.
 * On desktop, lists them via the Tauri command instead (no server involved). */
export async function listClusters(): Promise<OidcClusterRow[]> {
  if (isTauri()) return invokeCommand<OidcClusterRow[]>("list_clusters");
  const res = await fetch("/api/clusters", { credentials: "include", headers: { ...csrfHeader() } });
  if (!res.ok) await parseError(res);
  const data = (await res.json()) as { clusters?: OidcClusterRow[] };
  return data.clusters ?? [];
}

/** POST /api/clusters — define a new OIDC cluster from form fields. */
export async function createCluster(input: CreateClusterInput): Promise<void> {
  const res = await fetch("/api/clusters", {
    method: "POST",
    credentials: "include",
    headers: { "Content-Type": "application/json", ...csrfHeader() },
    body: JSON.stringify(input),
  });
  if (!res.ok) await parseError(res);
}

/** POST /api/clusters/:key/logout — sign out of a cluster's OIDC session. On
 * desktop, signs out via the Tauri command instead. */
export async function clusterLogout(key: string): Promise<void> {
  if (isTauri()) {
    await invokeCommand("cluster_logout", { key });
    return;
  }
  const res = await fetch(`/api/clusters/${encodeURIComponent(key)}/logout`, {
    method: "POST",
    credentials: "include",
    headers: { ...csrfHeader() },
  });
  if (!res.ok) await parseError(res);
}
