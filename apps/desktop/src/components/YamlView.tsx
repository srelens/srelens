import React, { Suspense, lazy, useEffect, useState } from "react";
import { getManifest, redactSecretManifest } from "@srelens/core";
import { Spinner } from "../ui";
import { ManifestEditor } from "./ManifestEditor";

// CodeMirror is heavy and only needed once a manifest is on screen — load on demand.
const CodeEditor = lazy(() => import("../ui/CodeEditor").then((m) => ({ default: m.CodeEditor })));

/**
 * Manifest view + editor for any resource in the detail drawer. Loads YAML via
 * `k8s.getManifest`, then hands off to the shared {@link ManifestEditor} for
 * editing and server-side apply (behind a confirm).
 *
 * **Except a (core) Secret, which is shown redacted and read-only** — the new
 * design's detail-pane YAML (`YamlPane` in ui-next's `detailData.tsx`), not
 * its Edit screen. `k8s.getManifest` is an ungated read: the host blanks a
 * Secret's values on it (#661), and this redacts again on arrival with
 * `redactSecretManifest`, failing closed, rather than trust that alone. Then
 * there is nothing on screen worth applying: a redacted manifest applied back
 * writes its placeholders over every value and annotation the Secret has. So
 * no Apply, and the reader is sent to the Overview tab, whose per-key reveal
 * reads the values through the consent-gated `k8s.getSecret`. (#659)
 */
export function YamlView({
  context,
  kind,
  namespace,
  name,
  crd,
}: {
  context: string;
  kind: string;
  namespace: string | null;
  name: string;
  /** Dynamic GVK for custom resources (not in the static kind table). */
  crd?: { group: string; version: string; plural: string };
}) {
  const [original, setOriginal] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState("");
  // The core Secret only. A CRD always has a group, so a custom kind that is
  // merely named `Secret` arrives with `crd` set — the host does not redact
  // it, and its drawer has no Overview tab to send anyone to.
  const isSecret = kind === "Secret" && !crd;

  function load() {
    let active = true;
    setOriginal(null);
    setError("");
    void getManifest(context, kind, namespace, name, undefined, crd).then((out) => {
      if (!active) return;
      if (out.error) {
        setError(out.error);
        return;
      }
      let yaml = out.yaml ?? "";
      if (isSecret) {
        // Fails closed: on a shape it does not understand the redactor returns
        // a message and no YAML at all, and the message is all that is shown.
        const redacted = redactSecretManifest(yaml);
        if (redacted.error !== undefined) {
          setError(redacted.error);
          return;
        }
        yaml = redacted.yaml ?? "";
      }
      setOriginal(yaml);
      setDraft(yaml);
    });
    return () => {
      active = false;
    };
  }

  useEffect(load, [context, kind, namespace, name, crd]);

  if (error) return <p style={{ color: "var(--fl-color-danger)" }}>Error: {error}</p>;
  if (original === null) return <Spinner label="Loading manifest" />;

  if (isSecret) {
    return (
      <div className="flex flex-col gap-2">
        {/* Told, not silently shown less: blanked values read as an empty
            Secret, and as disagreeing with `kubectl get -o yaml` for no
            reason anyone could see. */}
        <p className="text-xs text-muted-foreground" role="status">
          <strong>Values redacted.</strong> This Secret's values are not shown here. Reveal them one key at a
          time on the Overview tab.
        </p>
        <Suspense fallback={<Spinner label="Loading editor" />}>
          {/* Copy is safe here: what it copies is the redacted text. */}
          <CodeEditor value={original} readOnly copy ariaLabel="Manifest YAML" minHeight={320} maxHeight={520} />
        </Suspense>
      </div>
    );
  }

  return (
    <ManifestEditor
      context={context}
      namespace={namespace ?? undefined}
      copy
      yaml={draft}
      onYamlChange={setDraft}
      confirm={{ kind, name }}
      resetTo={original}
      onApplied={load}
    />
  );
}
