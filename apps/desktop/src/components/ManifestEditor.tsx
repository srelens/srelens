import React, { Suspense, lazy, useEffect, useRef, useState } from "react";
import { parseAllDocuments } from "yaml";
import { CircleCheck, Undo2, Upload } from "lucide-react";
import {
  applyManifest,
  diffManifest,
  parseResourceVersion,
  validateManifest,
  type ApplyDoc,
  type Conflict,
  type DiffDoc,
} from "@srelens/core";
import { notify } from "@srelens/core";
import { openApiSchema } from "@srelens/core";
import { useAccess, rbac, reportActionError, type AccessCheck } from "@srelens/core/react";
// Pure kind mapping, so it comes from the React-free entry point.
import { kindToResource } from "@srelens/core";
import { describeError } from "@srelens/core";
import { Spinner, Button, ConfirmDialog } from "../ui";
import { DiffView } from "./DiffView";

// CodeMirror is heavy and only needed where a manifest is edited — load on demand.
const CodeEditor = lazy(() => import("../ui/CodeEditor").then((m) => ({ default: m.CodeEditor })));

/**
 * The one YAML manifest editor shared by the New-resource tab, the Edit tab,
 * and the drawer YAML view. Wraps CodeMirror (YAML highlighting + schema
 * validation/completion for the context) with a server-side apply that
 * optionally confirms first and always toasts the result.
 *
 * `yaml` is controlled by the parent so callers can swap templates (create) or
 * load from the cluster (edit). `fill` pins the editor to fill a tab; otherwise
 * it grows within a bounded height for the drawer.
 */
export function ManifestEditor({
  context,
  yaml,
  onYamlChange,
  ariaLabel = "Manifest YAML",
  fill = false,
  applyLabel = "Apply",
  applyingLabel = "Applying…",
  applyIcon,
  confirm,
  resetTo,
  headerExtras,
  headerLabel,
  onApplied,
}: {
  context: string;
  yaml: string;
  onYamlChange: (yaml: string) => void;
  ariaLabel?: string;
  /** Fill the parent (tab); otherwise render at a bounded height (drawer). */
  fill?: boolean;
  applyLabel?: string;
  applyingLabel?: string;
  applyIcon?: React.ReactNode;
  /** When set, Apply opens a confirm dialog naming this resource first. */
  confirm?: { kind: string; name: string };
  /** When set, show a Reset button that reverts the draft to this text. */
  resetTo?: string;
  /** Extra header content on the left (e.g. a create-template picker). */
  headerExtras?: React.ReactNode;
  /** Short header title (e.g. "New resource" / "Edit ConfigMap/web"). */
  headerLabel?: string;
  /** Called with the applied object on success. */
  onApplied?: (result: { kind: string; name: string }) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [confirming, setConfirming] = useState(false);
  const [result, setResult] = useState<{ kind: string; name: string } | null>(null);
  const [conflictDocs, setConflictDocs] = useState<ApplyDoc[] | null>(null);
  const [diffDocs, setDiffDocs] = useState<DiffDoc[] | null>(null);
  const [diffing, setDiffing] = useState(false);
  const [showDiff, setShowDiff] = useState(false);
  const loadedRvRef = React.useRef<string | null>(null);
  // Set after a successful apply so the next background diff rebaselines
  // `loadedRvRef` to the freshly-bumped resourceVersion — otherwise the user's
  // OWN apply would be mistaken for an external change ("Changed elsewhere").
  const rebaselineRef = React.useRef(false);
  const diffHandleRef = useRef<HTMLDivElement>(null);
  const [diffWidth, setDiffWidth] = useState(480);
  const diffStartX = useRef(0);
  const diffStartW = useRef(0);
  const diffWidthRef = useRef(diffWidth);
  diffWidthRef.current = diffWidth;

  useEffect(() => {
    if (!showDiff) return;
    const handle = diffHandleRef.current;
    if (!handle) return;
    function move(e: MouseEvent) {
      // Divider sits on the LEFT edge of the diff pane: dragging left grows it.
      setDiffWidth(Math.max(300, Math.min(1100, diffStartW.current - (e.clientX - diffStartX.current))));
    }
    function up() {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.style.userSelect = "";
    }
    function down(e: MouseEvent) {
      e.preventDefault();
      diffStartX.current = e.clientX;
      diffStartW.current = diffWidthRef.current;
      document.body.style.userSelect = "none";
      window.addEventListener("mousemove", move);
      window.addEventListener("mouseup", up);
    }
    handle.addEventListener("mousedown", down);
    return () => {
      handle.removeEventListener("mousedown", down);
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.body.style.userSelect = "";
    };
  }, [showDiff]);

  // Capture the resourceVersion of the first loaded manifest, for stale detection.
  React.useEffect(() => {
    if (loadedRvRef.current == null && yaml.trim()) {
      loadedRvRef.current = parseResourceVersion(yaml);
    }
  }, [yaml]);

  // Derive the edited resource's identity from the FIRST YAML document so Apply
  // can be gated on whether the user is authorized to patch it. `parseAllDocuments`
  // (not `parse`, which THROWS on multi-document input) handles multi-doc streams;
  // we gate on the first document. Only EDIT mode (`confirm` set) is gated: the
  // New-resource tab creates (verb `create`, not `patch`), so gating on `patch`
  // there would wrongly disable Apply for a user who can create but not patch.
  // Unknown/CRD kinds (kindToResource → null) resolve to `null` too — Apply stays
  // enabled and the API server remains the source of truth for those.
  const identity: AccessCheck | null = (() => {
    if (!confirm) return null; // only gate edits; creates use a different verb (create)
    try {
      const docs = parseAllDocuments(yaml);
      const first = docs[0]?.toJS() as { kind?: string; metadata?: { namespace?: string } } | null;
      if (!first?.kind) return null;
      const res = kindToResource(first.kind);
      if (!res) return null;
      const namespace = first.metadata?.namespace;
      // No declared namespace: the resource relies on the context's default, so
      // a preflight built with an empty namespace would be cluster-scoped and
      // could FALSE-disable Apply. Skip gating — the API server still enforces.
      if (!namespace) return null;
      return rbac.edit(res.group, res.resource, namespace);
    } catch {
      return null;
    }
  })();
  const access = useAccess(context, identity ? [identity] : []);
  // Fail-CLOSED (matches DetailActions/NodeCordonAction and the spec): disabled
  // while the check is still loading OR resolved as denied. The explanatory
  // title only appears once the check has resolved as denied.
  const applyDenied = identity ? !access.allowed(identity) : false;
  const applyDeniedTitle =
    identity && access.known(identity) && !access.allowed(identity)
      ? `You don't have permission to patch ${identity.resource}${identity.namespace ? ` in ${identity.namespace}` : ""}`
      : undefined;

  const conflictEntries = (conflictDocs ?? []).filter(
    (d): d is ApplyDoc & { conflict: Conflict } => d.conflict != null,
  );
  const staleRv =
    diffDocs?.find((d) => d.currentResourceVersion && loadedRvRef.current && d.currentResourceVersion !== loadedRvRef.current)
      ?.currentResourceVersion ?? null;

  async function doApply(force: boolean) {
    setBusy(true);
    setError("");
    const out = await applyManifest(context, yaml, force);
    setBusy(false);
    if (out.error) {
      setError(describeError(out.error).detail);
      setConflictDocs(null);
      // Actionable toast; a 403 also invalidates this context's access cache.
      reportActionError(context, `Failed to apply ${confirm?.name ?? "resource"}`, out.error);
      return;
    }
    const docs = out.documents ?? [];
    const conflicted = docs.filter((d) => d.conflict);
    const failed = docs.filter((d) => d.error);
    // Surface every document that failed outright (inline error + toast).
    const surfaceFailed = (failedDocs: ApplyDoc[]) => {
      const names = failedDocs.map((d) => `${d.kind}/${d.name}`).join(", ");
      const label =
        failedDocs.length > 1 ? `Failed to apply ${failedDocs.length} documents: ${names}` : `Failed to apply ${names}`;
      const detail = failedDocs.map((d) => d.error).filter(Boolean).join("; ") || "apply failed";
      setError(label);
      // Actionable toast; a 403 also invalidates this context's access cache.
      reportActionError(context, label, detail);
    };
    // Close the confirm dialog for any resolved response (conflict, failure, or
    // success) — the conflict banner and error text render inline, outside the
    // (now-inert) dialog, so they'd otherwise never be reachable.
    setConfirming(false);
    if (conflicted.length > 0) {
      setConflictDocs(docs);
      // A different document may have hard-failed alongside the conflict — show
      // that error now too, rather than hiding it until the user clicks Force.
      if (failed.length > 0) surfaceFailed(failed);
      return;
    }
    setConflictDocs(null);
    if (failed.length > 0) {
      surfaceFailed(failed);
      return;
    }
    const first = docs[0];
    const applied = { kind: first?.kind ?? "", name: first?.name ?? "" };
    setResult(applied);
    // The apply bumped the live resourceVersion; rebaseline off the next diff so
    // this own change isn't later flagged as "Changed elsewhere".
    rebaselineRef.current = true;
    const label = docs.length > 1 ? `Applied ${docs.length} resources` : `Applied ${applied.kind || "resource"} ${applied.name}`.trim();
    notify.success(label);
    onApplied?.(applied);
  }

  function onApplyClick() {
    setConflictDocs(null);
    if (confirm) setConfirming(true);
    else void doApply(false);
  }

  // Debounced dry-run diff against the cluster. Runs in the background in
  // fill mode regardless of whether the Changes panel is open, so a
  // concurrently-modified resource is flagged (via `staleRv`, derived from
  // `diffDocs`) without the user having to open the panel first — the panel
  // itself only renders when `showDiff` is true. (The drawer has no diff
  // panel, so it's fine for non-fill callers to skip this.) Guarded by
  // `active` so a stale response from a superseded edit can't clobber a
  // newer one.
  React.useEffect(() => {
    if (!fill || !yaml.trim()) {
      setDiffDocs(null);
      return;
    }
    let active = true;
    setDiffing(true);
    const t = setTimeout(() => {
      void diffManifest(context, yaml).then((out) => {
        if (!active) return;
        const docs = out.error ? [] : out.documents ?? [];
        // After a successful apply, adopt the freshly-bumped resourceVersion as
        // the new baseline so the user's OWN change isn't flagged as external.
        if (rebaselineRef.current) {
          const rv = docs.find((d) => d.currentResourceVersion)?.currentResourceVersion;
          if (rv) {
            loadedRvRef.current = rv;
            rebaselineRef.current = false;
          }
        }
        setDiffDocs(docs);
        setDiffing(false);
      });
    }, 700);
    return () => {
      active = false;
      clearTimeout(t);
    };
  }, [context, yaml, fill]);

  const editor = (
    <Suspense fallback={<Spinner label="Loading editor" />}>
      <CodeEditor
        value={yaml}
        onChange={onYamlChange}
        language="yaml"
        ariaLabel={ariaLabel}
        fill={fill}
        minHeight={fill ? undefined : 320}
        maxHeight={fill ? undefined : 520}
        schemaValidate={(y) =>
          validateManifest(context, y).then((r) => (r.valid === false ? r.errors ?? [] : []))
        }
        schemaSource={(apiVersion, kind) =>
          openApiSchema(context, apiVersion, kind).then((r) => ("error" in r ? null : r))
        }
      />
    </Suspense>
  );

  const applyButton = (
    <Button
      onClick={onApplyClick}
      disabled={busy || !yaml.trim() || (resetTo != null && yaml === resetTo) || applyDenied}
      title={applyDeniedTitle}
    >
      {busy ? <Spinner label={applyingLabel} data-icon="inline-start" /> : applyIcon ?? <Upload data-icon="inline-start" />}
      {busy ? applyingLabel : applyLabel}
    </Button>
  );

  const confirmDialog = confirming ? (
    <ConfirmDialog
      title="Apply manifest?"
      message={
        <>
          <p style={{ marginTop: 0 }}>
            Server-side apply the edited <code>{confirm?.kind}</code> <code>{confirm?.name}</code> to the cluster?
          </p>
          {identity && (
            <p>
              This authorizes patch on {identity.resource}
              {identity.namespace ? ` in ${identity.namespace}` : ""}.
            </p>
          )}
          {error && <p style={{ color: "var(--fl-color-danger)" }}>Error: {error}</p>}
        </>
      }
      confirmLabel="Apply"
      busy={busy}
      onConfirm={() => void doApply(false)}
      onCancel={() => setConfirming(false)}
    />
  ) : null;

  const conflictBanner = conflictEntries.length > 0 ? (
    <div className="fl-apply-conflict" role="alert">
      <div className="fl-apply-conflict__list">
        {conflictEntries.map((d, i) => (
          <p key={`${d.kind}/${d.name}/${i}`} className="fl-apply-conflict__item" title={d.conflict.message}>
            <strong>
              {d.kind}/{d.name}
            </strong>{" "}
            conflicts with <strong>{d.conflict.managers.join(", ") || "another manager"}</strong>
            {d.conflict.fields.length > 0 && <> on {d.conflict.fields.join(", ")}</>}.
          </p>
        ))}
      </div>
      <Button variant="danger" onClick={() => void doApply(true)} disabled={busy}>
        {busy ? <Spinner label="Forcing…" data-icon="inline-start" /> : <Upload data-icon="inline-start" />}
        Force apply
      </Button>
    </div>
  ) : null;

  const inlineError =
    error && !confirming ? (
      <p className="fl-apply-inline-error" style={{ color: "var(--fl-color-danger)", margin: 0 }} title={error}>
        Error: {error}
      </p>
    ) : null;

  if (fill) {
    return (
      <div className="flex min-h-0 flex-1 flex-col bg-background">
        <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border px-3 py-2 text-sm">
          {headerLabel && <span className="font-medium">{headerLabel}</span>}
          <span className="text-xs text-muted-foreground">on {context}</span>
          {headerExtras}
          <div className="ml-auto flex items-center gap-3">
            {staleRv && (
              <span className="fl-apply-stale" title="The live object changed since you opened it">
                Changed elsewhere
              </span>
            )}
            <Button variant="ghost" onClick={() => setShowDiff((v) => !v)}>
              {showDiff ? "Hide changes" : "Changes"}
            </Button>
            {result && (
              <span className="fl-apply-success">
                <CircleCheck aria-hidden="true" />
                Applied {result.kind} <code>{result.name}</code>
              </span>
            )}
            {error && !confirming && (
              <span className="max-w-md truncate text-destructive" title={error}>
                Error: {error}
              </span>
            )}
            {applyButton}
          </div>
        </div>
        {conflictBanner}
        <div className="flex min-h-0 flex-1 overflow-hidden">
          <div className="relative min-h-0 flex-1 overflow-hidden">
            {/* Absolute-inset pins CodeMirror to a definite-height box so it fills the tab. */}
            <div className="absolute inset-0">{editor}</div>
          </div>
          {showDiff && (
            <>
              <div className="fl-changes-panel__resize" ref={diffHandleRef} aria-hidden="true" />
              <div
                className="fl-changes-panel min-h-0 shrink-0 overflow-auto border-l border-border"
                style={{ width: diffWidth }}
              >
                {diffing && !diffDocs ? (
                  <Spinner label="Computing diff" />
                ) : diffDocs && diffDocs.length > 0 ? (
                  diffDocs.map((d, i) => <DiffView key={`${d.kind}/${d.name}/${i}`} doc={d} />)
                ) : (
                  <p className="fl-diff__empty">No changes</p>
                )}
              </div>
            </>
          )}
        </div>
        {confirmDialog}
      </div>
    );
  }

  const dirty = resetTo == null || yaml !== resetTo;
  return (
    <div>
      {editor}
      {conflictBanner}
      {inlineError}
      <div style={{ display: "flex", gap: 8, alignItems: "center", marginTop: 8 }}>
        {applyButton}
        {resetTo != null && (
          <Button variant="ghost" onClick={() => onYamlChange(resetTo)} disabled={!dirty}>
            <Undo2 data-icon="inline-start" />
            Reset
          </Button>
        )}
        {result && !dirty && (
          <span className="fl-apply-success">
            <CircleCheck aria-hidden="true" /> Applied
          </span>
        )}
      </div>
      {confirmDialog}
    </div>
  );
}
