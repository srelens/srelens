import { useState } from "react";
import { PlusCircle } from "lucide-react";
import { Button, TextInput } from "../ui";
import { createCluster } from "../lib/webClusters";
import { notify } from "../lib/notify";

export interface WebAddClusterSectionProps {
  /** Called after a cluster is successfully added, so the caller can refresh contexts/cluster lists. */
  onAdded?: () => void;
}

/**
 * Settings → Kubernetes (web mode). Desktop resolves clusters from kubeconfig
 * files on disk; web additionally lets a user define an OIDC-protected
 * cluster directly from fields (no kubeconfig needed) — server, optional CA
 * cert or skip-TLS, and the OIDC issuer/client to sign in with.
 */
export function WebAddClusterSection({ onAdded }: WebAddClusterSectionProps) {
  const [name, setName] = useState("");
  const [server, setServer] = useState("");
  const [caCertPem, setCaCertPem] = useState("");
  const [skipTls, setSkipTls] = useState(false);
  const [issuer, setIssuer] = useState("");
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [extraScopes, setExtraScopes] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  const add = async () => {
    setError("");
    setBusy(true);
    try {
      await createCluster({
        name: name.trim(),
        server: server.trim(),
        caCertPem: caCertPem.trim() || undefined,
        insecureSkipTlsVerify: skipTls,
        oidc: {
          issuer: issuer.trim(),
          clientId: clientId.trim(),
          clientSecret: clientSecret.trim() || undefined,
          extraScopes: extraScopes.split(/[\s,]+/).filter(Boolean),
        },
      });
      notify.success("Cluster added", `${name.trim()} is ready to sign in to.`);
      setName("");
      setServer("");
      setCaCertPem("");
      setSkipTls(false);
      setIssuer("");
      setClientId("");
      setClientSecret("");
      setExtraScopes("");
      onAdded?.();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fl-kubeconfig-sources">
      <div>
        <span>
          <strong>Add an OIDC cluster</strong>
          <small>Define a cluster by its API server and OIDC issuer — no kubeconfig required.</small>
        </span>
      </div>

      <div className="fl-kubeconfig-paste">
        <div>
          <label>
            <span>Name</span>
            <TextInput value={name} onValueChange={setName} placeholder="Team or environment" aria-label="Cluster name" />
          </label>
          <label>
            <span>API server</span>
            <TextInput
              value={server}
              onValueChange={setServer}
              placeholder="https://k8s.example.com:6443"
              aria-label="API server"
            />
          </label>
        </div>

        <label>
          <input type="checkbox" checked={skipTls} onChange={(e) => setSkipTls(e.target.checked)} aria-label="Skip TLS verification" />
          <span>Skip TLS verification (insecure)</span>
        </label>

        {!skipTls && (
          <textarea
            value={caCertPem}
            onChange={(event) => setCaCertPem(event.target.value)}
            placeholder="Optional: paste the cluster's CA certificate (PEM)"
            aria-label="CA certificate"
            spellCheck={false}
          />
        )}

        <div>
          <label>
            <span>OIDC issuer</span>
            <TextInput
              value={issuer}
              onValueChange={setIssuer}
              placeholder="https://issuer.example.com"
              aria-label="OIDC issuer"
            />
          </label>
          <label>
            <span>Client ID</span>
            <TextInput value={clientId} onValueChange={setClientId} placeholder="Client ID" aria-label="OIDC client ID" />
          </label>
        </div>
        <div>
          <label>
            <span>Client secret</span>
            <TextInput
              value={clientSecret}
              onValueChange={setClientSecret}
              placeholder="Optional"
              type="password"
              aria-label="OIDC client secret"
            />
          </label>
          <label>
            <span>Extra scopes</span>
            <TextInput
              value={extraScopes}
              onValueChange={setExtraScopes}
              placeholder="Optional: space or comma separated, e.g. groups offline_access"
              aria-label="Extra OIDC scopes"
            />
          </label>
        </div>

        <footer>
          <Button
            size="sm"
            disabled={!name.trim() || !server.trim() || !issuer.trim() || !clientId.trim() || busy}
            onClick={() => void add()}
          >
            <PlusCircle data-icon="inline-start" /> Add cluster
          </Button>
        </footer>
      </div>
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
