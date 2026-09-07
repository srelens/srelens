import { useEffect, useMemo, useState } from "react";
import {
  listCrds,
  rowInSelection,
  watchNamespaceForSelection,
  type ClusterContext,
  type CrdRef,
} from "@srelens/core";
import { useNamespaceOptions } from "@srelens/core/react";
import {
  Button,
  ColumnPicker,
  ErrorState,
  FilterBar,
  LiveSignal,
  LoadingState,
  ResizeHandle,
  Screen,
  SideRail,
  Table,
  filterTableData,
  type Column,
} from "@srelens/ui-kit";
import { useConsole } from "../console";
import { getKubeconfigFiles, useActiveContext } from "../lib/clusters";
import { useHiddenColumns } from "../lib/columnPrefs";
import { detailRoute, newRoute, parseDetailRoute } from "../lib/detailRoute";
import { customDescriptor } from "../lib/kinds/custom";
import { descriptorFor } from "../lib/kinds/descriptors";
import { withRowAffordances } from "../lib/kinds/rowAffordances";
import { rowKey, type KindDescriptor, type ListRow } from "../lib/kinds/types";
import { clampPeekWidth, savePeekWidth, setPeekWidth, usePeekBounds, usePeekWidth } from "../lib/peekWidth";
import { useResourceList } from "../lib/resourceList";
import { describe, isBuiltInKind } from "../lib/routes";
import { openTab } from "../lib/tabsStore";
import { useResource } from "../lib/useResource";
import { setNamespaces, useNamespaces } from "../lib/workspace";
import { FailureAlert, FailureState } from "../lib/errorCopy";
import { AboutKind } from "./crd/AboutKind";
import { ResourceDetailView } from "./detail/ResourceDetailView";
import { ResourceTabView } from "./detail/ResourceTabView";
import { ResourceBulk } from "./ResourceBulk";
import { useRowMenu } from "./ResourceMenu";
import {
  NamespaceErrorAlert,
  NamespacePicker,
  NoClusterScreen,
  StaleSelectionAlert,
  columnOptionsFor,
  defaultHiddenKeys,
  emptyTableCopy,
  toggleColumnVisibility,
  useResourceTabView,
} from "./resourceShell";

/** The row identifier: always shown, never offered to the column picker. */
const NAME_KEY = "name";

/** Stable identity for "no columns", so a memo on it does not churn. */
const NO_COLUMNS: Column<ListRow>[] = [];

/** The "About this kind" rail's width, from the design's own table (§12). */
const CRD_RAIL_WIDTH = 264;

/**
 * The resource list: one screen for every `/k/<slug>` route there is.
 *
 * It names no kind. The slug is looked up as a descriptor — a built-in one for
 * core's kinds, or one built from the cluster's own CRDs — and everything on
 * screen is composed around that: the columns, the scope, whether there is a
 * namespace picker, whether the rows arrive on a watch or a poll. That is what
 * lets 34 sidebar entries plus every custom resource an operator installed
 * share a single screen instead of 34 near-copies.
 *
 * Split in two because of the guard rail at the top: with no cluster in focus
 * there is no context name to call core with, and a hook cannot be skipped.
 * The half below the split is only ever mounted once there is one.
 */
export function Resources({ route }: { route: string }) {
  const context = useActiveContext();
  const slug = route.slice("/k/".length);
  // The tab strip already knows what this route is called; asking `describe`
  // keeps the screen's title and the tab's title the same string.
  const title = describe(route, context?.name).title;

  if (!context) {
    return <NoClusterScreen title={title} noun="resources" />;
  }

  return <KindList route={route} slug={slug} title={title} context={context} />;
}

function KindList({
  route,
  slug,
  title,
  context,
}: {
  route: string;
  slug: string;
  title: string;
  context: ClusterContext;
}) {
  // Core takes a context *name*; the workspace holds a `stableId`. The two are
  // never interchangeable — see `lib/clusters`.
  const name = context.name;
  const files = getKubeconfigFiles();
  const builtIn = isBuiltInKind(slug);
  const { ask } = useConsole();

  // Discovery runs only for a slug that is not one of core's kinds: listing
  // pods must not cost a CRD round trip, and must not fail on a cluster whose
  // RBAC refuses `listCRDs`.
  const discovery = useResource<CrdRef[]>(
    async () => {
      if (builtIn) return [];
      const out = await listCrds(name);
      // `listCrds` reports failure in its result rather than by rejecting, and
      // "this cluster has no such CRD" is different news from "we were not
      // allowed to look".
      if (out.error) throw new Error(out.error);
      return out.crds ?? [];
    },
    [name, builtIn],
  );
  const crds = discovery.data;

  /**
   * The CRD this route names, once discovery has answered — `undefined` for a
   * built-in kind, and for a slug this cluster has no definition for (a tab
   * restored from a session can name a kind whose operator is gone).
   *
   * Looked up ONCE, here, rather than inside the memo below and again beside
   * it for the rail: the columns and the "About this kind" rail have to
   * describe the same definition, and two finds are two chances for them not
   * to. `builtIn` is the same flag discovery itself is gated on, so the slug's
   * shape is still tested in exactly one place. (`customDescriptorFor` did
   * this find and was deleted with this hoist — it had no other caller.)
   */
  const crd = useMemo(
    () => (builtIn ? undefined : crds?.find((c) => c.name === slug)),
    [builtIn, crds, slug],
  );

  const descriptor = useMemo(() => {
    if (builtIn) return descriptorFor(slug);
    // The same variance cast `descriptors.ts` makes for its typed column sets:
    // `CustomRow` is a proper subtype of `ListRow` on the data side, but
    // `Column`'s render/sort functions take the row contravariantly, so
    // TypeScript cannot see the assignment is safe. Every function on a custom
    // column only reads fields `ListRow` does not promise (`columns`,
    // `sortKeys`), so a bare `ListRow` cannot reach one wrongly.
    return (crd ? customDescriptor(crd) : undefined) as KindDescriptor<ListRow> | undefined;
  }, [builtIn, slug, crd]);

  const selection = useNamespaces(context.stableId);
  const { namespaces, scope, error: namespaceError } = useNamespaceOptions(name, files);

  // A namespace-restricted credential has one namespace and no way to ask for
  // another. Written to the workspace store rather than held here, so every
  // screen looking at this cluster follows the same scope.
  useEffect(() => {
    if (scope) setNamespaces(context.stableId, [scope]);
  }, [scope, context.stableId]);

  const clusterScoped = descriptor?.scope === "cluster";
  // One selected namespace is watched directly; none or several are watched
  // across the cluster and narrowed below, which is core's own rule.
  const namespace = clusterScoped ? "" : watchNamespaceForSelection(selection);
  const list = useResourceList<ListRow>(name, slug, descriptor, namespace, files);

  const allColumns = descriptor?.columns ?? NO_COLUMNS;
  const defaultHidden = useMemo(() => defaultHiddenKeys(allColumns), [allColumns]);
  const hidden = useHiddenColumns(slug, defaultHidden);
  const columns = useMemo(
    // The identifier is never hidden: a table whose rows lost their name is
    // not a table any more. `ColumnPicker` pins the same key.
    () => allColumns.filter((column) => column.key === NAME_KEY || !hidden.has(column.key)),
    [allColumns, hidden],
  );
  // The dot and the ask chip, layered on after hiding — not offered to
  // `ColumnPicker` (which is built from `allColumns` below) and not part of
  // what `filterTableData` searches. `flagged` is the only per-kind
  // knowledge either affordance needs, and most kinds have none; with no
  // descriptor yet, columns pass through undecorated, same as before.
  const renderedColumns = useMemo(
    () =>
      descriptor
        ? withRowAffordances(columns, (row) => descriptor.flagged?.(row) ?? false, ask)
        : columns,
    [columns, descriptor, ask],
  );

  // Sort, filter text and filter column live on the tab — see
  // `useResourceTabView`'s own comment for why, and why `filterKey` is
  // derived rather than merely cleared when this screen hides a column.
  const { tabId, sort, filter, filterKey, setFilter, setSort, setFilterKey } = useResourceTabView(route, columns);

  const rows = useMemo(
    () =>
      clusterScoped
        ? list.rows
        : list.rows.filter((row) => rowInSelection(row.namespace ?? "", selection)),
    [list.rows, clusterScoped, selection],
  );
  const filtered = useMemo(
    () => filterTableData(rows, columns, filter, filterKey),
    [rows, columns, filter, filterKey],
  );

  // Called unconditionally — same reason every hook above it is: the guard
  // for "no descriptor yet" is a `return` below, not a skip, and a hook
  // cannot follow one. `descriptorFor`'s own kind and actions feed it when
  // there is a descriptor; an absent one leaves the row menu with nothing to
  // gate on, which is moot since no row ever renders without one.
  const { items: rowMenuItems, dialog: rowMenuDialog } = useRowMenu({
    context: name,
    kind: descriptor?.k8sKind ?? "",
    actions: descriptor?.actions ?? {},
    group: descriptor?.group,
  });

  // The checkbox column's selection. Table owns the interaction (toggle,
  // shift-click range, select-all-of-filtered); this screen only holds the
  // set and hands it to `ResourceBulk`, which resolves each key back to a
  // row through the same `getRowKey` formula passed to `Table` below.
  const [selected, setSelected] = useState<Set<string>>(new Set());

  // A namespace switch makes every selected key's namespace half meaningless
  // — cleared rather than left to be silently resolved away, or a later
  // switch back to a namespace that still has the same-named row would
  // resurrect a checkbox the reader never re-checked. `selection` is a
  // stable array reference from `useNamespaces` (it only changes identity
  // when its contents actually change), so this does not fire on every
  // render.
  //
  // A CLUSTER switch makes the WHOLE key meaningless, and `selection` cannot
  // see that one: `useNamespaces` answers every cluster with no selection out
  // of one shared empty array, so on "all namespaces" — the setting most
  // readers leave it on — both clusters hand back the same identity and the
  // dependency above never moves. Nothing remounts either (the rail switches
  // `context` in place), so the checked keys survived into a cluster the
  // reader had checked nothing in, matched whatever `namespace/name` the new
  // rows happened to share, and `ResourceBulk` pinned the NEW `name` when the
  // bar was pressed — so its own cluster gate saw no divergence to warn
  // about. `name` is the identity every write below is addressed to, which is
  // why it is what this resets on; it is a string, so the effect still only
  // fires when the cluster actually changes.
  useEffect(() => setSelected(new Set()), [selection, name]);

  // The peek's subject: which row the pane beside the table is showing, or
  // `null` for no pane at all. Only the row's identity is held — the pane
  // reads the object itself — so nothing here can go stale against the list.
  //
  // The setter returns the PREVIOUS value when the row clicked is the one
  // already on show, so a second click on the same row is not even a state
  // change. `useObject` is keyed on the primitives below and would not refetch
  // either way, but a re-render per click of a list that already answers is
  // work nobody asked for, and holding the identity stable says so.
  const [peek, setPeek] = useState<{ context: string; namespace: string | null; name: string } | null>(
    null,
  );

  /**
   * The cluster the peek was opened on, held WITH the row and checked here —
   * and a peek held for any other cluster is dropped before it can be
   * rendered.
   *
   * The comment above is why this has to be said separately: "the pane reads
   * the object itself" is what makes a row leaving the table harmless, and it
   * holds for a namespace change while failing for a cluster one.
   * `namespace/name` is not unique across clusters, so the held identity
   * survived a rail switch and quietly resolved against the NEW cluster — a
   * pane the reader opened on one cluster's row, showing another cluster's
   * object, with that object's ports and its actions under it. (`Helm` drops
   * its own pane's subject on a re-list for exactly this reason.)
   *
   * Adjusted DURING render rather than in an effect, which is the same choice
   * `useObject` and `useDetailPaneState` both make and for the same reason: a
   * child's effects run before this component's, so an effect here would let
   * the pane mount once against the new cluster and send a `getObject` for an
   * object the reader never asked about. React re-runs this component with the
   * cleared state before committing anything, so the pane is not merely
   * closed again — it is never opened on the wrong cluster at all. That is
   * also why the JSX below tests `peek` alone: a second copy of this
   * comparison down there was measured to be unfalsifiable (removing it
   * changed no test), because a render that adjusts state here is discarded
   * before it can commit.
   */
  if (peek && peek.context !== name) setPeek(null);

  // How wide that pane is. Module state rather than this screen's, so the
  // width the reader dragged is the width every resource list opens at — and
  // so it outlives the tab.
  //
  // The bounds are this screen's own business, though, and they come off the
  // row below rather than off the window: the cluster rail and the navigation
  // sidebar are outside this screen and the sidebar is itself resizable, so
  // the window is space the list does not have. Clamped here rather than in
  // the store, and on the way out rather than written back, so a pane
  // squeezed by a narrow row widens again when the row does.
  const listRow = usePeekBounds();
  const peekWidth = clampPeekWidth(usePeekWidth(), listRow.bounds);

  function peekAt(rowNamespace: string | null, rowName: string) {
    setPeek((prev) =>
      prev && prev.name === rowName && prev.namespace === rowNamespace
        ? prev
        : // The cluster is captured with the row, at the click — see the gate
          // above. `name` is the cluster the row was listed from, which is
          // the only cluster this pane is ever about.
          { context: name, namespace: rowNamespace, name: rowName },
    );
  }

  const lower = title.toLocaleLowerCase();

  function onToggleColumn(key: string) {
    toggleColumnVisibility({ key, storageKey: slug, hidden, filterKey, tabId, defaultHidden });
  }

  if (!descriptor) {
    return (
      <Screen title={title} eyebrow={name} fill>
        {!builtIn && discovery.status === "loading" ? (
          <LoadingState label={`Looking for ${slug}`} />
        ) : discovery.status === "error" ? (
          <FailureState
            title={`Could not look up ${slug}`}
            error={discovery.error}
            onRetry={discovery.reload}
          />
        ) : (
          // A route string outlives the cluster it was written against: a tab
          // restored from a session can name a custom resource whose operator
          // has since been uninstalled. Naming the slug is what tells the
          // reader which tab to close.
          <ErrorState
            title={`Nothing on ${name} is called ${slug}`}
            detail="It is neither one of the kinds srelens knows nor a custom resource this cluster has. If an operator defined it, that operator may be gone."
            onRetry={builtIn ? undefined : discovery.reload}
          />
        )}
      </Screen>
    );
  }

  const columnOptions = columnOptionsFor(allColumns);

  // Read out here rather than inside the function below: a function
  // declaration closes over the widest type `descriptor` ever has, so the
  // narrowing the guard above performed is not in scope within one.
  const k8sKind = descriptor.k8sKind;

  /**
   * Promote a row to a tab of its own — the row's double click and the peek
   * header's "Open tab" alike. One expression rather than two: `openTab`
   * dedupes by route string, so two spellings of the same resource would
   * quietly become two tabs, which is the very bug the route model was built
   * to stop.
   */
  function openRowTab(rowNamespace: string | null, rowName: string) {
    openTab(detailRoute(k8sKind, rowNamespace, rowName), { clusterName: name });
  }

  // Loading and error each replace the table with their own state below; the
  // stale-rows alert and the bulk bar only ever mean something once there is
  // a table to warn about or select from.
  const showRows = list.status !== "loading" && list.status !== "error";

  /**
   * The list and the peek, side by side. Still not `SideRail`, which is the
   * kit's fixed rail and offers no grip by design — the peek is the one thing
   * on this screen the reader drags, and it carries a measured clamp and a
   * persisted width that a fixed rail has no use for. `min-w-0` on the
   * table's own column is what keeps the peek from widening this row past the
   * window — without it a flex item refuses to shrink below its content and
   * the whole screen scrolls sideways instead of the table scrolling inside
   * itself.
   *
   * Named rather than written inline because a custom resource wraps it in a
   * rail and a built-in kind does not, and one copy of it is the only way
   * those two branches cannot drift.
   */
  const listAndPeek = (
    <div ref={listRow.ref} className="flex min-h-0 flex-1">
      <div className="scroll min-h-0 min-w-0 flex-1">
        {list.status === "loading" ? (
          <LoadingState label={`Loading ${lower}`} />
        ) : list.status === "error" ? (
          <FailureState
            title={`Could not list ${lower} on ${name}`}
            error={list.error}
            onRetry={list.reload}
          />
        ) : (
          <Table
            columns={renderedColumns}
            data={filtered}
            getRowKey={rowKey}
            selection={{ selected, onChange: setSelected }}
            sort={sort}
            onSortChange={setSort}
            activeFilterKey={filterKey}
            onActiveFilterKeyChange={setFilterKey}
            // Single click peeks, double click (or Enter) opens the tab —
            // `Table` owns both gestures, so a row is reachable from the
            // keyboard either way.
            onRowClick={(row) => peekAt(row.namespace ?? null, row.name)}
            onRowActivate={(row) => openRowTab(row.namespace ?? null, row.name)}
            rowMenu={rowMenuItems}
            rowMenuLabel={`${title} actions`}
            {...emptyTableCopy(rows.length, lower, name, clusterScoped ? "" : " in the namespaces you are looking at")}
          />
        )}
      </div>
      {peek && (
        // Deliberately NOT keyed on the subject: `ResourceDetailView` gates its
        // own panes on the target it is rendering for, and remounting per
        // row would throw away the reader's selected pane on every click —
        // the one thing that component's own comments say must survive a
        // subject change.
        //
        // A plain `div`, not an `aside`: `Inspector` is already a named
        // region, and a second complementary landmark around it would be
        // noise (see the kit's own note on that).
        //
        // `relative` and an inline width rather than a `w-` utility: the
        // grip is positioned against this box, and the width is now a
        // number the reader owns. Changing it re-styles this element in
        // place — the pane below is not keyed and does not remount, or
        // every frame of a drag would refetch the resource.
        <div className="relative flex min-h-0 shrink-0 flex-col" style={{ width: peekWidth }}>
          {/* No `rule-l` on the box: the grip draws the rule between the
              list and the pane itself, the same way the sidebar's does. It
              is named after what the reader called it — "resource details"
              — since `ResizeHandle` announces itself as `Resize {label}`.
              Written live and persisted once on release. */}
          <ResizeHandle
            label="the resource details"
            width={peekWidth}
            minWidth={listRow.bounds.minWidth}
            maxWidth={listRow.bounds.maxWidth}
            edge="left"
            onResize={setPeekWidth}
            onCommit={savePeekWidth}
          />
          <ResourceDetailView
            context={name}
            kind={descriptor.k8sKind}
            namespace={peek.namespace}
            name={peek.name}
            // The one prop the tab host does not pass, carrying both of the
            // controls the design gives the peek's header. Promoting does
            // not dismiss: the reader asked for a tab, not for the list to
            // stop showing them what they were looking at.
            peek={{
              onClose: () => setPeek(null),
              onOpenTab: () => openRowTab(peek.namespace, peek.name),
            }}
          />
        </div>
      )}
    </div>
  );

  return (
    <Screen
      title={title}
      eyebrow={name}
      fill
      actions={
        <>
          {descriptor.source === "watch" && (
            <LiveSignal
              // The label carries the meaning; the tone only colours it.
              label={list.watch === "live" ? "Live" : "Stream lost"}
              tone={list.watch === "live" ? "ok" : "warn"}
            />
          )}
          <ColumnPicker
            columns={columnOptions}
            hidden={hidden}
            onToggle={onToggleColumn}
            pinnedKey={NAME_KEY}
          />
          <Button
            variant="secondary"
            title={`Create a resource on ${name} from a template`}
            onClick={() => openTab(newRoute(name), { clusterName: name })}
          >
            New
          </Button>
        </>
      }
    >
      <FilterBar
        value={filter}
        onValueChange={setFilter}
        label={`Filter ${lower}`}
        placeholder={`Filter ${lower}…`}
      >
        {!clusterScoped && (
          <NamespacePicker
            namespaces={namespaces}
            selection={selection}
            onChange={(next) => setNamespaces(context.stableId, next)}
          />
        )}
      </FilterBar>

      {!clusterScoped && <NamespaceErrorAlert error={namespaceError} />}

      {!clusterScoped && (
        <StaleSelectionAlert
          selection={selection}
          namespaces={namespaces}
          onReset={() => setNamespaces(context.stableId, [])}
        />
      )}

      {showRows && list.error && (
        // Rows and an error together: the last good list is still on screen
        // and is no longer being refreshed. Emptying the table would throw
        // away the only information the reader has. Pinned above the
        // scrolling table rather than inside it (D6+D7 review) — a "these
        // rows are stale" warning the reader scrolls past no longer warns
        // anyone. The table runs flush to the panel, so the alert carries
        // its own inset rather than borrowing the container's.
        <FailureAlert title={`These ${lower} are stale`} error={list.error} className="mx-3 mt-3 mb-3" />
      )}
      {showRows && (
        // Same reason as the alert above: selection actions that scroll out
        // of reach while the selection persists are worse than a warning
        // nobody sees.
        <ResourceBulk
          selected={selected}
          kind={lower}
          descriptor={descriptor}
          context={name}
          rows={filtered}
          onDone={() => setSelected(new Set())}
        />
      )}
      {crd ? (
        // The rail is the whole of what a custom resource's list adds. It is
        // mounted off `crd` rather than off the slug's shape: `builtIn` already
        // decided that once, upstream, and it is what gates discovery itself.
        <SideRail
          head="About this kind"
          width={CRD_RAIL_WIDTH}
          // The count is what the TABLE beside it holds — narrowed by the
          // namespace selection, not a cluster-wide total, because no such
          // total is available without a second call and a number that
          // disagreed with the rows under it would be worse than a narrow
          // one. It is withheld entirely until the list has answered:
          // `AboutKind` drops the row rather than drawing `Objects 0`, which
          // is a wrong number a reader would believe.
          rail={<AboutKind crd={crd} context={name} objects={showRows ? rows.length : undefined} />}
        >
          {/* The left pane's own head, as the design words it. `crd.kind` again
              — the slug is a plural DNS name and reads as one. */}
          <div className="pane-head">{`${crd.kind} \u00b7 custom resource`}</div>
          {listAndPeek}
        </SideRail>
      ) : (
        listAndPeek
      )}
      {/* Outside the scrolling table body: a `ConfirmDialog` is a portal
          anyway, but a clipped ancestor is one fewer thing to reason about. */}
      {rowMenuDialog}
    </Screen>
  );
}

/**
 * The resource detail route's screen — the peek's other host.
 *
 * One tab, one resource, filled edge to edge by `ResourceTabView`: the design's
 * own full-tab screen, which is NOT the peek at a wider width. Spec rule R-5
 * said it was, and the user's mock of this tab retired it — a breadcrumb
 * header with the actions on the same line, a metric strip, a three-column
 * fact grid, the containers table inline on Overview. What the two hosts
 * still share, they share through `detail/detailData`: one read of the object,
 * one lazy-load rule per pane, one derivation of a kind's facts, the same
 * per-kind blocks, the same actions.
 *
 * Everything it shows comes out of the route string:
 * `/k/<kind>/<namespace>/<name>` already carries the Kubernetes kind — not
 * the list screen's slug, and not for built-in kinds only, since
 * `customDescriptor` mints a CRD's route from `crd.kind` too — so there is
 * nothing to look up and nothing to keep in step with the list.
 *
 * No `Screen` wrapper: `ResourceTabView` already heads the page with the
 * resource's name and its breadcrumb, and the window's tab strip titles the
 * tab with the same name (`describe`). A toolbar above it would say
 * everything twice and cost the page a strip's worth of height.
 */
export function ResourceDetailScreen({ route }: { route: string }) {
  const context = useActiveContext();
  const parts = parseDetailRoute(route);
  // The strip's own title for this route — the resource's name — so the
  // guard below names the same thing the tab does.
  const title = parts?.name ?? describe(route).title;

  if (!context) {
    return <NoClusterScreen title={title} noun="resources" />;
  }

  if (!parts) {
    // Unreachable through `screenFor`, which only sends a route here once
    // `parseDetailRoute` has already accepted it. Kept as a state rather than
    // a throw: a route string can arrive from a persisted session, and a tab
    // that says what is wrong with it is worth more than a blank surface.
    return (
      <Screen title={title} eyebrow={context.name} fill>
        <ErrorState
          title={`${route} does not name a resource`}
          detail="A resource tab's route is /k/<kind>/<namespace>/<name>. Close this tab and open the resource from its list."
        />
      </Screen>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ResourceTabView
        context={context.name}
        kind={parts.kind}
        namespace={parts.namespace}
        name={parts.name}
      />
    </div>
  );
}
