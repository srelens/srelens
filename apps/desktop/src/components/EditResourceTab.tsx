import React, { useEffect, useRef, useState } from "react";
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
  draft,
  onDraftChange,
  onEdited,
}: {
  context: string;
  kind: string;
  namespace: string | null;
  name: string;
  /** The loaded/edited working copy owned by this tab; null means load it. */
  draft: string | null;
  onDraftChange: (yaml: string) => void;
  /** Called after a successful apply (so the parent can refresh views). */
  onEdited?: () => void;
}) {
  const [error, setError] = useState("");
  const onDraftChangeRef = useRef(onDraftChange);
  onDraftChangeRef.current = onDraftChange;

  useEffect(() => {
    // Returning to a tab with a working copy must neither show a loading flash
    // nor fetch the cluster's current manifest over unsaved edits.
    if (draft !== null) return;
    let active = true;
    setError("");
    void loadEditableManifest(context, kind, namespace, name).then((out) => {
      if (!active) return;
      if (out.error) setError(out.error);
      else onDraftChangeRef.current(out.yaml ?? "");
    });
    return () => {
      active = false;
    };
  }, [context, kind, namespace, name, draft]);

  if (error) return <p style={{ color: "var(--fl-color-danger)", padding: 12 }}>Error: {error}</p>;
  if (draft === null) return <Spinner label="Loading manifest" />;

  return (
    <ManifestEditor
      context={context}
      yaml={draft}
      onYamlChange={onDraftChange}
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
