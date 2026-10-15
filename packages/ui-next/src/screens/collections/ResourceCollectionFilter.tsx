import { useState } from "react";
import type { KindRef } from "@srelens/core";
import { Alert, MultiSelect, Spinner } from "@srelens/ui-kit";
import type { ResourceCollectionFilter } from "../../lib/resourceCollections";
import { openSettings } from "../../lib/settingsRequest";
import { CollectionDialog } from "./CollectionEditor";

/** `apps/Deployment`, or the bare kind for the core group — how a collection names it. */
function kindName(kind: KindRef, group?: string): string {
  const g = kind.group ?? group;
  return g ? `${g}/${kind.kind}` : kind.kind;
}

/**
 * The resource collections a list of one kind can be narrowed to (#885), next
 * to its namespace picker: a release's services, a team's deployments. Several
 * chosen are their union, always inside the namespaces in view.
 *
 * Save as collection… saves the rows ticked in the list as a new collection of
 * this kind, by name, so it matches the same names in every namespace in
 * view — one collection for a release across dev, staging and prod.
 */
export function ResourceCollectionPicker({
  filter,
  context,
  kind,
  coreGroup,
  noun,
  namespaces,
  ticked,
}: {
  filter: ResourceCollectionFilter;
  context: string;
  kind: KindRef;
  /** A built-in kind's group, when the view knows it (`apps` for Deployment). */
  coreGroup?: string;
  /** The kind's plural as the list says it: "deployments". */
  noun: string;
  namespaces: string[];
  /** The names of the rows ticked in the list, for Save as collection… */
  ticked: string[];
}) {
  const [saving, setSaving] = useState<string[] | null>(null);
  const chosen = filter.chosen.map((c) => c.name);
  const summary =
    chosen.length === 0 ? "Collections" : chosen.length <= 2 ? chosen.join(", ") : `${chosen.length} collections`;
  return (
    <>
      <MultiSelect
        options={filter.offered.map((c) => ({ value: c.name }))}
        selection={chosen}
        onChange={filter.choose}
        allLabel={`All ${noun}`}
        ariaLabel="Collections"
        summary={summary}
        selectAll={false}
        empty={`No collections cover ${noun} yet. Tick rows, then Save as collection…`}
        footerExtra={
          <>
            <button
              type="button"
              className="btn whitespace-nowrap !py-0"
              disabled={ticked.length === 0}
              title={ticked.length === 0 ? `Tick ${noun} in the list first, then save them as a collection` : undefined}
              onClick={() => setSaving(ticked)}
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
          type="resource"
          members={saving}
          kinds={[kindName(kind, coreGroup)]}
          previewKind={kind}
          previewNamespaces={namespaces}
          clusterName={context}
          onClose={() => setSaving(null)}
          onSaved={(saved) => filter.choose([saved.name])}
        />
      )}
    </>
  );
}

/**
 * What a list narrowed to collections needs said above it: that it is being
 * worked out, that nothing matched (an empty state, not an error, naming the
 * collections), that some namespaces could not be listed, or that it failed.
 */
export function ResourceCollectionNotice({ filter, noun, context }: { filter: ResourceCollectionFilter; noun: string; context: string }) {
  const { state, chosen } = filter;
  if (state.status === "off") return null;
  const names = chosen.map((c) => c.name).join(", ");
  const clear = () => filter.choose([]);
  if (state.status === "failed") {
    return (
      <Alert tone="sev" title={`Collection ${names} could not be worked out`} className="mx-3 mt-3 mb-3" onDismiss={clear} dismissLabel={`Show all ${noun}`}>
        {state.error}
      </Alert>
    );
  }
  if (state.status === "resolving") {
    return (
      <p className="mx-3 mt-2 flex items-center gap-1.5 text-[0.75rem] text-muted" role="status">
        <Spinner label={`Working out ${names}`} />
        Working out which {noun} {names} covers on {context}…
      </p>
    );
  }
  const problems = state.resolution.collections.filter((c) => c.problem);
  if (state.keys.size === 0) {
    return (
      <Alert tone="info" title={`No ${noun} match ${names} in the current scope`} className="mx-3 mt-3 mb-3" onDismiss={clear} dismissLabel={`Show all ${noun}`}>
        {problems.length > 0
          ? problems.map((c) => `${c.name}: ${c.problem}`).join(" ")
          : "Nothing is listed until something here matches its members, patterns or label selector."}
      </Alert>
    );
  }
  if (state.resolution.partial) {
    return (
      <p className="mx-3 mt-2 text-[0.75rem] text-muted" role="status">
        Some namespaces could not be listed on {context}, so {names} may have matched only part of what it covers.
      </p>
    );
  }
  return null;
}
