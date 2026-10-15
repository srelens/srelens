import { useEffect, useId, useMemo, useState } from "react";
import {
  parseMemberList,
  resolveNamespaceCollections,
  resolveResourceCollections,
  validateCollection,
  type Collection,
  type CollectionProblem,
  type CollectionType,
  type KindRef,
  type NamespaceResolution,
  type ResourceResolution,
} from "@srelens/core";
import { Button, Checkbox, Dialog, Field, TextInput } from "@srelens/ui-kit";
import { useCollections, saveCollection } from "../../lib/collections";
import { useListing } from "../../lib/collectionScope";

/** How long typing settles before the backend is asked to check or preview. */
const SETTLE_MS = 200;
/** How many matched namespaces the preview lists before it counts the rest. */
const PREVIEW_SHOWN = 40;

const TEXTAREA =
  "scroll w-full rounded-md border border-[var(--control-line)] bg-transparent px-2 py-1.5 font-mono text-[0.75rem] outline-none focus:border-[var(--accent)]";

/** What a draft says, as a collection; empty fields left out. */
function draftCollection(fields: Draft, type: CollectionType): Collection {
  const collection: Collection = { name: fields.name.trim(), type };
  if (fields.description.trim()) collection.description = fields.description.trim();
  if (type === "resource") collection.kinds = parseMemberList(fields.kinds);
  const members = parseMemberList(fields.members);
  if (members.length > 0) collection.members = members;
  const patterns = parseMemberList(fields.patterns);
  if (patterns.length > 0) collection.patterns = patterns;
  if (fields.labelSelector.trim()) collection.labelSelector = fields.labelSelector.trim();
  return collection;
}

interface Draft {
  name: string;
  description: string;
  kinds: string;
  members: string;
  patterns: string;
  labelSelector: string;
}

function draftOf(collection: Collection | undefined, members: readonly string[] = [], kinds: readonly string[] = []): Draft {
  return {
    name: collection?.name ?? "",
    description: collection?.description ?? "",
    kinds: (collection?.kinds ?? kinds).join(", "),
    members: (collection?.members ?? members).join("\n"),
    patterns: (collection?.patterns ?? []).join("\n"),
    labelSelector: collection?.labelSelector ?? "",
  };
}

/** A field's problems as one line each, a list entry's prefixed with what it was. */
function problemText(problems: CollectionProblem[], field: string): string | undefined {
  const mine = problems.filter((p) => p.field === field);
  return mine.length > 0 ? mine.map((p) => p.message).join(" ") : undefined;
}

export interface CollectionDialogProps {
  /** The collection being edited; absent for a new one. */
  editing?: Collection;
  /** For a new one: which type. The edited one's own type otherwise. */
  type?: CollectionType;
  /** For a new one, the members it starts with — Save as collection… */
  members?: readonly string[];
  /** For a new resource collection, the kinds it starts with: the list's own. */
  kinds?: readonly string[];
  /**
   * For a resource collection, the kind and namespace scope its preview lists
   * — the list Save as collection… was chosen from. Without it, the first
   * kind the draft names, across every namespace.
   */
  previewKind?: KindRef;
  previewNamespaces?: string[];
  /** The cluster the preview resolves against, by `stableId`, and its name. */
  clusterId?: string;
  clusterName?: string;
  onClose: () => void;
  /** After a save, with what was saved. */
  onSaved?: (collection: Collection) => void;
}

/**
 * Create or edit a collection: a namespace collection (#881), or a resource
 * collection (#885), which also names the kinds whose lists it filters. Either
 * is a name and any of explicit members, name patterns and a label selector.
 *
 * Checked by the backend as it is typed, so the rule that refuses a save is
 * the rule the matching applies, and previewed against the cluster in focus,
 * so a typo in a member or a pattern that matches nothing shows before the
 * collection is used: each member is marked found or missing.
 *
 * Members and patterns take one per line, or commas — a list pasted from a
 * release ticket is split, trimmed and kept once each.
 */
export function CollectionDialog({
  editing,
  type: newType = "namespace",
  members,
  kinds,
  previewKind,
  previewNamespaces,
  clusterId,
  clusterName,
  onClose,
  onSaved,
}: CollectionDialogProps) {
  const type = editing?.type ?? newType;
  const resource = type === "resource";
  const all = useCollections();
  const [draft, setDraft] = useState<Draft>(() => draftOf(editing, members, kinds));
  const [problems, setProblems] = useState<CollectionProblem[] | null>(null);
  // Set from each change until the backend has checked it. The problems shown
  // meanwhile are the last ones, so nothing flickers while typing, but a save
  // waits for the draft's own.
  const [checking, setChecking] = useState(true);
  const [checkError, setCheckError] = useState<string | null>(null);
  const [touchedName, setTouchedName] = useState(editing !== undefined);
  const collection = useMemo(() => draftCollection(draft, type), [draft, type]);
  const taken = useMemo(
    () => all.filter((c) => c.type === type && c.name !== editing?.name).map((c) => c.name),
    [all, type, editing?.name],
  );
  const ids = { members: useId(), patterns: useId(), selector: useId() };

  useEffect(() => {
    let current = true;
    setChecking(true);
    const timer = setTimeout(() => {
      validateCollection(collection, taken).then(
        (found) => {
          if (!current) return;
          setCheckError(null);
          setProblems(found);
          setChecking(false);
        },
        (error: unknown) => {
          if (!current) return;
          setCheckError(error instanceof Error ? error.message : String(error));
          setProblems([]);
          setChecking(false);
        },
      );
    }, SETTLE_MS);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [collection, taken]);

  const set = (field: keyof Draft) => (value: string) => setDraft((d) => ({ ...d, [field]: value }));
  // An empty name is a problem the reader has not made yet: it is not shown
  // until they have typed one or tried to save.
  const shown = (problems ?? []).filter((p) => p.field !== "name" || touchedName);
  const canSave = !checking && problems !== null && problems.length === 0 && checkError === null;
  // The preview needs the rules to be sound, not the whole collection: a
  // pattern typed before the name is worth seeing at once.
  const previewable =
    problems !== null && checkError === null && problems.every((p) => p.field === "name" || p.field === "description");

  function save() {
    setTouchedName(true);
    if (!canSave) return;
    const saved = { ...collection };
    saveCollection(saved, editing?.name);
    onSaved?.(saved);
    onClose();
  }

  return (
    <Dialog
      title={editing ? `Edit ${editing.name}` : resource ? "New resource collection" : "New namespace collection"}
      onClose={onClose}
      maxWidth={560}
      footer={
        <>
          <Button variant="secondary" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" size="sm" onClick={save} disabled={checking || (!canSave && touchedName)}>
            {editing ? "Save" : "Save collection"}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 p-3 text-[0.8125rem]">
        <Field label="Name" error={problemText(shown, "name")}>
          <TextInput
            value={draft.name}
            onValueChange={(v) => {
              setTouchedName(true);
              set("name")(v);
            }}
            onEnter={save}
            placeholder="prod"
           
          />
        </Field>
        <Field label="Description (optional)" error={problemText(shown, "description")}>
          <TextInput value={draft.description} onValueChange={set("description")} />
        </Field>
        {resource && (
          <Field
            label="Kinds"
            hint="The lists it filters: Deployment, Service, or group/Kind such as traefik.io/IngressRoute for a custom kind."
            error={problemText(shown, "kinds")}
          >
            <TextInput value={draft.kinds} onValueChange={set("kinds")} placeholder="apps/Deployment, apps/StatefulSet" />
          </Field>
        )}
        <Field
          label="Members"
          hint={
            resource
              ? "Always included. A name matches in every namespace in view; namespace/name pins one. One per line, or paste a list."
              : "Always included. One per line, or paste a comma-separated list."
          }
          error={problemText(shown, "members")}
        >
          <textarea
            id={ids.members}
            aria-label="Members"
            value={draft.members}
            onChange={(e) => set("members")(e.target.value)}
            rows={3}
            className={TEXTAREA}
            placeholder={resource ? "payment-service\norder-service" : "payments\ncheckout"}
          />
        </Field>
        <Field
          label="Patterns"
          hint="One per line; * matches any characters, e.g. prod-*, *-canary."
          error={problemText(shown, "patterns")}
        >
          <textarea
            id={ids.patterns}
            aria-label="Patterns"
            value={draft.patterns}
            onChange={(e) => set("patterns")(e.target.value)}
            rows={2}
            className={TEXTAREA}
            placeholder="prod-*"
          />
        </Field>
        <Field
          label="Label selector"
          hint={`As kubectl get -l takes it. With patterns, ${resource ? "an object" : "a namespace"} must match both.`}
          error={problemText(shown, "labelSelector")}
        >
          <TextInput
            value={draft.labelSelector}
            onValueChange={set("labelSelector")}
            placeholder="env=prod"
          />
        </Field>
        {checkError !== null && (
          <p role="alert" style={{ color: "var(--sev)" }}>
            The collection could not be checked: {checkError}
          </p>
        )}
        {resource ? (
          <ResourcePreview
            collection={collection}
            valid={previewable}
            clusterName={clusterName}
            kind={previewKind ?? firstKind(collection.kinds)}
            namespaces={previewNamespaces ?? []}
            onMembers={(next) => set("members")(next.join("\n"))}
          />
        ) : (
          <Preview
            collection={collection}
            valid={previewable}
            clusterId={clusterId}
            clusterName={clusterName}
            onMembers={(next) => set("members")(next.join("\n"))}
          />
        )}
      </div>
    </Dialog>
  );
}

/**
 * What the draft matches on the cluster in focus, one row per namespace, and
 * a box on each row to keep it as a member.
 *
 * A pattern finds namespaces; ticking one pins it, so it stays in the
 * collection whatever the pattern later says — and a namespace a member
 * names but this cluster does not have is listed too, marked not found, so a
 * typo shows before the collection is used.
 */
function Preview({
  collection,
  valid,
  clusterId,
  clusterName,
  onMembers,
}: {
  collection: Collection;
  /** Whether the rules are sound enough to resolve; the name is not needed. */
  valid: boolean;
  clusterId?: string;
  clusterName?: string;
  onMembers: (members: string[]) => void;
}) {
  const listing = useListing(clusterId);
  const [result, setResult] = useState<NamespaceResolution | null>(null);
  const [error, setError] = useState<string | null>(null);
  const empty = !collection.members && !collection.patterns && !collection.labelSelector;
  const members = collection.members ?? [];

  useEffect(() => {
    let current = true;
    setError(null);
    if (!listing || !valid || empty) {
      setResult(null);
      return;
    }
    const timer = setTimeout(() => {
      // Resolved under a stand-in name when it has none yet: the backend
      // refuses a nameless collection, and the name decides nothing here.
      resolveNamespaceCollections([{ ...collection, name: collection.name || "preview" }], listing.items, listing.complete).then(
        (r) => current && setResult(r),
        (e: unknown) => current && setError(e instanceof Error ? e.message : String(e)),
      );
    }, SETTLE_MS);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [collection, valid, empty, listing]);

  const toggle = (name: string, on: boolean) =>
    onMembers(on ? [...members.filter((m) => m !== name), name] : members.filter((m) => m !== name));

  const where = clusterName ?? "this cluster";
  let body;
  if (!clusterId || !listing) {
    body = <p className="text-muted">Open a namespace list on a cluster to preview what this matches there.</p>;
  } else if (empty) {
    body = <p className="text-muted">Add members, a pattern or a label selector to see what it matches on {where}.</p>;
  } else if (error !== null) {
    body = <p style={{ color: "var(--sev)" }}>The preview could not be worked out: {error}</p>;
  } else if (!valid) {
    body = <p className="text-muted">Correct the members, patterns or label selector above to see what they match.</p>;
  } else if (result === null) {
    body = <p className="text-muted">Working out what this matches on {where}…</p>;
  } else {
    const match = result.collections[0];
    const matched = match?.matched ?? [];
    const status = new Map((match?.members ?? []).map((m) => [m.name, m.status]));
    // Every namespace it matched, then each member this cluster lacks.
    const rows = [...matched, ...members.filter((m) => !matched.includes(m))];
    const shown = rows.slice(0, PREVIEW_SHOWN);
    body = (
      <>
        <p>
          {matched.length === 0
            ? `Matches no namespace on ${where}.`
            : `Matches ${matched.length} ${matched.length === 1 ? "namespace" : "namespaces"} on ${where}.`}
          {result.partial && " These credentials cannot list every namespace, so this may be partial."}
        </p>
        {rows.length > 0 && (
          <>
            <p className="mt-1 text-[0.75rem] text-muted">Tick a namespace to keep it as a member.</p>
            <ul className="scroll mt-1 flex max-h-48 flex-col" aria-label="Matched namespaces">
              {shown.map((name) => {
                const isMember = members.includes(name);
                const state = isMember ? status.get(name) : undefined;
                return (
                  <li key={name} className="flex items-center gap-2 py-0.5 text-[0.75rem]" data-namespace={name}>
                    <Checkbox
                      checked={isMember}
                      onChange={(on) => toggle(name, on)}
                      label={<span className="sr-only">Keep {name} as a member</span>}
                    />
                    <span className="min-w-0 flex-1 truncate font-mono">{name}</span>
                    {/* A word, and the colour on the word: never the colour
                        alone. Only what needs looking at is toned. */}
                    {state === "missing" ? (
                      <span className="shrink-0" style={{ color: "var(--warn)" }}>not found</span>
                    ) : state === "unknown" ? (
                      <span className="shrink-0 text-muted">cannot check</span>
                    ) : isMember ? (
                      <span className="shrink-0 text-muted">member</span>
                    ) : null}
                  </li>
                );
              })}
            </ul>
            {rows.length > PREVIEW_SHOWN && (
              <p className="mt-1 text-[0.75rem] text-muted">and {rows.length - PREVIEW_SHOWN} more</p>
            )}
          </>
        )}
      </>
    );
  }

  return (
    // Ruled off from the form, not boxed: a card inside the dialog's card is
    // one frame too many.
    <section aria-label="Preview" className="rule-t -mx-3 px-3 pt-3">
      <p className="eyebrow mb-1">Preview</p>
      {body}
    </section>
  );
}

/** A collection's first kind as a view would name it: `apps/Deployment` → Deployment in apps. */
function firstKind(kinds: readonly string[] | undefined): KindRef | undefined {
  const first = kinds?.[0];
  if (!first) return undefined;
  const slash = first.lastIndexOf("/");
  // A built-in kind is resolved by name; its group is the backend's to know.
  return slash < 0 ? { kind: first } : { kind: first.slice(slash + 1) };
}

/**
 * What a resource collection matches of one kind on the cluster in focus,
 * one row per name with the namespaces it is in, and a box on each to keep
 * that name as a member — which then matches it in every namespace in view.
 */
function ResourcePreview({
  collection,
  valid,
  clusterName,
  kind,
  namespaces,
  onMembers,
}: {
  collection: Collection;
  valid: boolean;
  clusterName?: string;
  kind: KindRef | undefined;
  namespaces: string[];
  onMembers: (members: string[]) => void;
}) {
  const [result, setResult] = useState<ResourceResolution | null>(null);
  const [error, setError] = useState<string | null>(null);
  const empty = !collection.members && !collection.patterns && !collection.labelSelector;
  const members = collection.members ?? [];
  const scopeKey = namespaces.join(",");

  useEffect(() => {
    let current = true;
    setError(null);
    if (!clusterName || !kind || !valid || empty) {
      setResult(null);
      return;
    }
    const timer = setTimeout(() => {
      resolveResourceCollections(clusterName, kind, namespaces, [{ ...collection, name: collection.name || "preview" }]).then(
        (r) => current && setResult(r),
        (e: unknown) => current && setError(e instanceof Error ? e.message : String(e)),
      );
    }, SETTLE_MS);
    return () => {
      current = false;
      clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [collection, valid, empty, clusterName, kind?.kind, kind?.group, kind?.plural, scopeKey]);

  const toggle = (name: string, on: boolean) =>
    onMembers(on ? [...members.filter((m) => m !== name), name] : members.filter((m) => m !== name));

  const what = kind ? kind.kind : "objects";
  const where = clusterName ?? "this cluster";
  let body;
  if (!clusterName) {
    body = <p className="text-muted">Pick a cluster to preview what this matches there.</p>;
  } else if (!kind) {
    body = <p className="text-muted">Add a kind to see what this matches on {where}.</p>;
  } else if (empty) {
    body = <p className="text-muted">Add members, a pattern or a label selector to see which {what} objects it matches on {where}.</p>;
  } else if (error !== null) {
    body = <p style={{ color: "var(--sev)" }}>The preview could not be worked out: {error}</p>;
  } else if (!valid) {
    body = <p className="text-muted">Correct the fields above to see what they match.</p>;
  } else if (result === null) {
    body = <p className="text-muted">Working out which {what} objects this matches on {where}…</p>;
  } else {
    const match = result.collections[0];
    const status = new Map((match?.members ?? []).map((m) => [m.name, m.status]));
    // Each name once, with the namespaces it was found in.
    const byName = new Map<string, string[]>();
    for (const key of result.keys) {
      const slash = key.indexOf("/");
      const namespace = key.slice(0, slash);
      const name = key.slice(slash + 1);
      byName.set(name, [...(byName.get(name) ?? []), namespace].filter(Boolean));
    }
    const plainMembers = members.filter((m) => !m.includes("/"));
    const rows = [...byName.keys(), ...plainMembers.filter((m) => !byName.has(m))];
    const shown = rows.slice(0, PREVIEW_SHOWN);
    body = (
      <>
        <p>
          {match?.problem
            ? match.problem
            : result.keys.length === 0
              ? `Matches no ${what} on ${where}${namespaces.length > 0 ? " in the namespaces in view" : ""}.`
              : `Matches ${result.keys.length} ${what} on ${where}${namespaces.length > 0 ? " in the namespaces in view" : ""}.`}
          {result.partial && " Some namespaces could not be listed, so this may be partial."}
        </p>
        {rows.length > 0 && (
          <>
            <p className="mt-1 text-[0.75rem] text-muted">Tick a name to keep it as a member, in every namespace in view.</p>
            <ul className="scroll mt-1 flex max-h-48 flex-col" aria-label="Matched names">
              {shown.map((name) => {
                const isMember = members.includes(name);
                const state = isMember ? status.get(name) : undefined;
                const where = byName.get(name) ?? [];
                return (
                  <li key={name} className="flex items-center gap-2 py-0.5 text-[0.75rem]" data-name={name}>
                    <Checkbox
                      checked={isMember}
                      onChange={(on) => toggle(name, on)}
                      label={<span className="sr-only">Keep {name} as a member</span>}
                    />
                    <span className="min-w-0 flex-1 truncate font-mono">{name}</span>
                    {where.length > 0 && <span className="min-w-0 shrink truncate text-muted">{where.join(", ")}</span>}
                    {state === "missing" ? (
                      <span className="shrink-0" style={{ color: "var(--warn)" }}>not found</span>
                    ) : state === "unknown" ? (
                      <span className="shrink-0 text-muted">cannot check</span>
                    ) : isMember ? (
                      <span className="shrink-0 text-muted">member</span>
                    ) : null}
                  </li>
                );
              })}
            </ul>
            {rows.length > PREVIEW_SHOWN && (
              <p className="mt-1 text-[0.75rem] text-muted">and {rows.length - PREVIEW_SHOWN} more</p>
            )}
          </>
        )}
      </>
    );
  }

  return (
    <section aria-label="Preview" className="rule-t -mx-3 px-3 pt-3">
      <p className="eyebrow mb-1">Preview{kind ? ` · ${kind.kind}` : ""}</p>
      {body}
    </section>
  );
}
