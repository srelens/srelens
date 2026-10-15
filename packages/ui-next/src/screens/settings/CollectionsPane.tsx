import { useEffect, useState } from "react";
import type { Collection, CollectionType } from "@srelens/core";
import { Button, Checkbox, ConfirmDialog, EmptyState, Tabs } from "@srelens/ui-kit";
import { deleteCollections, refreshCollections, useCollections } from "../../lib/collections";
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
 * Settings → Collections: on one tab the namespace collections every
 * namespace picker offers (#881), on the other the resource collections the
 * lists of their kinds offer (#885) — created, edited and deleted in one
 * place, several at once, for cleaning up after a release.
 *
 * The editor previews each collection against the cluster in focus, so what
 * it matches there is visible before it is used.
 */
export function CollectionsPane() {
  const all = useCollections();
  const context = useActiveContext();
  // Another window may have saved or deleted one since this one started.
  useEffect(() => {
    void refreshCollections();
  }, []);
  const [type, setType] = useState<CollectionType>("namespace");
  const resource = type === "resource";
  const collections = all.filter((c) => c.type === type).sort((a, b) => a.name.localeCompare(b.name));
  const count = (t: CollectionType) => all.filter((c) => c.type === t).length;
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
        <h2 className="font-semibold text-ink">Collections</h2>
        <p className="mt-1 max-w-[70ch] text-muted">
          Saved groups — by name pattern (prod-*), by label (env=prod), or picked by hand. A namespace collection is
          offered by every namespace picker and scopes a list to the namespaces it matches; a resource collection is
          offered by the lists of its kinds and narrows them to what it matches, such as the services in next
          Tuesday&apos;s release. Either matches on the cluster you are looking at, so one collection works on every
          cluster. Members are always included; with both patterns and a label selector, both have to match.
        </p>
        <div className="mt-2">
          <Tabs
            variant="underline"
            label="Collection type"
            active={type}
            onChange={(id) => {
              setType(id === "resource" ? "resource" : "namespace");
              setPicked(new Set());
            }}
            tabs={[
              { id: "namespace", label: `Namespaces (${count("namespace")})` },
              { id: "resource", label: `Resources (${count("resource")})` },
            ]}
          />
        </div>
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
          title={resource ? "No resource collections yet" : "No namespace collections yet"}
          hint={
            resource
              ? "Create one here, or tick rows in a list and choose Save as collection… in its Collections filter."
              : "Create one here, or pick namespaces in any list and choose Save as collection… in the namespace picker."
          }
          className="py-10"
        />
      ) : (
        <ul aria-label={resource ? "Resource collections" : "Namespace collections"}>
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
                {resource && (
                  <p className="mt-0.5 text-[0.75rem] text-muted" data-slot="collection-kinds">
                    {(c.kinds ?? []).join(", ")}
                  </p>
                )}
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
          type={type}
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
              ? resource
                ? "Lists narrowed to it show every row again."
                : "Lists scoped to it go back to their default namespace."
              : `${deleting.join(", ")}. ${resource ? "Lists narrowed to them show every row again." : "Lists scoped to them go back to their default namespace."}`
          }
          confirmLabel="Delete"
          danger
          onConfirm={() => {
            deleteCollections(type, deleting);
            setPicked(new Set());
            setDeleting(null);
          }}
          onCancel={() => setDeleting(null)}
        />
      )}
    </div>
  );
}
