import { useState } from "react";
import type { Collection } from "@srelens/core";
import { Button, Checkbox, ConfirmDialog, EmptyState } from "@srelens/ui-kit";
import { deleteCollections, useCollections } from "../../lib/collections";
import { useActiveContext } from "../../lib/clusters";
import { CollectionDialog } from "../collections/CollectionEditor";

/** What a collection's rules are, in a line: members, patterns, selector. */
function rules(c: Collection): string {
  const parts: string[] = [];
  const members = c.members ?? [];
  if (members.length > 0) {
    parts.push(members.length <= 3 ? members.join(", ") : `${members.slice(0, 3).join(", ")} and ${members.length - 3} more`);
  }
  if ((c.patterns ?? []).length > 0) parts.push((c.patterns ?? []).join(", "));
  if (c.labelSelector) parts.push(c.labelSelector);
  return parts.join(" · ") || "No members or rules";
}

/**
 * Settings → Collections (#881): the namespace collections every namespace
 * picker offers, created, edited and deleted in one place — several at once,
 * for cleaning up after a release.
 *
 * The editor previews each collection against the cluster in focus, so what
 * it matches there is visible before it is used.
 */
export function CollectionsPane() {
  const all = useCollections();
  const context = useActiveContext();
  const collections = all.filter((c) => c.type === "namespace").sort((a, b) => a.name.localeCompare(b.name));
  const [editing, setEditing] = useState<Collection | "new" | null>(null);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [deleting, setDeleting] = useState<string[] | null>(null);
  // Only names that still exist count as picked: one deleted elsewhere, or
  // renamed in the editor, is no longer a row to act on.
  const chosen = collections.filter((c) => picked.has(c.name)).map((c) => c.name);

  const toggle = (name: string, on: boolean) =>
    setPicked((current) => {
      const next = new Set(current);
      if (on) next.add(name);
      else next.delete(name);
      return next;
    });

  return (
    <div className="min-w-0 text-[0.8125rem]">
      <section className="border-b border-rule px-4 py-3" aria-label="About collections">
        <h2 className="font-semibold text-ink">Namespace collections</h2>
        <p className="mt-1 max-w-[70ch] text-muted">
          Saved groups of namespaces — by name pattern (prod-*), by label (env=prod), or picked by hand — that every
          namespace picker offers above the namespaces. Choosing one scopes the list to whatever it matches on the
          cluster you are looking at, so one collection works on every cluster. Members are always included; with
          both patterns and a label selector, a namespace has to match both.
        </p>
      </section>

      <section className="flex flex-wrap items-center gap-2 border-b border-rule px-4 py-2" aria-label="Collection actions">
        <Button variant="primary" onClick={() => setEditing("new")}>
          New collection…
        </Button>
        {chosen.length > 0 && (
          <Button variant="secondary" onClick={() => setDeleting(chosen)}>
            Delete {chosen.length} selected
          </Button>
        )}
        <span className="ml-auto text-muted">
          {collections.length} {collections.length === 1 ? "collection" : "collections"}
        </span>
      </section>

      {collections.length === 0 ? (
        <EmptyState
          title="No namespace collections yet"
          hint="Create one here, or pick namespaces in any list and choose Save as collection… in the namespace picker."
          className="py-10"
        />
      ) : (
        <ul aria-label="Namespace collections">
          {collections.map((c) => (
            <li key={c.name} className="flex items-start gap-3 border-b border-rule px-4 py-2.5" data-collection={c.name}>
              <Checkbox
                checked={picked.has(c.name)}
                onChange={(on) => toggle(c.name, on)}
                label={<span className="sr-only">Select {c.name}</span>}
              />
              <div className="min-w-0 flex-1">
                <p className="font-mono font-semibold text-ink">{c.name}</p>
                {c.description && <p className="mt-0.5 text-ink-soft">{c.description}</p>}
                <p className="mt-0.5 overflow-x-auto whitespace-nowrap font-mono text-[0.75rem] text-muted" data-slot="collection-rules">
                  {rules(c)}
                </p>
              </div>
              <div className="flex shrink-0 gap-1.5">
                <Button variant="secondary" size="xs" onClick={() => setEditing(c)}>
                  Edit
                </Button>
                <Button variant="secondary" size="xs" onClick={() => setDeleting([c.name])}>
                  Delete
                </Button>
              </div>
            </li>
          ))}
        </ul>
      )}

      {editing !== null && (
        <CollectionDialog
          editing={editing === "new" ? undefined : editing}
          clusterId={context?.stableId}
          clusterName={context?.name}
          onClose={() => setEditing(null)}
        />
      )}
      {deleting !== null && (
        <ConfirmDialog
          title={deleting.length === 1 ? `Delete ${deleting[0]}?` : `Delete ${deleting.length} collections?`}
          message={
            deleting.length === 1
              ? "Lists scoped to it go back to their default namespace."
              : `${deleting.join(", ")}. Lists scoped to them go back to their default namespace.`
          }
          confirmLabel="Delete"
          danger
          onConfirm={() => {
            deleteCollections("namespace", deleting);
            setPicked(new Set());
            setDeleting(null);
          }}
          onCancel={() => setDeleting(null)}
        />
      )}
    </div>
  );
}
