import { ExtensionLogo } from "./ExtensionLogo";
import { ExtensionRequirements } from "./ExtensionRequirements";
import { AgeCell } from "../lib/ageCell";
import { useNamespaceOptions } from "@srelens/core/react";
import { Fragment, useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import {
  ageSortValue,
  eventVerdict,
  loadKubeconfigFiles,
  onExtensionResourceChanged,
  readExtension,
  itemStatuses,
  rowInSelection,
  type ClusterContext,
  type ExtensionDashboardCard,
  type ExtensionPage,
  type InstalledExtension,
  type EventSummary,
  type NormalizedStatus,
} from "@srelens/core";
import { AppCards } from "./DashboardCards";
import { plainText } from "./displayText";
import { STATUS_WORD } from "./AppStatus";
import { Button, ColumnPicker, EmptyState, FilterBar, LoadingState, StatusPill, Table, Tabs, Toolbar, filterMatcher, tableFilterError, type Column } from "@srelens/ui-kit";
import { ExtensionResults } from "./ExtensionResults";
import { useResource } from "../lib/useResource";
import { LiveNotice, LiveReaders, LiveStatus, useLiveReaders } from "./liveReaders";
import { readAcross, readerKind, readerNamespaces, readsClusterScope } from "./readAcross";
import { nameFilterAnswered, useCollectionNameFilter } from "../lib/collectionNameFilter";
import { FailureAlert, FailureState, NamespaceFailuresAlert } from "../lib/errorCopy";
import { useFallbackNamespace, useInCollectionScope, useNamespaces, useSetNamespaces } from "../lib/workspace";
import { CollectionNameNotice, CollectionScopeNotice, NamespaceScopePicker } from "../screens/collections/NamespaceScope";
import { ListCount } from "../screens/ListCount";
import { StaleSelectionAlert, columnOptionsFor, emptyTableCopy, useResourceTabView } from "../screens/resourceShell";
import { toggleColumn, useHiddenColumns } from "../lib/columnPrefs";

// The six normalized statuses (#541), each counted and listed by its word:
// the legend names every one, zero included, so no colour is the only signal.
const statuses: NormalizedStatus[] = ["healthy", "warning", "error", "progressing", "suspended", "unknown"];
// Unknown is a state, drawn in a readable ink (`--ink-faint`, 5.5:1 or better
// against the surface in every theme) — never `--rule`, the hairline the empty
// ring below is drawn in, which an all-Unknown ring was indistinguishable from.
export const DONUT_COLORS: Record<NormalizedStatus, string> = {
  healthy: "var(--ok, var(--fl-color-success, #388b5d))",
  warning: "var(--warn, var(--fl-color-warning, #bf8e32))",
  error: "var(--sev, var(--fl-color-danger, #d15f54))",
  progressing: "var(--info, var(--fl-color-info, #518dcc))",
  suspended: "var(--ink-muted, var(--fl-color-text-muted, #85818f))",
  unknown: "var(--ink-faint, var(--fl-color-text-muted, #696475))",
};
/** A ring with nothing in it: the hairline, so "none" never reads as a status. */
export const EMPTY_DONUT = "var(--rule, #85818f)";
/** The share of the ring cut from the end of each segment, in percent. */
const GAP = 1.5;
/**
 * The least arc a non-zero status is drawn with, in percent: four gaps, so
 * its colour (the arc less its gap) is three times as wide as the gap beside
 * it — about 16° of the ring, a mark the eye finds, not a hairline.
 */
const MIN_ARC = 4 * GAP;

/**
 * Each drawn status's arc, in percent of the ring, summing to 100.
 *
 * Proportional, except that a non-zero status never gets less than `MIN_ARC`:
 * one error among a thousand rows is what an operator scans the ring for,
 * and a true 0.1% share (smaller than the gap) would not be drawn at all. The
 * space is taken from the larger statuses in proportion to their size. The
 * ring may exaggerate a sliver; the legend's counts never do.
 */
function arcs(shares: number[]): number[] {
  let floored = shares.map(() => false);
  for (;;) {
    const fixed = floored.filter(Boolean).length * MIN_ARC;
    const free = shares.reduce((sum, share, i) => (floored[i] ? sum : sum + share), 0);
    const scaled = shares.map((share, i) => (floored[i] ? MIN_ARC : (share * (100 - fixed)) / free));
    const next = scaled.map((arc, i) => floored[i] || arc < MIN_ARC);
    if (next.every((value, i) => value === floored[i])) return scaled;
    floored = next;
  }
}

/**
 * The ring for these counts. Adjacent segments are parted by a sliver of the
 * surface, inset at each segment's end: Suspended and Unknown are both
 * neutral inks, too close to tell apart by colour alone, and each reads at
 * 3:1 or better against the surface. With `MIN_ARC` at four gaps, every
 * segment keeps a colour three times as wide as its gap.
 */
export function donutBackground(counts: Record<NormalizedStatus, number>): string {
  const total = statuses.reduce((sum, status) => sum + counts[status], 0);
  if (!total) return EMPTY_DONUT;
  const drawn = statuses.filter((status) => counts[status] > 0);
  const gap = drawn.length > 1 ? GAP : 0;
  const widths = arcs(drawn.map((status) => (counts[status] / total) * 100));
  let offset = 0;
  const stops = drawn.flatMap((status, i) => {
    const start = offset;
    // The last arc closes the ring exactly, whatever the rounding.
    offset = i === drawn.length - 1 ? 100 : offset + widths[i];
    const end = offset - gap;
    return gap
      ? [`${DONUT_COLORS[status]} ${start}% ${end}%`, `var(--surface, #ffffff) ${end}% ${offset}%`]
      : [`${DONUT_COLORS[status]} ${start}% ${offset}%`];
  });
  return `conic-gradient(${stops.join(",")})`;
}
function Summary({
  plugin,
  page,
  context,
  clusterId,
  namespaces: selected,
  refresh,
  onPage,
}: {
  plugin: InstalledExtension;
  page: ExtensionPage;
  context: string;
  clusterId: string;
  namespaces: string[];
  refresh: number;
  onPage(id: string): void;
}) {
  const namespaces = readerNamespaces(plugin, page.capability, selected);
  const data = useResource(
    async () => {
      const { answers, failures } = await readAcross(namespaces, (namespace) =>
        readExtension(plugin.manifest.id, plugin.revision, page.capability, context, namespace));
      return { items: answers.flatMap((answer) => answer.items), failures };
    },
    [
      plugin.manifest.id,
      plugin.revision,
      page.capability,
      context,
      namespaces.join("\u0000"),
      refresh,
    ],
  );
  const { reload } = data;
  // Counts follow the reader's kind (#566), re-read in place on every change.
  const live = useLiveReaders({
    plugin, capabilities: [page.capability], context, namespaces, label: `summary:${page.id}`, onChange: data.refresh,
  });
  // Status counts change when an action on one of these resources is accepted.
  useEffect(
    () =>
      onExtensionResourceChanged((changed) => {
        if (
          changed.id === plugin.manifest.id &&
          changed.context === context &&
          changed.capability === page.capability &&
          rowInSelection(changed.namespace, namespaces)
        )
          reload();
      }),
    [plugin.manifest.id, context, page.capability, namespaces.join("\u0000"), reload],
  );
  // The same rows the page's list shows once a collection's name filter narrows it.
  const nameFilter = useCollectionNameFilter(clusterId, readerKind(plugin, page.capability), data.data?.items ?? []);
  // The host's resolved status per row; a page still on the deprecated
  // `statusColumns` is read through the same mapping (`itemStatus`).
  const resolved = itemStatuses(nameFilter.apply(data.data?.items ?? []), page.statusColumns);
  const counts = statuses.map((status) => resolved.filter((found) => found === status).length);
  const total = counts.reduce((a, b) => a + b, 0);
  const ring = donutBackground(
    Object.fromEntries(statuses.map((status, i) => [status, counts[i]])) as Record<NormalizedStatus, number>,
  );
  return (
    <section className="extension-summary" data-stale={live.state === "reconnecting" || undefined}>
      <Button variant="ghost" onClick={() => onPage(page.id)}>
        {page.title}
      </Button>
      {/* In words, where the counts are: a tint or a tooltip alone says nothing to most readers. */}
      <LiveStatus live={live} />
      <LiveNotice live={live} what="counts" />
      <NamespaceFailuresAlert what={plainText(page.title).toLowerCase()} failures={data.data?.failures ?? []} />
      <CollectionNameNotice filter={nameFilter} noun={plainText(page.title).toLowerCase()} />
      {data.status === "error" ? (
        <FailureState error={data.error} onRetry={data.reload} />
      ) : data.status === "loading" ? (
        <LoadingState compact label={`Counting ${plainText(page.title).toLowerCase()}`} />
      ) : !nameFilterAnswered(nameFilter) ? null : (
        <div className="extension-summary-body">
          <div
            className="extension-donut"
            style={{ background: ring }}
            aria-label={`${total} ${page.title}`}
          >
            <span>{total}</span>
          </div>
          <ul>
            {statuses.map((status, i) => (
              <li key={status}>
                <i aria-hidden="true" style={{ background: DONUT_COLORS[status] }} />
                {STATUS_WORD[status]}: {counts[i]}
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}
const eventKey = (event: EventSummary) => `${event.namespace}/${event.name}`;
const EVENT_PINNED = "message";
const EVENT_COLUMNS: Column<EventSummary>[] = [
  {
    key: "type",
    header: "Type",
    render: (event) => {
      const { health, bad } = eventVerdict(event.type);
      return <StatusPill status={event.type || "—"} kind={health} tinted={bad} />;
    },
  },
  { key: EVENT_PINNED, header: "Message", sortable: false, render: (event) => <span className="block max-w-[32rem] truncate" title={event.message}>{event.message}</span> },
  { key: "namespace", header: "Namespace", render: (event) => event.namespace || "—" },
  { key: "object", header: "Involved object", render: (event) => event.object || "—" },
  { key: "source", header: "Source", render: (event) => event.source || "—" },
  { key: "count", header: "Count", align: "end" },
  {
    key: "firstAge",
    header: "Age",
    align: "end",
    getSortValue: (event) => ageSortValue({ created: event.firstCreated, age: event.firstAge }),
    render: (event) => <AgeCell created={event.firstCreated} age={event.firstAge} />,
  },
  { key: "age", header: "Last seen", align: "end", getSortValue: ageSortValue, render: (event) => <AgeCell created={event.created} age={event.age} /> },
];

function Events({
  plugin,
  config,
  context,
  namespaces,
  search,
  regex,
  refresh,
  header,
}: {
  plugin: InstalledExtension;
  config: NonNullable<
    NonNullable<ExtensionPage["dashboard"]>["events"]
  >;
  context: string;
  namespaces: string[];
  search: string;
  regex: boolean;
  refresh: number;
  header: HTMLElement | null;
}) {
  const result = useResource(
    async () => {
      const { answers, failures } = await readAcross(namespaces, (namespace) =>
        readExtension<{ events: EventSummary[]; truncated?: boolean }>(plugin.manifest.id, plugin.revision, config.capability, context, namespace));
      return { events: answers.flatMap((answer) => answer.events ?? []), truncated: answers.some((answer) => answer.truncated), failures };
    },
    [
      plugin.manifest.id,
      plugin.revision,
      config.capability,
      context,
      namespaces.join("\u0000"),
      refresh,
    ],
  );
  const live = useLiveReaders({
    plugin, capabilities: [config.capability], context, namespaces, label: "events", onChange: result.refresh,
  });
  const columnsKey = `extension:${plugin.manifest.id}:${config.capability}`;
  const hidden = useHiddenColumns(columnsKey);
  const columns = useMemo(() => EVENT_COLUMNS.filter((column) => column.key === EVENT_PINNED || !hidden.has(column.key)), [hidden]);
  if (result.status === "error")
    return <FailureState error={result.error} onRetry={result.reload} />;
  if (result.status === "loading") return <LoadingState compact label="Loading events" />;
  const matches = filterMatcher(search, regex);
  const ours = (result.data?.events ?? []).filter((event) => config.apiGroups.includes(event.objectApiVersion?.split("/")[0] ?? ""));
  const events = ours.filter((event) =>
    [event.message, event.object, event.namespace, event.reason, event.source].some((value) => matches(value ?? "")),
  );
  return (
    <section className="extension-results">
      <div className="extension-resource-list">
      <h3 className="extension-message">Events</h3>
      {header && createPortal(
        <>
          <ListCount total={ours.length} shown={events.length} truncated={result.data?.truncated} noun="events" />
          <ColumnPicker columns={columnOptionsFor(EVENT_COLUMNS)} hidden={hidden} onToggle={(key) => toggleColumn(columnsKey, key)} pinnedKey={EVENT_PINNED} />
        </>,
        header,
      )}
      <NamespaceFailuresAlert what="events" failures={result.data?.failures ?? []} />
      <LiveNotice live={live} what="events" />
      {result.data?.truncated && (
        <p className="extension-message" role="status">
          Showing the first {(result.data.events ?? []).length.toLocaleString()} events; more remain on the cluster.
        </p>
      )}
      <div className="extension-table-scroll">
        <Table
          columns={columns}
          data={events}
          getRowKey={eventKey}
          {...(ours.length ? emptyTableCopy(ours.length, "events", "", "") : { emptyText: "No events", emptyHint: "This app's resources have no recent events." })}
        />
      </div>
      </div>
    </section>
  );
}

/** The same host-resolved figures as the cluster overview, following this page's readers. */
function OverviewCards({plugin,context,namespaces,refresh,onOpen}: {
  plugin: InstalledExtension; context: string; namespaces: string[]; refresh: number;
  onOpen(card: ExtensionDashboardCard): void;
}) {
  const [pulse,setPulse] = useState(0);
  const cards = plugin.manifest.contributions.dashboardCards ?? [];
  const live = useLiveReaders({plugin,context,namespaces,capabilities:cards.map(card=>card.source),label:"overview:cards",onChange:()=>setPulse(n=>n+1)});
  useEffect(()=>onExtensionResourceChanged(changed=>{
    if (changed.id===plugin.manifest.id && changed.context===context && cards.some(card=>card.source===changed.capability) && rowInSelection(changed.namespace,namespaces)) setPulse(n=>n+1);
  }),[plugin.manifest.id,context,namespaces.join("\u0000"),cards.map(card=>card.source).join("\u0000")]);
  return <>
    <div className="extension-overview-live"><LiveStatus live={live}/><LiveNotice live={live} what="figures"/></div>
    <AppCards plugin={plugin} host={context} selection={namespaces} refresh={refresh} pulse={pulse} cards={cards} onOpen={onOpen} previewLists/>
  </>;
}

/** Native, data-only extension workspace. Every reader stays pinned to this route's context. */
export function ExtensionWorkspace({
  plugin,
  page,
  cluster,
  context,
  route,
  actions,
  namespace: cardNamespace = "",
  onPage,
  onCard,
  card,
  cardNamespaces,
}: {
  plugin: InstalledExtension;
  page: ExtensionPage;
  /** The cluster this route names, whose namespaces the tab has selected. */
  cluster: ClusterContext;
  /** Its pinned ID, which the host is asked by. */
  context: string;
  /** This tab's route, which its filter is kept with. */
  route: string;
  /** Where the page's header actions go, in the screen's header. */
  actions: HTMLElement | null;
  /** On a card's target: the one namespace it counted in. */
  namespace?: string;
  onPage(id: string): void;
  onCard(id: string, namespaces: string[], card: string): void;
  /** A dashboard card whose rows the page shows (#540); the whole list when absent. */
  card?: string;
  /** The several namespaces the card counted in, when it counted in more than one. */
  cardNamespaces?: string[];
}) {
  const view = useResourceTabView(route, []);
  const [refresh, setRefresh] = useState(0);
  const namespaceOptions = useNamespaceOptions(context, loadKubeconfigFiles(), refresh);
  const {namespaces: listed, scope, error:namespaceError} = namespaceOptions;
  const selection = useNamespaces(cluster.stableId);
  const setNamespaces = useSetNamespaces();
  useFallbackNamespace(cluster.stableId, scope);
  const scopedByCollection = useInCollectionScope(cluster.stableId);
  const [count, setCount] = useState<HTMLElement | null>(null);
  const [bulkBar, setBulkBar] = useState<HTMLElement | null>(null);
  // A card's target reads exactly the namespaces the card counted in — one (in
  // the route's path), several (its list) or every one — and nothing else: its
  // route, not a restricted credential's one namespace or the picker, says what
  // the page shows. A credential that cannot read them gets the host's refusal
  // for the card's scope, never quietly another namespace's rows.
  const namespaces = card
    ? cardNamespace ? [cardNamespace] : cardNamespaces ?? []
    : scope && !scopedByCollection ? [scope] : selection;
  const pages = plugin.manifest.contributions.pages;
  const cards = plugin.manifest.contributions.dashboardCards ?? [];
  // Every reader this page shows. A dashboard's own capability reads nothing of it.
  const readers = page.dashboard
    ? [
        ...pages.filter((p) => page.dashboard!.pages.includes(p.id)).map((p) => p.capability),
        ...(page.dashboard.events ? [page.dashboard.events.capability] : []),
        ...cards.map((card) => card.source),
      ]
    : [page.capability];
  const clusterScoped = readers.every((reader) => readsClusterScope(plugin, reader));
  const groups = [...new Set(pages.map((p) => p.group ?? p.title))];
  const openCard = (opened: ExtensionDashboardCard) => {
    if (opened.target) onCard(opened.target.page, namespaces, opened.id);
  };
  if (!context) return <EmptyState title="No cluster in focus" hint="Pick a cluster in the rail to open this app page." />;
  const noun = page.dashboard ? "events" : plainText(page.title).toLowerCase();
  const scopeControl = card
    ? <span className="extension-card-scope">{cardNamespace || cardNamespaces?.join(", ") || "All namespaces"}</span>
    : !clusterScoped && <NamespaceScopePicker context={cluster} options={namespaceOptions} />;
  return (
    // One view (#566): every reader this page shows is watched once, and the
    // watches end with the page.
    <LiveReaders plugin={plugin} label={`page:${page.id}`}>
    {actions && createPortal(
      <>
        <span ref={setCount} className="contents" />
        <Button variant="secondary" onClick={() => setRefresh((v) => v + 1)}>
          Refresh
        </Button>
      </>,
      actions,
    )}
    <div className="extension-workspace">
      {/* A page's tabs scroll sideways rather than wrapping, as the built-in tab strip does. */}
      <nav aria-label={`${plugin.manifest.name} pages`}>
        <Toolbar className="overflow-x-auto">
          <ExtensionLogo icon={plugin.icon} name={plugin.manifest.name} />
          <Tabs
            variant="underline"
            activation="manual"
            label={`${plugin.manifest.name} sections`}
            tabs={groups.map((group) => ({ id: group, label: group }))}
            active={page.group ?? page.title}
            onChange={(group) => onPage(pages.find((p) => (p.group ?? p.title) === group)!.id)}
          />
        </Toolbar>
      </nav>
      {page.group && (
        <nav aria-label={`${page.group} pages`}>
          <Toolbar className="extension-subnavigation overflow-x-auto">
            <Tabs
              variant="underline"
              activation="manual"
              label={`${page.group} pages`}
              tabs={pages.filter((p) => p.group === page.group).map((p) => ({ id: p.id, label: p.title }))}
              active={page.id}
              onChange={onPage}
            />
          </Toolbar>
        </nav>
      )}
      <ExtensionRequirements plugin={plugin} page={page} context={context} refresh={refresh}>
      {!page.dashboard || page.dashboard.events ? (
        <FilterBar
          value={view.filter}
          onValueChange={view.setFilter}
          regex={view.regex}
          onRegexChange={view.setRegex}
          invalid={tableFilterError(view.filter, view.regex) !== null}
          label={`Filter ${noun}`}
          placeholder={`Filter ${noun}…`}
          leading={<span ref={setBulkBar} className="contents" />}
        >
          {scopeControl}
        </FilterBar>
      ) : scopeControl && <Toolbar wrap>{scopeControl}</Toolbar>}
      {namespaceError && (
        <FailureAlert tone="sev" title="Could not list namespaces" error={namespaceError} onRetry={()=>setRefresh(v=>v+1)} />
      )}
      {!card && !clusterScoped && (
        <>
          <CollectionScopeNotice context={cluster} />
          <StaleSelectionAlert selection={selection} namespaces={listed} onReset={() => setNamespaces(cluster.stableId, [])} />
        </>
      )}
      {listed === null ? <LoadingState label="Loading namespaces" /> : page.dashboard ? (
        <>
          <div className={cards.length ? "extension-overview" : "extension-summaries"}>
            {page.dashboard.pages.map((id,index) => {
              const p = pages.find(p => p.id === id);
              return p && <Fragment key={id}>
                <Summary plugin={plugin} page={p} context={context} clusterId={cluster.stableId} namespaces={namespaces} refresh={refresh} onPage={onPage}/>
                {index === 0 && cards.length > 0 && <OverviewCards plugin={plugin} context={context} namespaces={namespaces} refresh={refresh} onOpen={openCard}/>}
              </Fragment>;
            })}
          </div>
          {page.dashboard.events && (
            <Events
              plugin={plugin}
              config={page.dashboard.events}
              context={context}
              namespaces={namespaces}
              search={view.filter}
              regex={view.regex}
              refresh={refresh}
              header={count}
            />
          )}
        </>
      ) : (
        <ExtensionResults
          plugin={plugin}
          capability={page.capability}
          context={context}
          clusterId={cluster.stableId}
          namespaces={namespaces}
          search={view.filter}
          regex={view.regex}
          refresh={refresh}
          hideToolbar
          card={card}
          noun={noun}
          header={count}
          bulkBar={bulkBar}
        />
      )}
      </ExtensionRequirements>
    </div>
    </LiveReaders>
  );
}
