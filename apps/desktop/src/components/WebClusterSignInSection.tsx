import { useEffect, useState } from "react";
import { LogIn, LogOut, Pencil } from "lucide-react";
import { Button } from "../ui";
import { listClusters, clusterLogout, type OidcClusterRow } from "../lib/webClusters";
import { notify } from "../lib/notify";
import { isTauri } from "../transport/platform";
import { invokeCommand } from "../transport/transport";

export interface WebClusterSignInSectionProps {
  /** Bumped by the parent (e.g. after adding a cluster) to force a refresh. */
  refreshNonce?: number;
  /** Edit a cluster's config: called with a context name to prefill the form. */
  onEdit?: (context: string) => void;
}

/**
 * Settings → Kubernetes (web mode). Lists the caller's OIDC clusters with
 * their sign-in status, and offers a per-cluster Sign in (full-page redirect
 * into the OIDC login flow) / Sign out. This is what makes the "lazy prompt"
 * (login triggered on first API 401) discoverable ahead of time, and gives an
 * explicit way to end a cluster's session.
 */
export function WebClusterSignInSection({ refreshNonce, onEdit }: WebClusterSignInSectionProps) {
  const [rows, setRows] = useState<OidcClusterRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [signingOut, setSigningOut] = useState<Set<string>>(new Set());

  const refresh = () => {
    setLoading(true);
    setError("");
    listClusters()
      .then(setRows)
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  };

  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(refresh, [refreshNonce]);

  const signIn = (key: string) => {
    if (isTauri()) {
      // Desktop: the same managed flow via the Tauri command (opens the browser
      // + captures the loopback callback), then refresh the list.
      void invokeCommand("cluster_login", { key })
        .then(refresh)
        .catch((e) => notify.error("Sign-in failed", String(e)));
      return;
    }
    window.location.href = `/auth/cluster/login?key=${encodeURIComponent(key)}`;
  };

  const signOut = async (row: OidcClusterRow) => {
    if (signingOut.has(row.key)) return;
    setSigningOut((prev) => new Set(prev).add(row.key));
    try {
      await clusterLogout(row.key);
      notify.success("Signed out", row.contexts.join(", ") || row.issuer);
      refresh();
    } catch (e) {
      notify.error("Sign out failed", String(e));
    } finally {
      setSigningOut((prev) => {
        const next = new Set(prev);
        next.delete(row.key);
        return next;
      });
    }
  };

  return (
    <div className="fl-kubeconfig-sources">
      <div>
        <span>
          <strong>Cluster sign-in</strong>
          <small>Sign in or out of your OIDC-protected clusters.</small>
        </span>
      </div>

      {loading ? (
        <p className="fl-settings-context-state">Loading…</p>
      ) : error ? (
        <p role="alert">{error}</p>
      ) : rows.length === 0 ? (
        <p className="fl-settings-context-state">
          No OIDC clusters. Add one above, or upload a kubeconfig with an OIDC user.
        </p>
      ) : (
        <div className="fl-cluster-signin-list">
          {rows.map((row) => {
            const label = row.contexts.length > 0 ? row.contexts.join(", ") : row.issuer;
            const busy = signingOut.has(row.key);
            return (
              <div key={row.key} className="fl-cluster-signin-row">
                <span>
                  <strong>{label}</strong>
                  <small>{row.issuer}</small>
                  <small>
                    {row.signedIn
                      ? `Signed in${row.expiresAt ? ` · expires ${new Date(row.expiresAt * 1000).toLocaleString()}` : ""}`
                      : "Not signed in"}
                  </small>
                </span>
                <span className="fl-kubeconfig-sources__actions">
                  {onEdit && row.contexts[0] && (
                    <Button size="sm" variant="ghost" onClick={() => onEdit(row.contexts[0])}>
                      <Pencil data-icon="inline-start" /> Edit
                    </Button>
                  )}
                  {row.signedIn ? (
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={busy}
                      onClick={() => void signOut(row)}
                    >
                      <LogOut data-icon="inline-start" /> Sign out
                    </Button>
                  ) : (
                    <Button size="sm" variant="ghost" onClick={() => signIn(row.key)}>
                      <LogIn data-icon="inline-start" /> Sign in
                    </Button>
                  )}
                </span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
