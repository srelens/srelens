import { useEffect, useState } from "react";
import { FilePlus2, Plug, Trash2, Upload } from "lucide-react";
import { Button, TextInput } from "../ui";
import { list, remove, upload, type KubeconfigMeta } from "@srelens/core";
import { testKubeconfigYaml, type TestResult } from "@srelens/core";

/**
 * Settings → Kubernetes (web mode). Desktop merges local kubeconfig files
 * from disk via a native picker; web has no local filesystem to read, so
 * instead the caller uploads kubeconfig YAML here — sealed at rest on the
 * server and scoped to their account — and the server resolves contexts from
 * whatever's been uploaded.
 */
export function WebKubeconfigSection() {
  const [items, setItems] = useState<KubeconfigMeta[]>([]);
  const [name, setName] = useState("");
  const [yaml, setYaml] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<TestResult | null>(null);

  const refresh = () => void list().then(setItems).catch((e) => setError(String(e)));
  useEffect(refresh, []);

  const test = async () => {
    setError("");
    setTestResult(null);
    setTesting(true);
    try {
      setTestResult(await testKubeconfigYaml(yaml));
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
      await upload(name.trim(), yaml);
      setName("");
      setYaml("");
      refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const onFile = async (file: File) => {
    const text = await file.text();
    setYaml(text);
    if (!name) setName(file.name.replace(/\.[^.]+$/, ""));
  };

  return (
    <div className="fl-kubeconfig-sources">
      <div>
        <span>
          <strong>Kubeconfigs</strong>
          <small>Uploaded kubeconfigs are sealed at rest and used to resolve your contexts.</small>
        </span>
      </div>

      {items.length > 0 && (
        <div className="fl-kubeconfig-sources__files">
          {items.map((k) => (
            <span key={k.id} title={k.name}>
              <code>{k.name}</code>
              <button
                type="button"
                onClick={() => void remove(k.id).then(refresh)}
                aria-label={`Remove kubeconfig ${k.name}`}
              >
                <Trash2 aria-hidden="true" />
              </button>
            </span>
          ))}
        </div>
      )}

      <div className="fl-kubeconfig-paste">
        <div>
          <label>
            <span>Name</span>
            <TextInput
              value={name}
              onValueChange={setName}
              placeholder="Team or environment"
              aria-label="Kubeconfig name"
            />
          </label>
          <label className="fl-context-editor__upload">
            <Upload aria-hidden="true" />
            <span>Choose a file</span>
            <input
              type="file"
              accept=".yaml,.yml,.config,.kubeconfig,text/plain"
              onChange={(e) => e.target.files?.[0] && void onFile(e.target.files[0])}
              aria-label="Upload kubeconfig file"
            />
          </label>
        </div>
        <textarea
          value={yaml}
          onChange={(event) => setYaml(event.target.value)}
          placeholder="…or paste kubeconfig YAML here"
          aria-label="Kubeconfig YAML"
          spellCheck={false}
        />
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
            disabled={!yaml.trim() || testing || busy}
            onClick={() => void test()}
          >
            <Plug data-icon="inline-start" /> {testing ? "Testing…" : "Test connection"}
          </Button>
          <Button size="sm" disabled={!yaml.trim() || busy || testing} onClick={() => void add()}>
            <FilePlus2 data-icon="inline-start" /> Upload kubeconfig
          </Button>
        </footer>
      </div>
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
