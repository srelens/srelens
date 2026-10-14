import { useEffect, useId, useMemo, useState } from "react";
import {
  parseMemberList,
  resolveNamespaceCollections,
  validateCollection,
  type Collection,
  type CollectionProblem,
  type NamespaceResolution,
} from "@srelens/core";
import { Button, Dialog, Field, TextInput } from "@srelens/ui-kit";
import { useCollections, saveCollection } from "../../lib/collections";
import { useListing } from "../../lib/collectionScope";

/** How long typing settles before the backend is asked to check or preview. */
const SETTLE_MS = 200;
/** How many matched namespaces the preview lists before it counts the rest. */
const PREVIEW_SHOWN = 40;

const TEXTAREA =
  "scroll w-full rounded-md border border-[var(--control-line)] bg-transparent px-2 py-1.5 font-mono text-[0.75rem] outline-none focus:border-[var(--accent)]";

/** What a draft says, as a collection; empty fields left out. */
function draftCollection(fields: Draft): Collection {
  const collection: Collection = { name: fields.name.trim(), type: "namespace" };
  if (fields.description.trim()) collection.description = fields.description.trim();
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
  members: string;
  patterns: string;
  labelSelector: string;
}

function draftOf(collection: Collection | undefined, members: readonly string[] = []): Draft {
  return {
    name: collection?.name ?? "",
    description: collection?.description ?? "",
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
  /** For a new one, the namespaces it starts with — Save as collection… */
  members?: readonly string[];
  /** The cluster the preview resolves against, by `stableId`, and its name. */
  clusterId?: string;
  clusterName?: string;
  onClose: () => void;
  /** After a save, with what was saved. */
  onSaved?: (collection: Collection) => void;
}

/**
 * Create or edit a namespace collection (#881): a name, and any of explicit
 * members, name patterns and a label selector.
 *
 * Checked by the backend as it is typed, so the rule that refuses a save is
 * the rule the matching applies, and previewed against the cluster in focus,
 * so a typo in a member or a pattern that matches nothing shows before the
 * collection is used: each member is marked found or missing.
 *
 * Members and patterns take one per line, or commas — a list pasted from a
 * release ticket is split, trimmed and kept once each.
 */
export function CollectionDialog({ editing, members, clusterId, clusterName, onClose, onSaved }: CollectionDialogProps) {
  const all = useCollections();
  const [draft, setDraft] = useState<Draft>(() => draftOf(editing, members));
  const [problems, setProblems] = useState<CollectionProblem[] | null>(null);
  const [checkError, setCheckError] = useState<string | null>(null);
  const [touchedName, setTouchedName] = useState(editing !== undefined);
  const collection = useMemo(() => draftCollection(draft), [draft]);
  const taken = useMemo(
    () => all.filter((c) => c.type === "namespace" && c.name !== editing?.name).map((c) => c.name),
    [all, editing?.name],
  );
  const ids = { members: useId(), patterns: useId(), selector: useId() };

  useEffect(() => {
    let current = true;
    setProblems(null);
    const timer = setTimeout(() => {
      validateCollection(collection, taken).then(
        (found) => {
          if (!current) return;
          setCheckError(null);
          setProblems(found);
        },
        (error: unknown) => {
          if (!current) return;
          setCheckError(error instanceof Error ? error.message : String(error));
          setProblems([]);
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
  const canSave = problems !== null && problems.length === 0 && checkError === null;

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
      title={editing ? `Edit ${editing.name}` : "New namespace collection"}
      onClose={onClose}
      maxWidth={560}
      footer={
        <>
          <Button variant="secondary" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" size="sm" onClick={save} disabled={problems === null || (!canSave && touchedName)}>
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
        <Field
          label="Members"
          hint="Always included. One per line, or paste a comma-separated list."
          error={problemText(shown, "members")}
        >
          <textarea
            id={ids.members}
            aria-label="Members"
            value={draft.members}
            onChange={(e) => set("members")(e.target.value)}
            rows={3}
            className={TEXTAREA}
            placeholder={"payments\ncheckout"}
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
          hint="As kubectl get -l takes it. With patterns, a namespace must match both."
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
        <Preview collection={collection} valid={canSave} clusterId={clusterId} clusterName={clusterName} />
      </div>
    </Dialog>
  );
}

/** What the draft matches on the cluster in focus, member by member. */
function Preview({
  collection,
  valid,
  clusterId,
  clusterName,
}: {
  collection: Collection;
  valid: boolean;
  clusterId?: string;
  clusterName?: string;
}) {
  const listing = useListing(clusterId);
  const [result, setResult] = useState<NamespaceResolution | null>(null);
  const [error, setError] = useState<string | null>(null);
  const empty = !collection.members && !collection.patterns && !collection.labelSelector;

  useEffect(() => {
    let current = true;
    setResult(null);
    setError(null);
    if (!listing || !valid || empty) return;
    const timer = setTimeout(() => {
      resolveNamespaceCollections([collection], listing.items, listing.complete).then(
        (r) => current && setResult(r),
        (e: unknown) => current && setError(e instanceof Error ? e.message : String(e)),
      );
    }, SETTLE_MS);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [collection, valid, empty, listing]);

  const where = clusterName ?? "this cluster";
  let body;
  if (!clusterId || !listing) {
    body = <p className="text-muted">Open a namespace list on a cluster to preview what this matches there.</p>;
  } else if (empty) {
    body = <p className="text-muted">Add members, a pattern or a label selector to see what it matches on {where}.</p>;
  } else if (error !== null) {
    body = <p style={{ color: "var(--sev)" }}>The preview could not be worked out: {error}</p>;
  } else if (!valid || result === null) {
    body = <p className="text-muted">…</p>;
  } else {
    const match = result.collections[0];
    const matched = match?.matched ?? [];
    body = (
      <>
        <p>
          {matched.length === 0
            ? `Matches no namespace on ${where}.`
            : `Matches ${matched.length} ${matched.length === 1 ? "namespace" : "namespaces"} on ${where}.`}
          {result.partial && " These credentials cannot list every namespace, so this may be partial."}
        </p>
        {matched.length > 0 && (
          <p className="mt-1 text-[0.75rem] text-muted" data-slot="preview-matched">
            {matched.slice(0, PREVIEW_SHOWN).join(", ")}
            {matched.length > PREVIEW_SHOWN && ` and ${matched.length - PREVIEW_SHOWN} more`}
          </p>
        )}
        {match && match.members.length > 0 && (
          // A word beside each member, and the colour on that word: never the
          // colour alone. Only the ones that need looking at are toned.
          <ul className="mt-1.5 flex flex-col gap-0.5" aria-label="Members">
            {match.members.map((m) => (
              <li key={m.name} className="flex items-baseline gap-2 text-[0.75rem]" data-member={m.name}>
                <span className="min-w-0 truncate font-mono">{m.name}</span>
                <span
                  className="shrink-0"
                  style={{ color: m.status === "missing" ? "var(--warn)" : "var(--ink-muted)" }}
                >
                  {m.status === "found" ? "found" : m.status === "missing" ? "not found" : "unknown"}
                </span>
              </li>
            ))}
          </ul>
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
