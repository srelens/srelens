import { ExtensionLogo } from "./ExtensionLogo";
import { ExtensionRequirements } from "./ExtensionRequirements";
import { AgeCell } from "../lib/ageCell";
import { useNamespaceOptions } from "@srelens/core/react";
import { useContext, useEffect, useState } from "react";
import {
  loadKubeconfigFiles,
  onExtensionResourceChanged,
  readExtension,
  itemStatuses,
  type ExtensionPage,
  type InstalledExtension,
  type EventSummary,
  type NormalizedStatus,
} from "@srelens/core";
import { STATUS_WORD } from "./StatusBadge";
import { ExtensionControls } from "./ExtensionControls";
import { ErrorNotice, ExtensionResults } from "./ExtensionResults";
import { useResource } from "../lib/useResource";

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
 * The ring for these counts. Adjacent segments are parted by a sliver of the
 * surface: Suspended and Unknown are both neutral inks, too close to tell
 * apart by colour alone, and each reads at 3:1 or better against the surface.
 */
export function donutBackground(counts: Record<NormalizedStatus, number>): string {
  const total = statuses.reduce((sum, status) => sum + counts[status], 0);
  if (!total) return EMPTY_DONUT;
  const drawn = statuses.filter((status) => counts[status] > 0);
  const gap = drawn.length > 1 ? GAP : 0;
  let offset = 0;
  const stops = drawn.flatMap((status) => {
    const start = offset;
    offset += (counts[status] / total) * 100;
    const end = Math.max(start, offset - gap);
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
  namespace,
  refresh,
  onPage,
}: {
  plugin: InstalledExtension;
  page: ExtensionPage;
  context: string;
  namespace: string;
  refresh: number;
  onPage(id: string): void;
}) {
  const { Button } = useContext(ExtensionControls);
  const data = useResource(
    () =>
      readExtension(
        plugin.manifest.id,
        plugin.revision,
        page.capability,
        context,
        namespace,
      ),
    [
      plugin.manifest.id,
      plugin.revision,
      page.capability,
      context,
      namespace,
      refresh,
    ],
  );
  const { reload } = data;
  // Status counts change when an action on one of these resources is accepted.
  useEffect(
    () =>
      onExtensionResourceChanged((changed) => {
        if (
          changed.id === plugin.manifest.id &&
          changed.context === context &&
          changed.capability === page.capability &&
          (!namespace || changed.namespace === namespace)
        )
          reload();
      }),
    [plugin.manifest.id, context, page.capability, namespace, reload],
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
    <section className="extension-summary">
      <Button variant="ghost" onClick={() => onPage(page.id)}>
        {page.title}
      </Button>
      {data.status === "error" ? (
        <ErrorNotice cluster message={data.error} retry={data.reload} />
      ) : data.status === "loading" ? (
        <p role="status">Loading…</p>
      ) : (
        <>
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
        </>
      )}
    </section>
  );
}
function Events({
  plugin,
  config,
  context,
  namespace,
  search,
  refresh,
}: {
  plugin: InstalledExtension;
  config: NonNullable<
    NonNullable<ExtensionPage["dashboard"]>["events"]
  >;
  context: string;
  namespace: string;
  search: string;
  refresh: number;
}) {
  const result = useResource(
    () =>
      readExtension<{ events: EventSummary[]; truncated?: boolean }>(
        plugin.manifest.id,
        plugin.revision,
        config.capability,
        context,
        namespace,
      ),
    [
      plugin.manifest.id,
      plugin.revision,
      config.capability,
      context,
      namespace,
      refresh,
    ],
  );
  // Bound how many matching rows enter the DOM; Load more reveals the next page
  // of the already-capped backend result (#609).
  const PAGE = 100;
  const [visible, setVisible] = useState(PAGE);
  useEffect(() => {
    setVisible(PAGE);
  }, [plugin.manifest.id, plugin.revision, config.capability, context, namespace, search, refresh]);
  if (result.status === "error")
    return <ErrorNotice cluster message={result.error} retry={result.reload} />;
  if (result.status === "loading")
    return (
      <p className="extension-message" role="status">
        Loading events…
      </p>
    );
  const events = (result.data?.events ?? []).filter(
    (event) =>
      config.apiGroups.includes(event.objectApiVersion?.split("/")[0] ?? "") &&
      [event.message, event.object, event.namespace, event.reason, event.source]
        .join(" ")
        .toLowerCase()
        .includes(search.toLowerCase()),
  );
  const shown = events.slice(0, visible);
  const hidden = Math.max(0, events.length - shown.length);
  return (
    <section className="extension-results">
      <h3 className="extension-message">
        Events <small>({events.length}{result.data?.truncated ? "+" : ""})</small>
      </h3>
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
                <tr key={e.name}>
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

/** Native, data-only extension workspace. Every reader stays pinned to this route's context. */
export function ExtensionWorkspace({
  plugin,
  page,
  context,
  namespace: initialNamespace = "",
  onPage,
  onNamespace,
}: {
  plugin: InstalledExtension;
  page: ExtensionPage;
  context: string;
  namespace?: string;
  onPage?(id: string, namespace: string): void;
  onNamespace?(namespace: string): void;
}) {
  const { Button, Combobox } = useContext(ExtensionControls);
  const [localPage, setLocalPage] = useState(page.id);
  const [selectedNamespace, setNamespace] = useState(initialNamespace);
  const [search, setSearch] = useState("");
  const [refresh, setRefresh] = useState(0);
  const {namespaces, scope, error:namespaceError} = useNamespaceOptions(context, loadKubeconfigFiles(), refresh);
  const namespace = scope || selectedNamespace;
  const current = onPage
    ? page
    : (plugin.manifest.contributions.pages.find((p) => p.id === localPage) ??
      page);
  const navigate = (id: string) => {
    setSearch("");
    if (onPage) onPage(id, namespace);
    else setLocalPage(id);
  };
  const pages = plugin.manifest.contributions.pages;
  const groups = [...new Set(pages.map((p) => p.group ?? p.title))];
  if (!context)
    return (
      <p className="extension-message">
        Choose a cluster before opening an app page.
      </p>
    );
  return (
    <div className="extension-workspace">
      <nav
        className="extension-toolbar extension-navigation"
        aria-label={`${plugin.manifest.name} pages`}
      >
        <ExtensionLogo id={plugin.manifest.id} name={plugin.manifest.name} />
        {groups.map((group) => (
          <Button
            key={group}
            variant="ghost"
            aria-pressed={(current.group ?? current.title) === group}
            onClick={() =>
              navigate(pages.find((p) => (p.group ?? p.title) === group)!.id)
            }
          >
            {group}
          </Button>
        ))}
      </nav>
      {current.group && (
        <nav
          className="extension-toolbar extension-navigation extension-subnavigation"
          aria-label={`${current.group} pages`}
        >
          {pages
            .filter((p) => p.group === current.group)
            .map((p) => (
              <Button
                key={p.id}
                variant="ghost"
                aria-pressed={p.id === current.id}
                onClick={() => navigate(p.id)}
              >
                {p.title}
              </Button>
            ))}
        </nav>
      )}
      <ExtensionRequirements plugin={plugin} page={current} context={context} refresh={refresh}>
      <div className="extension-toolbar extension-filters">
        {namespaces === null ? <Button variant="secondary" disabled>Loading namespaces…</Button> : <Combobox
          ariaLabel="App namespace"
          value={namespace}
          onValueChange={(value) => { setNamespace(value); onNamespace?.(value); }}
          options={[
            ...(scope ? [] : [{ value: "", label: "All namespaces" }]),
            ...(namespaces ?? []).map(n=>({value:n,label:n})),
          ]}
          placeholder={
            namespaces === null
              ? "Loading namespaces…"
              : "Namespace"
          }
        />}
        <input
          className="extension-search"
          aria-label="Search app resources"
          placeholder={
            current.dashboard ? "Search events…" : "Search resources…"
          }
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <Button variant="secondary" onClick={() => setRefresh((v) => v + 1)}>
          Refresh
        </Button>
      </div>
      {namespaceError && (
        <ErrorNotice
          cluster
          message={namespaceError}
          retry={()=>setRefresh(v=>v+1)}
        />
      )}
      {namespaces === null ? <p role="status" className="extension-message">Loading namespaces…</p> : current.dashboard ? (
        <>
          <div className="extension-summaries">
            {current.dashboard.pages.map((id) => {
              const p = pages.find((p) => p.id === id);
              return (
                p && (
                  <Summary
                    key={id}
                    plugin={plugin}
                    page={p}
                    context={context}
                    namespace={namespace}
                    refresh={refresh}
                    onPage={navigate}
                  />
                )
              );
            })}
          </div>
          {current.dashboard.events && (
            <Events
              plugin={plugin}
              config={current.dashboard.events}
              context={context}
              namespace={namespace}
              search={search}
              refresh={refresh}
            />
          )}
        </>
      ) : (
        <ExtensionResults
          plugin={plugin}
          capability={current.capability}
          context={context}
          namespace={namespace}
          search={search}
          refresh={refresh}
          hideToolbar
        />
      )}
      </ExtensionRequirements>
    </div>
  );
}
