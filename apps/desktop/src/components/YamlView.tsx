import React, { Suspense, lazy, useEffect, useState } from "react";
import { getManifest, getSecret, redactSecretManifest, revealSecretManifest } from "@srelens/core";
import { Spinner } from "../ui";
import { ManifestEditor } from "./ManifestEditor";

// CodeMirror is heavy and only needed once a manifest is on screen — load on demand.
const CodeEditor = lazy(() => import("../ui/CodeEditor").then((m) => ({ default: m.CodeEditor })));

/**
 * Manifest view + editor for any resource in the detail drawer. Loads YAML via
 * `k8s.getManifest`, then hands off to the shared {@link ManifestEditor} for
 * editing and server-side apply (behind a confirm).
 *
 * **Except a (core) Secret, which is shown read-only** — the new design's
 * detail-pane YAML (`YamlPane` in ui-next's `detailData.tsx`), not its Edit
 * screen. `k8s.getManifest` is an ungated read: the host blanks a Secret's
 * values on it (#661), and this redacts again on arrival, failing closed,
 * rather than trust that alone. The `data` then shown is what the audited
 * `k8s.getSecret` answered (#1085); if it refuses, the manifest stays
 * redacted. Annotation values stay blanked either way, and a manifest applied
 * back would write those placeholders over the Secret's annotations. So no
 * Apply. (#659)
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
  /** What of a Secret is blanked on screen: every value, only annotation values, or nothing. */
  const [redacted, setRedacted] = useState<"values" | "annotations" | null>("values");
  // The core Secret only. A CRD always has a group, so a custom kind that is
  // merely named `Secret` arrives with `crd` set — the host does not redact
  // it, and its drawer has no Overview tab to send anyone to.
  const isSecret = kind === "Secret" && !crd;

  function load() {
    let active = true;
    setOriginal(null);
    setError("");
    setRedacted("values");
    void getManifest(context, kind, namespace, name, undefined, crd).then(async (out) => {
      if (!active) return;
      if (out.error) {
        setError(out.error);
        return;
      }
      let yaml = out.yaml ?? "";
      if (isSecret) {
        // The values come through `k8s.getSecret`, the audited read, and
        // nothing else: what it answered is put into the redacted manifest,
        // and when it refuses the redacted manifest is what is shown.
        const secret = await getSecret(context, namespace ?? "", name);
        if (!active) return;
        // Fails closed: on a shape it does not understand the redactor returns
        // a message and no YAML at all, and the message is all that is shown.
        const shown = secret.data ? revealSecretManifest(yaml, secret.data) : redactSecretManifest(yaml);
        if (shown.yaml === undefined) {
          setError(shown.error ?? "");
          return;
        }
        yaml = shown.yaml;
        setRedacted("annotationsRedacted" in shown ? (shown.annotationsRedacted ? "annotations" : null) : "values");
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
        {redacted !== null && (
          <p className="text-xs text-muted-foreground" role="status">
            {redacted === "annotations" ? (
              <>
                <strong>Annotation values redacted.</strong> This Secret's data is shown as the cluster has it.
                Its annotation values are not, so it cannot be applied from here.
              </>
            ) : (
              <>
                <strong>Values redacted.</strong> This Secret's values could not be read, so they are not shown
                here.
              </>
            )}
          </p>
        )}
        <Suspense fallback={<Spinner label="Loading editor" />}>
          {/* Copy takes what is on screen, which came through the audited read. */}
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
