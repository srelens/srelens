import { ExtensionLogo } from "./ExtensionLogo";
import { ExtensionRequirements } from "./ExtensionRequirements";
import { AgeCell } from "../lib/ageCell";
import { useNamespaceOptions } from "@srelens/core/react";
import { Fragment, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import {
  loadKubeconfigFiles,
  onExtensionResourceChanged,
  readExtension,
  itemStatuses,
  rowInSelection,
  watchNamespaceForSelection,
  type ClusterContext,
  type ExtensionDashboardCard,
  type ExtensionPage,
  type InstalledExtension,
  type EventSummary,
  type NormalizedStatus,
} from "@srelens/core";
import { AppCards } from "./DashboardCards";
import { plainText } from "./displayText";
import { STATUS_WORD } from "./StatusBadge";
import { Button, FilterBar, Tabs, filterMatcher, tableFilterError } from "@srelens/ui-kit";
import { ErrorNotice, ExtensionResults } from "./ExtensionResults";
import { useResource } from "../lib/useResource";
import { LiveNotice, LiveReaders, LiveStatus, useLiveReaders } from "./liveReaders";
import { readAcross } from "./readAcross";
import { NamespaceFailuresAlert } from "../lib/errorCopy";
import { useFallbackNamespace, useInCollectionScope, useNamespaces, useSetNamespaces } from "../lib/workspace";
import { CollectionScopeNotice, NamespaceScopePicker } from "../screens/collections/NamespaceScope";
import { ListCount } from "../screens/ListCount";
import { StaleSelectionAlert, useResourceTabView } from "../screens/resourceShell";

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
  namespaces,
  refresh,
  onPage,
}: {
  plugin: InstalledExtension;
  page: ExtensionPage;
  context: string;
  namespaces: string[];
  refresh: number;
  onPage(id: string): void;
}) {
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
    plugin, capabilities: [page.capability], context, namespace: watchNamespaceForSelection(namespaces), label: `summary:${page.id}`, onChange: data.refresh,
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
  // The host's resolved status per row; a page still on the deprecated
  // `statusColumns` is read through the same mapping (`itemStatus`).
  const resolved = itemStatuses(data.data?.items ?? [], page.statusColumns);
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
      {data.status === "error" ? (
        <ErrorNotice cluster message={data.error} retry={data.reload} />
      ) : data.status === "loading" ? (
        <p role="status">Loading…</p>
      ) : (
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
    plugin, capabilities: [config.capability], context, namespace: watchNamespaceForSelection(namespaces), label: "events", onChange: result.refresh,
  });
  // Bound how many matching rows enter the DOM; Load more reveals the next page
  // of the already-capped backend result (#609).
  const PAGE = 100;
  const [visible, setVisible] = useState(PAGE);
  useEffect(() => {
    setVisible(PAGE);
  }, [plugin.manifest.id, plugin.revision, config.capability, context, namespaces.join("\u0000"), search, regex, refresh]);
  if (result.status === "error")
    return <ErrorNotice cluster message={result.error} retry={result.reload} />;
  if (result.status === "loading")
    return (
      <p className="extension-message" role="status">
        Loading events…
      </p>
    );
  const matches = filterMatcher(search, regex);
  const ours = (result.data?.events ?? []).filter((event) => config.apiGroups.includes(event.objectApiVersion?.split("/")[0] ?? ""));
  const events = ours.filter((event) =>
    [event.message, event.object, event.namespace, event.reason, event.source].some((value) => matches(value ?? "")),
  );
  const shown = events.slice(0, visible);
  const hidden = Math.max(0, events.length - shown.length);
  return (
    <section className="extension-results">
      <h3 className="extension-message">Events</h3>
      {header && createPortal(<ListCount total={ours.length} shown={events.length} truncated={result.data?.truncated} noun="events" />, header)}
      <NamespaceFailuresAlert what="events" failures={result.data?.failures ?? []} />
      <LiveNotice live={live} what="events" />
      {result.data?.truncated && (
        <p className="extension-message" role="status">
          Showing the first {(result.data.events ?? []).length.toLocaleString()} events; more remain on the cluster.
        </p>
      )}
      {shown.length ? (
        <div className="extension-table-scroll">
          <table>
            <thead>
              <tr>
                {[
                  "Type",
                  "Message",
                  "Namespace",
                  "Involved object",
                  "Source",
                  "Count",
                  "Age",
                  "Last seen",
                ].map((h) => (
                  <th key={h}>{h}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {shown.map((e) => (
                <tr key={`${e.namespace}/${e.name}`}>
                  <td>{e.type}</td>
                  <td title={e.message}>{e.message}</td>
                  <td>{e.namespace}</td>
                  <td>{e.object}</td>
                  <td>{e.source || "—"}</td>
                  <td>{e.count}</td>
                  <td><AgeCell created={e.firstCreated} age={e.firstAge} /></td>
                  <td><AgeCell created={e.created} age={e.age} /></td>
                </tr>
              ))}
            </tbody>
          </table>
          {hidden > 0 && (
            <p className="extension-message">
              <button type="button" className="extension-resource-link" onClick={() => setVisible((n) => n + PAGE)}>
                Show {Math.min(PAGE, hidden).toLocaleString()} more
              </button>
              <span> · {hidden.toLocaleString()} matching rows not shown</span>
            </p>
          )}
        </div>
      ) : (
        <p className="extension-message">No matching events.</p>
      )}
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
  const live = useLiveReaders({plugin,context,namespace:watchNamespaceForSelection(namespaces),capabilities:cards.map(card=>card.source),label:"overview:cards",onChange:()=>setPulse(n=>n+1)});
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
  const clusterScoped = plugin.manifest.capabilities.find((binding) => binding.name === page.capability)?.arguments.namespaced === false;
  // A card's target reads exactly the namespaces the card counted in — one (in
  // the route's path), several (its list) or every one — and nothing else: its
  // route, not a restricted credential's one namespace or the picker, says what
  // the page shows. A credential that cannot read them gets the host's refusal
  // for the card's scope, never quietly another namespace's rows.
  const namespaces = card
    ? cardNamespace ? [cardNamespace] : cardNamespaces ?? []
    : clusterScoped ? [] : scope && !scopedByCollection ? [scope] : selection;
  const pages = plugin.manifest.contributions.pages;
  const cards = plugin.manifest.contributions.dashboardCards ?? [];
  const groups = [...new Set(pages.map((p) => p.group ?? p.title))];
  const openCard = (opened: ExtensionDashboardCard) => {
    if (opened.target) onCard(opened.target.page, namespaces, opened.id);
  };
  if (!context)
    return (
      <p className="extension-message">
        Choose a cluster before opening an app page.
      </p>
    );
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
      <nav
        className="extension-toolbar extension-navigation"
        aria-label={`${plugin.manifest.name} pages`}
      >
        <ExtensionLogo icon={plugin.icon} name={plugin.manifest.name} />
        <Tabs
          variant="underline"
          activation="manual"
          label={`${plugin.manifest.name} sections`}
          tabs={groups.map((group) => ({ id: group, label: group }))}
          active={page.group ?? page.title}
          onChange={(group) => onPage(pages.find((p) => (p.group ?? p.title) === group)!.id)}
        />
      </nav>
      {page.group && (
        <nav
          className="extension-toolbar extension-navigation extension-subnavigation"
          aria-label={`${page.group} pages`}
        >
          <Tabs
            variant="underline"
            activation="manual"
            label={`${page.group} pages`}
            tabs={pages.filter((p) => p.group === page.group).map((p) => ({ id: p.id, label: p.title }))}
            active={page.id}
            onChange={onPage}
          />
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
      ) : scopeControl && <div className="extension-toolbar extension-scope">{scopeControl}</div>}
      {namespaceError && (
        <ErrorNotice
          cluster
          message={namespaceError}
          retry={()=>setRefresh(v=>v+1)}
        />
      )}
      {!card && !clusterScoped && (
        <>
          <CollectionScopeNotice context={cluster} />
          <StaleSelectionAlert selection={selection} namespaces={listed} onReset={() => setNamespaces(cluster.stableId, [])} />
        </>
      )}
      {listed === null ? <p role="status" className="extension-message">Loading namespaces…</p> : page.dashboard ? (
        <>
          <div className={cards.length ? "extension-overview" : "extension-summaries"}>
            {page.dashboard.pages.map((id,index) => {
              const p = pages.find(p => p.id === id);
              return p && <Fragment key={id}>
                <Summary plugin={plugin} page={p} context={context} namespaces={namespaces} refresh={refresh} onPage={onPage}/>
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
