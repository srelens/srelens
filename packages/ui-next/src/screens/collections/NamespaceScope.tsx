import { useEffect, useState } from "react";
import { listNamespaces, selectsNothing, type ClusterContext, type Collection } from "@srelens/core";

/** How often a collection-scoped view reads the namespaces again. */
export const RELIST_MS = 30_000;
import type { NamespaceOptions } from "@srelens/core/react";
import { Alert, Button, MultiSelect, PickerRow, Spinner } from "@srelens/ui-kit";
import { byRecentUse, useCollections } from "../../lib/collections";
import { noteNamespaceListing, useCollectionScope, useListing } from "../../lib/collectionScope";
import { openSettings } from "../../lib/settingsRequest";
import {
  useNamespaceCollections,
  useNamespaces,
  useSetNamespaceCollections,
  useSetNamespaces,
} from "../../lib/workspace";
import { CollectionDialog } from "./CollectionEditor";

/**
 * The namespace picker, with namespace collections on a tab beside the
 * namespaces (#881).
 *
 * Two ways to say where a list looks, chosen in one place: pick namespaces,
 * or pick collections — saved rules such as `prod-*` or `team=platform` — and
 * the list follows whatever they resolve to on this cluster. Choosing either
 * replaces the other. Several collections are their union.
 *
 * It also reports the namespace listing it shows, labels included, which is
 * what a collection is resolved against: a view that can see a namespace is
 * a view that can scope to it.
 */
export function NamespaceScopePicker({
  context,
  options,
  onChange,
}: {
  context: ClusterContext;
  options: NamespaceOptions;
  /** After the scope changes, either way — for a screen holding something tied to the old one. */
  onChange?: () => void;
}) {
  const clusterId = context.stableId;
  const { namespaces } = options;
  // A listing without labels (an older caller, a test's stand-in) still
  // scopes by name; it just offers nothing to resolve a collection against.
  const items = options.items ?? null;
  const complete = options.complete ?? false;
  const collections = useCollections();
  const chosen = useNamespaceCollections(clusterId) ?? [];
  const selection = useNamespaces(clusterId);
  const setPicked = useSetNamespaces();
  const setChosen = useSetNamespaceCollections();
  const setNamespaces = (id: string, next: string[]) => {
    setPicked(id, next);
    onChange?.();
  };
  const setCollections = (id: string, next: string[]) => {
    setChosen(id, next);
    onChange?.();
  };
  const scope = useCollectionScope(clusterId, chosen);
  // The listing a collection is resolved against, read again while one is in
  // force: the rows offer the same namespaces, so one created since is a row
  // that can be seen and unticked, not a tick on nothing.
  const listing = useListing(clusterId);
  const [saving, setSaving] = useState<readonly string[] | null>(null);

  useEffect(() => {
    if (items !== null) noteNamespaceListing(clusterId, items, complete);
  }, [clusterId, items, complete]);

  // The view's listing is read once, when it opens. A collection is a rule,
  // and a namespace created since — or relabelled — may now match it, so
  // while one is in force the listing is read again on a cadence, and an
  // unchanged answer changes nothing. Only a complete listing is taken: what
  // restricted credentials could see is already what the view has.
  const scopedHere = chosen.length > 0;
  useEffect(() => {
    if (!scopedHere || !complete) return;
    let live = true;
    const timer = setInterval(() => {
      void listNamespaces(context.name).then((outcome) => {
        if (!live || outcome.error || !outcome.summaries) return;
        noteNamespaceListing(
          clusterId,
          outcome.summaries.map((ns) => ({ name: ns.name, labels: ns.labels })),
          true,
        );
      });
    }, RELIST_MS);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [scopedHere, complete, context.name, clusterId]);

  if (namespaces === null) {
    return (
      <Button variant="secondary" className="justify-between gap-1.5" disabled aria-label="Namespaces">
        <Spinner label="Loading namespaces" />
        Loading namespaces…
      </Button>
    );
  }

  const scoped = scope !== null && scope.status !== "gone";
  // What the namespace rows show as ticked. Scoped to collections, that is
  // what they matched here — so the rows agree with the list under them —
  // and nothing until that is known. Ticking or unticking one from there
  // turns it into namespaces picked by hand, starting from the collection's:
  // "this collection, less one" is a hand-picked set, and is saved as one.
  const resolved = scoped && scope.status === "ready" && !selectsNothing(scope.selection) ? [...scope.selection] : [];
  const ticked = scoped ? resolved : selection;
  const handPicked = scoped ? [] : selection;
  const offered = byRecentUse(collections, "namespace");
  const active = new Set(scoped ? scope.collections.map((c) => c.name) : []);

  const toggle = (name: string) => {
    const next = active.has(name) ? [...active].filter((n) => n !== name) : [...active, name];
    setCollections(clusterId, next);
  };

  const summary = scoped ? collectionSummary(scope.collections, scope.status === "ready" ? scope.selection : null) : undefined;

  return (
    <>
      <MultiSelect
        options={(listing?.complete && complete ? listing.items.map((ns) => ns.name) : namespaces).map((ns) => ({ value: ns }))}
        selection={ticked}
        onChange={(next) => setNamespaces(clusterId, next)}
        allLabel="All namespaces"
        allChecked={!scoped && handPicked.length === 0}
        ariaLabel="Namespaces"
        summary={summary}
        // Two tabs side by side, Namespaces and Collections, each counting
        // what is chosen on it. The picker opens on the one in force.
        optionsLabel="Namespaces"
        openOn={scoped ? "other" : "options"}
        otherTab={{
          label: "Collections",
          count: active.size,
          rows: () =>
            offered.map((c) => (
              <PickerRow
                key={c.name}
                // Prefixed so a collection and a namespace of the same name
                // are two rows, not one.
                value={`collection:${c.name}`}
                label={c.name}
                checked={active.has(c.name)}
                onSelect={() => toggle(c.name)}
                mark="box"
                trailing={c.description ? <span className="truncate text-faint">{c.description}</span> : undefined}
              />
            )),
          empty:
            offered.length === 0
              ? "No collections yet. Pick namespaces, then Save as collection…"
              : "No collection matches the search",
        }}
        footerExtra={
          <>
            <button
              type="button"
              className="btn whitespace-nowrap !py-0"
              disabled={handPicked.length === 0}
              title={
                handPicked.length > 0
                  ? undefined
                  : scoped
                    ? "This is already a collection"
                    : "Pick namespaces first, then save them as a collection"
              }
              onClick={() => setSaving(handPicked)}
            >
              Save as collection…
            </button>
            <button
              type="button"
              className="btn ml-auto whitespace-nowrap !py-0"
              aria-label="Manage collections…"
              title="Manage collections in Settings"
              onClick={() => openSettings("collections")}
            >
              Manage…
            </button>
          </>
        }
      />
      {saving !== null && (
        <CollectionDialog
          members={saving}
          clusterId={clusterId}
          clusterName={context.name}
          onClose={() => setSaving(null)}
          onSaved={(saved) => setCollections(clusterId, [saved.name])}
        />
      )}
    </>
  );
}

/** "prod · 12 namespaces", or "prod, staging" while they resolve. */
function collectionSummary(collections: Collection[], selection: readonly string[] | null): string {
  const names = collections.length <= 2 ? collections.map((c) => c.name).join(", ") : `${collections.length} collections`;
  if (selection === null) return names;
  const count = selectsNothing(selection) ? 0 : selection.length;
  return `${names} · ${count} ${count === 1 ? "namespace" : "namespaces"}`;
}

/**
 * What a collection scope needs said beside the list it narrows: that it is
 * still being worked out, that it matches nothing here, that it could not be
 * worked out, or that the credentials could not see every namespace.
 *
 * Matching nothing is a state, not an error: the list is empty because the
 * collection says so, and the reader is told which collection, and offered
 * the way back to every namespace.
 */
export function CollectionScopeNotice({ context }: { context: ClusterContext }) {
  const clusterId = context.stableId;
  const chosen = useNamespaceCollections(clusterId);
  const scope = useCollectionScope(clusterId, chosen);
  const setCollections = useSetNamespaceCollections();
  if (scope === null || scope.status === "gone") return null;
  const names = scope.collections.map((c) => c.name).join(", ");
  const clear = () => setCollections(clusterId, []);

  if (scope.status === "failed") {
    return (
      <Alert tone="sev" title={`Collection ${names} could not be worked out`} className="mx-3 mt-3 mb-3" onDismiss={clear} dismissLabel="Show all namespaces">
        {scope.error}
      </Alert>
    );
  }
  if (scope.status !== "ready" || !scope.resolution) {
    return (
      <p className="mx-3 mt-2 flex items-center gap-1.5 text-[0.75rem] text-muted" role="status">
        <Spinner label={`Working out ${names}`} />
        Working out which namespaces {names} covers on {context.name}…
      </p>
    );
  }
  const problems = scope.resolution.collections.filter((c) => c.problem);
  if (scope.resolution.namespaces.length === 0) {
    return (
      <Alert tone="info" title={`No namespaces match ${names} on ${context.name}`} className="mx-3 mt-3 mb-3" onDismiss={clear} dismissLabel="Show all namespaces">
        {problems.length > 0
          ? problems.map((c) => `${c.name}: ${c.problem}`).join(" ")
          : "Nothing is listed until a namespace here matches its members, patterns or label selector."}
      </Alert>
    );
  }
  if (scope.resolution.partial) {
    return (
      <p className="mx-3 mt-2 text-[0.75rem] text-muted" role="status">
        These credentials cannot list every namespace on {context.name}, so {names} matched only the namespaces they
        can see. Results may be partial.
      </p>
    );
  }
  return null;
}
