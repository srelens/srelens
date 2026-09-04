import { useEffect, useMemo, useState } from "react";
import {
  applyManifest,
  deleteResource,
  describeError,
  diffManifest,
  getManifest,
  getSecret,
  notify,
  parseResourceVersion,
  redactSecretManifest,
  validateManifest,
  type ApplyDoc,
  type ClusterContext,
  type DiffDoc,
  type DynamicGvk,
} from "@srelens/core";
import { useNamespaceOptions } from "@srelens/core/react";
import {
  Button,
  CodeEditor,
  ConfirmDialog,
  DiffLines,
  EmptyState,
  Screen,
  Select,
} from "@srelens/ui-kit";
import { getKubeconfigFiles, useActiveContext } from "../lib/clusters";
import { useClusterGate } from "../lib/clusterMoved";
import { isBuiltInKind, resolveCrdGvk } from "../lib/crdGvk";
import { parseEditRoute, type DetailRouteParts } from "../lib/detailRoute";
import { FailureAlert } from "../lib/errorCopy";
import { useResource } from "../lib/useResource";
import { NoClusterScreen } from "./resourceShell";

/**
 * The resource editor: one YAML manifest, applied server-side.
 *
 * Two routes land here. `/edit/<kind>/<namespace>/<name>` — what the row
 * menu's `Edit` and the detail pane's footer mint — loads the live manifest
 * and edits it in place; `/new` starts from a template and creates. Both were
 * routes before this screen existed: the menu opened a correctly titled tab
 * onto the Placeholder, which is what "clicking Edit does nothing" was.
 *
 * **Every write goes to the cluster the tab was opened on**, not to whatever
 * the rail points at now. The route names a resource, and a resource belongs
 * to one cluster; a reader who moved the rail mid-edit is told, and asked to
 * confirm, exactly as the row menu's dialogs do. See `lib/clusterMoved`.
 *
 * **Apply is server-side apply**, so the same button creates and updates, and
 * a field another manager owns comes back as a conflict rather than an
 * overwrite — the reader chooses to force it, and is told whose field it was.
 *
 * What is NOT here yet: schema-driven completion in the editor. The kit takes
 * a completion source and core has the schema helpers, but wiring the two
 * needs CodeMirror's autocomplete types in this package, which it does not
 * yet depend on. Validation — a strict dry run against the API server — is
 * wired, and is the half that catches a wrong manifest before it is applied.
 */
export function EditResource({ route }: { route: string }) {
  const context = useActiveContext();
  const creating = route === "/new";
  const parts = creating ? null : parseEditRoute(route);
  const title = parts ? `Edit ${parts.name}` : "New resource";

  if (!context) return <NoClusterScreen title={title} noun="resources" />;

  if (!creating && !parts) {
    // Unreachable through `screenFor`, which sends a route here only once
    // `parseEditRoute` has accepted it — but a route string can arrive from a
    // persisted session, and a tab that says what is wrong with it is worth
    // more than a blank one.
    return (
      <Screen title="Edit" eyebrow={context.name} fill>
        <div className="p-4">
          <FailureAlert
            title={`${route} does not name a resource`}
            error="An editor tab's route is /edit/<kind>/<namespace>/<name>. Close this tab and open the resource from its list."
          />
        </div>
      </Screen>
    );
  }

  // Keyed by cluster and route, so a tab that is re-pointed starts its draft
  // over rather than carrying one resource's edits onto another.
  return parts ? (
    <EditExisting key={`${context.stableId}:${route}`} context={context} parts={parts} />
  ) : (
    <NewResource key={context.stableId} context={context} />
  );
}

/**
 * What the API server answers about an apply, reduced to what the screen has
 * to say next: nothing, a conflict to force, or a failure to show.
 */
function outcome(docs: ApplyDoc[]): { conflicts: ApplyDoc[]; failures: ApplyDoc[] } {
  return {
    conflicts: docs.filter((d) => d.conflict),
    failures: docs.filter((d) => d.error),
  };
}

function failureLine(failures: ApplyDoc[]): string {
  const names = failures.map((d) => `${d.kind}/${d.name}`).join(", ");
  const detail = failures.map((d) => d.error).filter(Boolean).join("; ");
  return `${failures.length > 1 ? `Failed to apply ${failures.length} documents: ${names}` : `Failed to apply ${names}`}${
    detail ? ` — ${detail}` : ""
  }`;
}

/**
 * The parts of applying a manifest that edit and create share: the call, the
 * conflict, the failure, the success — and the cluster gate in front of all of
 * them. The two screens differ only in where the YAML came from and in what
 * the button is called.
 */
function useApply({
  pinned,
  live,
  verb,
  onApplied,
}: {
  pinned: string;
  live: string;
  verb: string;
  onApplied: (docs: ApplyDoc[]) => void;
}) {
  const gate = useClusterGate({ pinned, live, verb });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [conflicts, setConflicts] = useState<ApplyDoc[]>([]);
  const [applied, setApplied] = useState<{ kind: string; name: string } | null>(null);

  async function apply(yaml: string, force: boolean) {
    // Asked before anything else: it is the only question whose answer changes
    // what every other name on the screen refers to.
    if (gate.refusal) {
      setError(gate.refusal);
      return false;
    }
    setBusy(true);
    setError("");
    setApplied(null);
    const out = await applyManifest(pinned, yaml, force);
    setBusy(false);
    if (out.error) {
      setError(describeError(out.error).detail);
      setConflicts([]);
      return false;
    }
    const docs = out.documents ?? [];
    // An emptied editor applies nothing, and the API answers with no
    // documents — neither a conflict nor a failure, so it used to read as a
    // success, toast one, and reload the unchanged object.
    if (docs.length === 0) {
      setError("Nothing to apply: the manifest has no documents in it.");
      setConflicts([]);
      return false;
    }
    const { conflicts: conflicted, failures } = outcome(docs);
    setConflicts(conflicted);
    // A conflict and a hard failure can arrive together; the failure is shown
    // now rather than after the reader has forced the conflict.
    if (failures.length > 0) setError(failureLine(failures));
    if (conflicted.length > 0 || failures.length > 0) return false;
    const first = docs[0];
    const result = { kind: first?.kind ?? "", name: first?.name ?? "" };
    setApplied(result);
    notify.success(
      docs.length > 1
        ? `Applied ${docs.length} resources`
        : `Applied ${result.kind || "resource"} ${result.name}`.trim(),
    );
    onApplied(docs);
    return true;
  }

  return { gate, busy, error, setError, conflicts, applied, apply };
}

/** The dry-run validation the editor lints with: strict, against the API server. */
function validator(context: string) {
  return (yaml: string) =>
    validateManifest(context, yaml).then((r) => (r.valid === false ? (r.errors ?? []) : []));
}

/** The conflict banner: who owns the field, and the one way past it. */
function Conflicts({
  conflicts,
  busy,
  onForce,
}: {
  conflicts: ApplyDoc[];
  busy: boolean;
  onForce: () => void;
}) {
  if (conflicts.length === 0) return null;
  return (
    <div
      role="alert"
      className="flex flex-wrap items-center gap-3 rounded-md border border-warn/40 bg-warn-wash px-3 py-2 text-sm"
    >
      <div className="min-w-0 flex-1">
        {conflicts.map((d, i) => (
          <p key={`${d.kind}/${d.name}/${i}`} className="break-words" title={d.conflict?.message}>
            <strong>
              {d.kind}/{d.name}
            </strong>{" "}
            conflicts with <strong>{d.conflict?.managers.join(", ") || "another manager"}</strong>
            {d.conflict && d.conflict.fields.length > 0 && <> on {d.conflict.fields.join(", ")}</>}.
          </p>
        ))}
        <p className="text-xs text-muted">
          Forcing takes those fields over; the other manager will see its next write conflict.
        </p>
      </div>
      <Button variant="danger" onClick={onForce} disabled={busy}>
        {busy ? "Forcing…" : "Force apply"}
      </Button>
    </div>
  );
}

/**
 * The Changes panel: what applying the draft would do, per document, from a
 * dry run — and the one line that matters most, whether the live object has
 * moved since the manifest was loaded.
 */
function Changes({ docs, computing }: { docs: DiffDoc[] | null; computing: boolean }) {
  if (computing && !docs) return <p className="p-3 text-sm text-muted">Comparing with the cluster…</p>;
  if (!docs || docs.length === 0) return <p className="p-3 text-sm text-muted">No changes.</p>;
  return (
    <div className="flex flex-col gap-4 p-3">
      {docs.map((d, i) => (
        <section key={`${d.kind}/${d.name}/${i}`}>
          <h3 className="mb-1 text-[10px] uppercase text-faint">
            {d.kind} {d.name}
            {!d.exists && " · will be created"}
            {d.exists && !d.changed && " · unchanged"}
          </h3>
          {d.changed && <DiffLines rows={d.rows} />}
        </section>
      ))}
    </div>
  );
}

function EditExisting({ context, parts }: { context: ClusterContext; parts: DetailRouteParts }) {
  // The cluster this tab was opened on. Reads and writes go here, whatever the
  // rail does later.
  const [pinned] = useState(context.name);
  const { kind, namespace, name } = parts;

  /**
   * A Secret's values stay out of the DOM until the reader reveals them.
   *
   * `k8s.getManifest` returns them in the clear, so the manifest is redacted
   * on arrival — the same redactor the detail pane's YAML view uses, failing
   * closed on any shape it does not understand — and the editor shows that,
   * read-only, until Reveal. Reveal goes through `k8s.getSecret`, the
   * consent-gated read, and only after it answers is the real manifest put
   * in the editor. The real text is held here in the meantime, in state, and
   * never rendered; and Apply is off while the editor shows placeholders,
   * because applying a redacted manifest would write the placeholders over
   * the values.
   */
  const isSecret = kind === "Secret";
  const [revealed, setRevealed] = useState(!isSecret);
  const [revealBusy, setRevealBusy] = useState(false);
  const [revealError, setRevealError] = useState("");

  const manifest = useResource(
    async () => {
      // A kind outside the built-in table has to be resolved to its group
      // first, or the read fails for every custom resource.
      let crd: DynamicGvk | undefined;
      if (!isBuiltInKind(kind)) {
        const resolved = await resolveCrdGvk(pinned, kind);
        if (resolved.error) throw new Error(resolved.error);
        crd = resolved.crd;
      }
      const out = await getManifest(pinned, kind, namespace, name, undefined, crd);
      if (out.error) throw new Error(out.error);
      const raw = out.yaml ?? "";
      if (!isSecret) return { raw, shown: raw };
      const redacted = redactSecretManifest(raw);
      if (redacted.error !== undefined) throw new Error(redacted.error);
      return { raw, shown: redacted.yaml ?? "" };
    },
    [pinned, kind, namespace, name],
    // An empty manifest is still a manifest that loaded.
    () => false,
  );
  const live = revealed ? manifest.data?.raw : manifest.data?.shown;

  async function reveal() {
    setRevealBusy(true);
    setRevealError("");
    const out = await getSecret(pinned, namespace ?? "", name);
    setRevealBusy(false);
    if (out.error) {
      setRevealError(describeError(out.error).detail);
      return;
    }
    setRevealed(true);
  }

  // `null` is "the reader has not typed": the editor shows the live manifest,
  // Apply is off, and a reload from the cluster is not a lost draft.
  const [draft, setDraft] = useState<string | null>(null);
  const yaml = draft ?? live ?? "";
  const dirty = draft !== null && draft !== live;
  const loadedRv = useMemo(() => parseResourceVersion(manifest.data?.raw ?? ""), [manifest.data]);

  const [confirming, setConfirming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleted, setDeleted] = useState(false);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [deleteError, setDeleteError] = useState("");
  const [showChanges, setShowChanges] = useState(false);

  const editor = useApply({
    pinned,
    live: context.name,
    verb: "apply",
    onApplied: () => {
      // The live object is now what was typed; the draft has nothing left to
      // say and the next load is the new baseline for "changed elsewhere".
      setDraft(null);
      setConfirming(false);
      // The panel closes with the draft it was comparing; left open, the
      // button read "Hide changes" over no panel at all.
      setShowChanges(false);
      manifest.reload();
    },
  });

  /**
   * A dry-run diff of the draft, debounced. Runs whether or not the panel is
   * open, because its side product — the live resourceVersion — is what says
   * the object changed under the reader, and that is worth knowing before
   * they open anything.
   */
  const [diff, setDiff] = useState<DiffDoc[] | null>(null);
  const [diffing, setDiffing] = useState(false);
  useEffect(() => {
    if (!dirty) {
      setDiff(null);
      return;
    }
    let active = true;
    setDiffing(true);
    const t = setTimeout(() => {
      void diffManifest(pinned, yaml).then((out) => {
        if (!active) return;
        setDiff(out.error ? [] : (out.documents ?? []));
        setDiffing(false);
      });
    }, 600);
    return () => {
      active = false;
      clearTimeout(t);
    };
  }, [pinned, yaml, dirty]);
  const stale = useMemo(
    () =>
      diff?.some(
        (d) => d.currentResourceVersion && loadedRv && d.currentResourceVersion !== loadedRv,
      ) ?? false,
    [diff, loadedRv],
  );

  async function remove() {
    if (editor.gate.refusal) {
      setDeleteError(editor.gate.refusal);
      return;
    }
    setDeleteBusy(true);
    setDeleteError("");
    const out = await deleteResource(pinned, kind, namespace, name);
    setDeleteBusy(false);
    if (out.error) {
      setDeleteError(describeError(out.error).detail);
      return;
    }
    setDeleting(false);
    setDeleted(true);
    notify.success(`Deleted ${kind} ${name}`);
  }

  const where = namespace === null ? kind : `${kind} · ${namespace}`;

  if (deleted) {
    return (
      <Screen title={`Edit ${name}`} eyebrow={`${where} · ${pinned}`} fill>
        <div className="grid h-full place-items-center">
          <EmptyState
            title={`${kind} ${name} was deleted`}
            hint="This tab has nothing left to edit. Close it, or reopen the resource from its list if it is recreated."
          />
        </div>
      </Screen>
    );
  }

  return (
    <Screen
      title={`Edit ${name}`}
      eyebrow={`${where} · ${pinned}`}
      actions={
        <>
          {stale && (
            <span
              className="text-xs text-warn"
              title="The live object has a newer resourceVersion than the one loaded here. Reload to see it; applying over it takes your version."
            >
              Changed elsewhere
            </span>
          )}
          {isSecret && !revealed && (
            <Button
              variant="secondary"
              onClick={() => void reveal()}
              disabled={revealBusy || manifest.status !== "ready"}
              title="Reads the Secret's values through the consent-gated read, then shows the real manifest"
            >
              {revealBusy ? "Revealing…" : "Reveal values"}
            </Button>
          )}
          <Button variant="ghost" onClick={() => setShowChanges((v) => !v)} disabled={!dirty}>
            {showChanges ? "Hide changes" : "Changes"}
          </Button>
          <Button
            variant="ghost"
            onClick={() => {
              setDraft(null);
              manifest.reload();
            }}
            title="Throw the draft away and load the manifest again from the cluster"
          >
            Reload
          </Button>
          <Button
            variant="danger"
            onClick={() => {
              setDeleteError("");
              editor.gate.reset();
              setDeleting(true);
            }}
          >
            Delete
          </Button>
          <Button
            variant="primary"
            disabled={
              !revealed || !dirty || !yaml.trim() || editor.busy || manifest.status !== "ready"
            }
            onClick={() => {
              editor.setError("");
              editor.gate.reset();
              setConfirming(true);
            }}
          >
            {editor.busy ? "Applying…" : "Apply"}
          </Button>
        </>
      }
      fill
    >
      <div className="flex h-full min-h-0 flex-col">
        <div className="flex flex-col gap-2 px-4 pt-3 empty:hidden">
          {manifest.status === "error" && (
            <FailureAlert title={`Could not load ${kind} ${name}`} error={manifest.error} />
          )}
          {isSecret && !revealed && manifest.status === "ready" && (
            <p className="text-sm text-muted" data-testid="secret-redacted">
              Values are redacted. Reveal them to edit this Secret; applying is off until then.
            </p>
          )}
          {revealError && <FailureAlert title={`Could not reveal ${name}`} error={revealError} />}
          <Conflicts
            conflicts={editor.conflicts}
            busy={editor.busy}
            onForce={() => void editor.apply(yaml, true)}
          />
          {editor.error && !confirming && (
            <FailureAlert title={`Could not apply ${name}`} error={editor.error} />
          )}
          {editor.applied && !dirty && (
            <p className="text-sm text-ok" role="status">
              Applied {editor.applied.kind} {editor.applied.name}.
            </p>
          )}
        </div>
        <div className="flex min-h-0 flex-1">
          <div className="relative min-h-0 min-w-0 flex-1 p-4">
            {manifest.status === "loading" ? (
              <p className="text-sm text-muted">Loading the manifest…</p>
            ) : manifest.status === "error" ? null : (
              <div className="absolute inset-4">
                <CodeEditor
                  value={yaml}
                  onChange={setDraft}
                  language="yaml"
                  fill
                  readOnly={!revealed}
                  ariaLabel={`${name} manifest`}
                  schemaValidate={validator(pinned)}
                />
              </div>
            )}
          </div>
          {showChanges && dirty && (
            <aside className="rule-l scroll w-[440px] shrink-0" aria-label="Changes">
              <Changes docs={diff} computing={diffing} />
            </aside>
          )}
        </div>
      </div>
      {confirming && (
        <ConfirmDialog
          title="Apply changes?"
          confirmLabel="Apply"
          busy={editor.busy}
          // Closed on any answer, not only success: a conflict and a failure
          // are shown on the page behind the dialog, and a modal left open
          // would hide the one thing the reader now has to act on.
          onConfirm={() => void editor.apply(yaml, false).finally(() => setConfirming(false))}
          onCancel={() => setConfirming(false)}
          message={
            <>
              <p>
                Server-side apply the edited <code>{kind}</code> <code>{name}</code> on{" "}
                <code>{pinned}</code>
                {namespace ? (
                  <>
                    {" "}
                    in <code>{namespace}</code>
                  </>
                ) : null}
                .
              </p>
              {stale && (
                <p className="mt-2 text-warn">
                  The live object changed since you opened it; applying takes your version of every
                  field you edited.
                </p>
              )}
              {editor.gate.alert}
              {editor.error && <p className="mt-2 text-sev">{editor.error}</p>}
            </>
          }
        />
      )}
      {deleting && (
        <ConfirmDialog
          title={`Delete ${kind} ${name}?`}
          confirmLabel="Delete"
          danger
          busy={deleteBusy}
          onConfirm={() => void remove()}
          onCancel={() => setDeleting(false)}
          message={
            <>
              <p>
                This deletes <code>{name}</code> from <code>{pinned}</code>
                {namespace ? (
                  <>
                    {" "}
                    in <code>{namespace}</code>
                  </>
                ) : null}
                . Anything it owns goes with it.
              </p>
              {editor.gate.alert}
              {deleteError && <p className="mt-2 text-sev">{deleteError}</p>}
            </>
          }
        />
      )}
    </Screen>
  );
}

/** Starter manifests for the kinds people most often create by hand. */
export const TEMPLATES: Record<string, (ns: string) => string> = {
  Blank: () => "",
  Deployment: (ns) => `apiVersion: apps/v1
kind: Deployment
metadata:
  name: my-app
  namespace: ${ns}
spec:
  replicas: 1
  selector:
    matchLabels:
      app: my-app
  template:
    metadata:
      labels:
        app: my-app
    spec:
      containers:
        - name: app
          image: nginx:1.27
          ports:
            - containerPort: 80
`,
  Service: (ns) => `apiVersion: v1
kind: Service
metadata:
  name: my-app
  namespace: ${ns}
spec:
  selector:
    app: my-app
  ports:
    - port: 80
      targetPort: 80
`,
  ConfigMap: (ns) => `apiVersion: v1
kind: ConfigMap
metadata:
  name: my-config
  namespace: ${ns}
data:
  key: value
`,
  Secret: (ns) => `apiVersion: v1
kind: Secret
metadata:
  name: my-secret
  namespace: ${ns}
type: Opaque
stringData:
  key: value
`,
  Ingress: (ns) => `apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: my-ingress
  namespace: ${ns}
spec:
  rules:
    - host: example.com
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: my-app
                port:
                  number: 80
`,
  Namespace: () => `apiVersion: v1
kind: Namespace
metadata:
  name: my-namespace
`,
};

export const TEMPLATE_ORDER = ["Deployment", "Service", "ConfigMap", "Secret", "Ingress", "Namespace", "Blank"];

function NewResource({ context }: { context: ClusterContext }) {
  const [pinned] = useState(context.name);
  const { namespaces } = useNamespaceOptions(pinned, getKubeconfigFiles());
  const options = namespaces ?? [];

  const [namespace, setNamespace] = useState("default");
  const [template, setTemplate] = useState(TEMPLATE_ORDER[0]);
  const [yaml, setYaml] = useState(() => TEMPLATES[TEMPLATE_ORDER[0]](namespace));
  // Once the cluster's namespaces are known, a `default` that does not exist
  // there is swapped for one that does — but only while the draft is still
  // the untouched template, so a reader who has typed keeps what they typed.
  useEffect(() => {
    if (options.length === 0 || options.includes(namespace)) return;
    const first = options[0];
    setNamespace(first);
    setYaml((current) => (current === TEMPLATES[template](namespace) ? TEMPLATES[template](first) : current));
  }, [options, namespace, template]);

  const editor = useApply({ pinned, live: context.name, verb: "create", onApplied: () => {} });
  const untouched = yaml === TEMPLATES[template](namespace);

  const pickTemplate = (next: string) => {
    setTemplate(next);
    setYaml(TEMPLATES[next](namespace));
  };
  const pickNamespace = (next: string) => {
    // The template's namespace follows the picker only while the draft is
    // still the template; a typed manifest is the reader's.
    setYaml((current) => (current === TEMPLATES[template](namespace) ? TEMPLATES[template](next) : current));
    setNamespace(next);
  };

  return (
    <Screen
      title="New resource"
      eyebrow={pinned}
      actions={
        <>
          <label className="flex items-center gap-2 text-xs text-muted">
            Template
            <Select
              value={template}
              onValueChange={pickTemplate}
              options={TEMPLATE_ORDER.map((t) => ({ value: t }))}
              className="min-w-36"
            />
          </label>
          <label className="flex items-center gap-2 text-xs text-muted">
            Namespace
            <Select
              value={namespace}
              onValueChange={pickNamespace}
              options={(options.length > 0 ? options : [namespace]).map((n) => ({ value: n }))}
              className="min-w-36"
            />
          </label>
          <Button
            variant="primary"
            disabled={!yaml.trim() || editor.busy}
            onClick={() => {
              editor.setError("");
              void editor.apply(yaml, false);
            }}
          >
            {editor.busy ? "Creating…" : "Create"}
          </Button>
        </>
      }
      fill
    >
      <div className="flex h-full min-h-0 flex-col">
        <div className="flex flex-col gap-2 px-4 pt-3 empty:hidden">
          {editor.gate.alert}
          <Conflicts
            conflicts={editor.conflicts}
            busy={editor.busy}
            onForce={() => void editor.apply(yaml, true)}
          />
          {editor.error && <FailureAlert title="Could not create the resource" error={editor.error} />}
          {editor.applied && (
            <p className="text-sm text-ok" role="status">
              Created {editor.applied.kind} {editor.applied.name}.{" "}
              {untouched ? "" : "The editor keeps your manifest, so a second one is a name away."}
            </p>
          )}
        </div>
        <div className="relative min-h-0 flex-1 p-4">
          <div className="absolute inset-4">
            <CodeEditor
              value={yaml}
              onChange={setYaml}
              language="yaml"
              fill
              ariaLabel="New resource manifest"
              schemaValidate={validator(pinned)}
            />
          </div>
        </div>
      </div>
    </Screen>
  );
}
