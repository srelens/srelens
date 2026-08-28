import React, { useEffect, useState } from "react";
import { Spinner } from "../ui";
import { loadEditableManifest } from "@srelens/core";
import { ManifestEditor } from "./ManifestEditor";

/**
 * A full-tab editor preloaded with a resource's current manifest (mirroring the
 * New-resource tab). Loads via {@link loadEditableManifest} — which routes
 * Secrets through the gated getSecret path — and applies via the shared
 * {@link ManifestEditor} behind a confirm, toasting the result.
 */
export function EditResourceTab({
  context,
  kind,
  namespace,
  name,
  onEdited,
}: {
  context: string;
  kind: string;
  namespace: string | null;
  name: string;
  /** Called after a successful apply (so the parent can refresh views). */
  onEdited?: () => void;
}) {
  const [yaml, setYaml] = useState<string | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    let active = true;
    setYaml(null);
    setError("");
    void loadEditableManifest(context, kind, namespace, name).then((out) => {
      if (!active) return;
      if (out.error) setError(out.error);
      else setYaml(out.yaml ?? "");
    });
    return () => {
      active = false;
    };
  }, [context, kind, namespace, name]);

  if (error) return <p style={{ color: "var(--fl-color-danger)", padding: 12 }}>Error: {error}</p>;
  if (yaml === null) return <Spinner label="Loading manifest" />;

  return (
    <ManifestEditor
      context={context}
      yaml={yaml}
      onYamlChange={setYaml}
      ariaLabel="Edit resource YAML"
      fill
      headerLabel={`Edit ${kind}/${name}`}
      applyLabel="Apply"
      applyingLabel="Applying…"
      confirm={{ kind, name }}
      onApplied={onEdited}
    />
  );
}
