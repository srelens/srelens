import React, { useEffect, useState } from "react";
import { getManifest } from "@srelens/core";
import { Spinner } from "../ui";
import { ManifestEditor } from "./ManifestEditor";

/**
 * Manifest view + editor for any resource in the detail drawer. Loads YAML via
 * `k8s.getManifest`, then hands off to the shared {@link ManifestEditor} for
 * editing and server-side apply (behind a confirm).
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

  function load() {
    let active = true;
    setOriginal(null);
    setError("");
    void getManifest(context, kind, namespace, name, undefined, crd).then((out) => {
      if (!active) return;
      if (out.error) setError(out.error);
      else {
        setOriginal(out.yaml ?? "");
        setDraft(out.yaml ?? "");
      }
    });
    return () => {
      active = false;
    };
  }

  useEffect(load, [context, kind, namespace, name, crd]);

  if (error) return <p style={{ color: "var(--fl-color-danger)" }}>Error: {error}</p>;
  if (original === null) return <Spinner label="Loading manifest" />;

  return (
    <ManifestEditor
      context={context}
      namespace={namespace ?? undefined}
      // Every kind but one. This view loads through `getManifest`, which
      // redacts nothing, so a Secret's values are on screen in the clear —
      // a gap this design has had since before #656 and one that wants the
      // redaction the new design's pane does, not a Copy button over the top
      // of it. Withheld rather than shipped while that is outstanding.
      copy={kind !== "Secret"}
      yaml={draft}
      onYamlChange={setDraft}
      confirm={{ kind, name }}
      resetTo={original}
      onApplied={load}
    />
  );
}
