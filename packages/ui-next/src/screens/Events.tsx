import { useEffect, useMemo, useState } from "react";
import {
  eventVerdict,
  plural,
  rowInSelection,
  watchNamespaceForSelection,
  type ClusterContext,
} from "@srelens/core";
import { useNamespaceOptions } from "@srelens/core/react";
import {
  Button,
  ColumnPicker,
  Eyebrow,
  FilterBar,
  LiveSignal,
  LoadingState,
  Screen,
  SideRail,
  Table,
  Tabs,
  filterTableData,
  type TabItem,
} from "@srelens/ui-kit";
import { useConsole } from "../console";
import { getKubeconfigFiles, useActiveContext } from "../lib/clusters";
import { useHiddenColumns } from "../lib/columnPrefs";
import { detailRoute } from "../lib/detailRoute";
import { FailureAlert, FailureState } from "../lib/errorCopy";
import { Icons } from "../lib/icons";
import {
  EVENT_DESCRIPTOR,
  eventColumns,
  eventNamespace,
  involvedObject,
  withEventAsk,
  type EventRow,
} from "../lib/kinds/events";
import { useResourceList } from "../lib/resourceList";
import { describe } from "../lib/routes";
import { openTab } from "../lib/tabsStore";
import { setNamespaces, useNamespaces } from "../lib/workspace";
import { ReasonRail } from "./events/ReasonRail";
import {
  NamespaceErrorAlert,
  NamespacePicker,
  NoClusterScreen,
  StaleSelectionAlert,
  columnOptionsFor,
  emptyTableCopy,
  toggleColumnVisibility,
  useResourceTabView,
} from "./resourceShell";

/** The kind the engine watches, and the key its column preferences live under. */
const KIND = "events";

/** The id of the segment that narrows nothing (§8's own, lower-case). */
const ALL = "all";

/**
 * §8's segments, in its order and with its ids: the two real ones are the
 * values Kubernetes itself puts in `Event.type`, so the control filters on the
 * field rather than on a word this screen invented for it.
 */
const SEGMENTS: TabItem[] = [
  { id: ALL, label: "All" },
  { id: "Warning", label: "Warnings" },
  { id: "Normal", label: "Normal" },
];

/**
 * What the free-text field searches — the three fields its placeholder names,
 * and no others.
 *
 * Every other list in the app searches every column, because every other list
 * says "Filter pods…" and promises nothing in particular. This one names its
 * fields on the field itself, and a search that also matched the namespace
 * column would quietly disagree with the words under the reader's cursor while
 * the picker beside it is what narrows by namespace.
 *
 * **One divergence from §8, deliberately:** it matches `message` and `reason`
 * case-insensitively but `object` case-SENSITIVELY. All three are matched the
 * same way here, because that is what `filterTableData` does and so what every
 * other list in the app does. A field that changed its matching rule between
 * the three fields its own placeholder names is a rule no reader could learn,
 * and the case-sensitive half is the one that fails closed: a reader typing
 * `pod/web` would be told there is no such event.
 */
const SEARCH_KEYS = new Set(["reason", "message", "object"]);

/**
 * The Reason column is this table's identifier: it is the one the design draws
 * `font-medium`, the way every other list draws its Name, and an events table
 * with no reason on its rows is not a table any more. So it is what the column
 * picker holds on — there is no Name column here to pin.
 */
const PINNED_KEY = "reason";

/** §8's own width for the by-reason rail. Fixed: nothing here resizes. */
const REASON_RAIL_WIDTH = 250;

/** §8's header chip, verbatim: the word on it and the question behind it. */
const GROUP_BY_CAUSE_LABEL = "Group by cause";
const GROUP_BY_CAUSE = "What do these warning events have in common?";

/**
 * `/events` — the design's Events screen (§8).
 *
 * Its rows come off the very engine every resource list runs on:
 * `useResourceList` with the events descriptor, so a watch, a fallen-back
 * poll, the row cache and cancellation are all the ones already in service
 * rather than a second copy that drifts. What is its own is the chrome — an
 * eyebrow that counts what is on screen, a segmented control over the event
 * type, and a row click that opens the object an event is ABOUT rather than
 * the event itself.
 *
 * Split in two the way `Resources.tsx` and `Workloads.tsx` are: with no
 * cluster in focus there is no context name to watch, and a hook cannot be
 * skipped, so the guard is a `return` before any hook runs.
 */
export function Events({ route }: { route: string }) {
  const context = useActiveContext();
  const title = describe(route, context?.name).title;

  if (!context) {
    return <NoClusterScreen title={title} noun="events" />;
  }

  return <EventList route={route} title={title} context={context} />;
}

function EventList({
  route,
  title,
  context,
}: {
  route: string;
  title: string;
  context: ClusterContext;
}) {
  // Core takes a context *name*; the workspace holds a `stableId`. The two are
  // never interchangeable — see `lib/clusters`.
  const name = context.name;
  const files = getKubeconfigFiles();
  const { ask } = useConsole();

  const selection = useNamespaces(context.stableId);
  const { namespaces, scope, error: namespaceError } = useNamespaceOptions(name, files);

  // A namespace-restricted credential has one namespace and no way to ask for
  // another — written to the workspace store rather than held here, so every
  // screen looking at this cluster follows the same scope.
  useEffect(() => {
    if (scope) setNamespaces(context.stableId, [scope]);
  }, [scope, context.stableId]);

  // One selected namespace is watched directly; none or several are watched
  // across the cluster and narrowed below, which is core's own rule. Events
  // are namespaced, so there is no cluster-scoped branch to take.
  const namespace = watchNamespaceForSelection(selection);
  const list = useResourceList<EventRow>(name, KIND, EVENT_DESCRIPTOR, namespace, files);

  const hidden = useHiddenColumns(KIND);
  const columns = useMemo(
    () => eventColumns.filter((column) => column.key === PINNED_KEY || !hidden.has(column.key)),
    [hidden],
  );
  // The ask chip, layered on AFTER hiding — the same split `Resources.tsx`
  // makes with `withRowAffordances`. A column the reader can neither sort nor
  // hide has no business in the column picker (which is built from the full
  // set below) and none in what the search looks through.
  const renderedColumns = useMemo(() => withEventAsk(columns, ask), [columns, ask]);

  // Sort, filter text and filter column live on this route's own tab, so they
  // survive a restart with it — see `useResourceTabView`'s own comment.
  const { tabId, sort, filter, filterKey, setFilter, setSort, setFilterKey } = useResourceTabView(
    route,
    columns,
  );

  // The segment narrows the rows on screen and nothing else: it never touches
  // the watch above, so switching segments cannot re-list. Component state
  // rather than the tab's, exactly as `Workloads.tsx` holds its own.
  const [segment, setSegment] = useState(ALL);

  // Narrowed by the picker's selection through `eventNamespace` rather than by
  // reading the field itself: where an event's namespace comes from has already
  // changed once (it used to be recovered from the composite key), and the
  // helper is the one place that answers for it — the Namespace column, the row
  // click below and the by-reason rail all ask it too. A cluster-scoped event
  // belongs to no namespace and so falls outside any selection that names one.
  const rows = useMemo(
    () => list.rows.filter((row) => rowInSelection(eventNamespace(row), selection)),
    [list.rows, selection],
  );
  const segmented = useMemo(
    () => (segment === ALL ? rows : rows.filter((row) => row.type === segment)),
    [rows, segment],
  );
  // A funnel scopes the search to the one column the reader pointed it at, and
  // that is their instruction rather than the placeholder's promise — so the
  // narrowed set applies only to the whole-row search.
  const searchColumns = useMemo(
    () => (filterKey ? columns : columns.filter((column) => SEARCH_KEYS.has(column.key))),
    [columns, filterKey],
  );
  const filtered = useMemo(
    () => filterTableData(segmented, searchColumns, filter, filterKey),
    [segmented, searchColumns, filter, filterKey],
  );

  // Both counts are of what is ON SCREEN, per §8 — a header that counted the
  // loaded set would disagree with the rows under it the moment anyone typed.
  // What counts as a warning is core's own reading of the type, the same one
  // the Type pill is toned by, so a type this screen shows plain is never
  // counted as alarming here.
  const warnings = filtered.filter((row) => eventVerdict(row.type).bad).length;

  const lower = title.toLocaleLowerCase();

  function onToggleColumn(key: string) {
    toggleColumnVisibility({ key, storageKey: KIND, hidden, filterKey, tabId });
  }

  /**
   * Open what the event is ABOUT.
   *
   * `row.name` is the event's own key (`billing/api-7.17a`) and names nothing
   * a reader wants to look at; `row.object` names the pod. The namespace comes
   * through the same helper the Namespace column reads, because an involved
   * object lives in the namespace its event was recorded in — and `null` for a
   * cluster-scoped one, which is what `detailRoute` spells `-`.
   *
   * An object that arrived without a kind in front of it names no route at
   * all, and a tab that cannot resolve is worse than a click that did nothing.
   */
  function openInvolved(row: EventRow) {
    const { kind, name: objectName } = involvedObject(row);
    if (!kind || !objectName) return;
    openTab(detailRoute(kind, eventNamespace(row) || null, objectName), { clusterName: name });
  }

  // Loading and error each replace the table with their own state; the
  // stale-rows alert only ever means something once there is a table to warn
  // about.
  const showRows = list.status !== "loading" && list.status !== "error";
  const Sparkle = Icons.ask;

  return (
    <Screen
      title={title}
      eyebrow={name}
      fill
      actions={
        <>
          {/* §8's two header actions, in its order. `<n> events · <m>
              warnings` is that document's notation for the shape of the line,
              not its copy — nothing in it asks for the letter `s` when n is 1,
              and a cluster with one warning is the ordinary case (the demo one
              splits 1 / 63). `plural` is the house answer, already worn in
              four places. */}
          <Eyebrow>{`${plural(filtered.length, "event")} \u00b7 ${plural(warnings, "warning")}`}</Eyebrow>
          {/* A `Button` rather than the row's `AskChip`, for the reason
              `DetailActions` gives: the chip is `opacity: 0` until its row is
              hovered, which is right for one of forty rows and invisible on a
              toolbar. The visible word is the design's; the question goes in
              the accessible name behind it, where it says what will actually
              be sent — the same split the chip itself makes. */}
          <Button
            type="button"
            size="sm"
            aria-label={`${GROUP_BY_CAUSE_LABEL}: ${GROUP_BY_CAUSE}`}
            title={`${GROUP_BY_CAUSE_LABEL}: ${GROUP_BY_CAUSE}`}
            onClick={() => ask(GROUP_BY_CAUSE)}
          >
            <Sparkle size={12} aria-hidden="true" />
            {GROUP_BY_CAUSE_LABEL}
          </Button>
          {/* Not in the design, which draws a screen whose fixtures never
              fail. The rows arrive on a watch that can drop to a poll, and the
              label carries that meaning; the tone only colours it. */}
          <LiveSignal
            label={list.watch === "live" ? "Live" : "Stream lost"}
            tone={list.watch === "live" ? "ok" : "warn"}
          />
          <ColumnPicker
            columns={columnOptionsFor(eventColumns)}
            hidden={hidden}
            onToggle={onToggleColumn}
            pinnedKey={PINNED_KEY}
          />
        </>
      }
    >
      {/* §8's two panes: the table pane, and the fixed 250px rail beside it.
          The rail is handed the segmented set — after the namespace selection
          and the type control, but BEFORE the search — so it describes the
          screen rather than the cluster while staying navigable.

          §8 says "the currently filtered set", and the mock's rail does narrow
          to the row just clicked. WE DO NOT FOLLOW IT, because the rail is
          both the way in and the way back out: clicking `Unhealthy` sets the
          search to it, and a rail rebuilt from the search would then contain
          only `Unhealthy` — the reader has lost the list of the OTHER things
          going wrong, which is the entire question the rail answers, with
          nothing on screen to say that clearing the search box is what brings
          it back. A control that destroys itself when used is a dead end. The
          mock runs on fixture data where nobody clicked twice.

          The type control still reshapes it, which is right: All, Warnings and
          Normal are genuinely different questions, and each has its own answer
          to "what is going wrong". */}
      <SideRail
        head="By reason"
        width={REASON_RAIL_WIDTH}
        rail={<ReasonRail rows={segmented} onPick={setFilter} />}
      >
        <FilterBar
          value={filter}
          onValueChange={setFilter}
          label={`Filter ${lower}`}
          // Verbatim from §8. The label above is what NAMES the field; this only
          // says what it matches.
          placeholder="Filter by reason, message or object"
        >
          <Tabs tabs={SEGMENTS} active={segment} onChange={setSegment} label="Type" variant="segmented" />
          {/* Inherited, not grown here: the selection is the workspace's, shared
              by every list looking at this cluster. */}
          <NamespacePicker
            namespaces={namespaces}
            selection={selection}
            onChange={(next) => setNamespaces(context.stableId, next)}
          />
        </FilterBar>

        <NamespaceErrorAlert error={namespaceError} />

        <StaleSelectionAlert
          selection={selection}
          namespaces={namespaces}
          onReset={() => setNamespaces(context.stableId, [])}
        />

        {showRows && list.error && (
          // Rows and an error together: the last good list is still on screen
          // and is no longer being refreshed. Emptying the table would throw
          // away the only information the reader has. Pinned ABOVE the scrolling
          // table rather than inside it — a "these rows are stale" warning the
          // reader scrolls past no longer warns anyone. The table runs flush to
          // the panel, so the alert carries its own inset.
          <FailureAlert title={`These ${lower} are stale`} error={list.error} className="mx-3 mt-3 mb-3" />
        )}

        <div className="scroll min-h-0 flex-1">
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
              // The event's own key, which already carries its namespace and is
              // unique across the cluster.
              getRowKey={(row) => row.name}
              sort={sort}
              onSortChange={setSort}
              activeFilterKey={filterKey}
              onActiveFilterKeyChange={setFilterKey}
              // One gesture, two ways in. §8 opens the involved object on a
              // single click — there is no peek here to take that click, and an
              // event is a report, not something to select — and `onRowActivate`
              // is what makes the same thing reachable with Enter. `openTab`
              // dedupes by route, so a double click is not two tabs.
              //
              // A second divergence from §8, which draws these rows
              // `cursor-default`: `Table` gives an interactive row
              // `cursor-pointer`, and it keeps it. A row that navigates on a
              // click while showing the cursor of one that does nothing is the
              // dead-affordance defect this spec objects to elsewhere, pointed
              // the other way round.
              onRowClick={openInvolved}
              onRowActivate={openInvolved}
              // The count is what the namespace selection left, before the
              // segment and the search — both of which are filters the reader
              // set, and "clear them" is different advice from "this cluster has
              // none".
              {...emptyTableCopy(rows.length, lower, name, " in the namespaces you are looking at")}
            />
          )}
        </div>
      </SideRail>
    </Screen>
  );
}
