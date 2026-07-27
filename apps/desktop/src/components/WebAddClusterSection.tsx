import { useState } from "react";
import { PlusCircle, Plug } from "lucide-react";
import { Button, TextInput } from "../ui";
import { addCluster, testClusterForm, type CreateClusterInput, type TestResult } from "../lib/addCluster";
import { notify } from "../lib/notify";

export interface WebAddClusterSectionProps {
  /** Called after a cluster is added. On desktop the argument is the saved
   * kubeconfig file path (so the caller can track it); on web it's undefined. */
  onAdded?: (savedPath?: string) => void;
}

/**
 * "Add an OIDC cluster" form — works on desktop and web. It synthesizes a
 * one-context kubeconfig (using the `exec` kubelogin form) from the fields. On
 * desktop the file is saved locally and native kubelogin authenticates it; on
 * web the server stores it and a srelens-managed token authenticates it. A
 * "Test connection" button probes reachability before saving.
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
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<TestResult | null>(null);

  const canSubmit =
    !!name.trim() && !!server.trim() && !!issuer.trim() && !!clientId.trim();

  const buildInput = (): CreateClusterInput => ({
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

  const test = async () => {
    setError("");
    setTestResult(null);
    setTesting(true);
    try {
      setTestResult(await testClusterForm(buildInput()));
    } catch (e) {
      setError(String(e));
    } finally {
      setTesting(false);
    }
  };

  const add = async () => {
    setError("");
    setBusy(true);
    try {
      const savedPath = await addCluster(buildInput());
      notify.success("Cluster added", `${name.trim()} is ready to use.`);
      setName("");
      setServer("");
      setCaCertPem("");
      setSkipTls(false);
      setIssuer("");
      setClientId("");
      setClientSecret("");
      setExtraScopes("");
      setTestResult(null);
      onAdded?.(savedPath);
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

        {testResult && (
          <p
            role="status"
            className={testResult.reachable ? "fl-cluster-test--ok" : "fl-cluster-test--fail"}
          >
            {testResult.reachable
              ? `✓ Reachable${testResult.version ? ` — ${testResult.version}` : ""}${
                  testResult.error ? ` (${testResult.error})` : ""
                }`
              : `✕ Not reachable: ${testResult.error ?? "unknown error"}`}
          </p>
        )}

        <footer>
          <Button
            variant="outline"
            size="sm"
            disabled={!canSubmit || testing || busy}
            onClick={() => void test()}
          >
            <Plug data-icon="inline-start" /> {testing ? "Testing…" : "Test connection"}
          </Button>
          <Button size="sm" disabled={!canSubmit || busy || testing} onClick={() => void add()}>
            <PlusCircle data-icon="inline-start" /> Add cluster
          </Button>
        </footer>
      </div>
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
