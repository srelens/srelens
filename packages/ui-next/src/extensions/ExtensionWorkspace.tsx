import { useContext, useState } from "react";
import {
  listNamespaces,
  readExtension,
  type ExtensionContribution,
  type InstalledExtension,
  type EventSummary,
} from "@srelens/core";
import { ExtensionControls } from "./ExtensionControls";
import { ErrorNotice, ExtensionResults } from "./ExtensionResults";
import { useResource } from "../lib/useResource";

const statuses = [
  "Ready",
  "Not ready",
  "In progress",
  "Suspended",
  "Unknown",
] as const;
type Status = (typeof statuses)[number];
export function resourceStatus(
  values: string[],
  columns: NonNullable<ExtensionContribution["statusColumns"]>,
): Status {
  const truth = (index?: number) =>
    index !== undefined && values[index]?.toLowerCase() === "true";
  if (truth(columns.suspended)) return "Suspended";
  if (truth(columns.progressing)) return "In progress";
  const ready = values[columns.ready]?.toLowerCase();
  return ready === "true"
    ? "Ready"
    : ready === "false"
      ? "Not ready"
      : "Unknown";
}
const colors = [
  "var(--ok, var(--fl-color-success, #388b5d))",
  "var(--sev, var(--fl-color-danger, #d15f54))",
  "var(--warn, var(--fl-color-warning, #bf8e32))",
  "var(--info, var(--fl-color-info, #518dcc))",
  "var(--ink-muted, var(--fl-color-text-muted, #85818f))",
];
function Summary({
  plugin,
  page,
  context,
  namespace,
  refresh,
  onPage,
}: {
  plugin: InstalledExtension;
  page: ExtensionContribution;
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
  const counts = statuses.map(
    (status) =>
      (data.data?.items ?? []).filter(
        (row) => resourceStatus(row.columns, page.statusColumns!) === status,
      ).length,
  );
  const total = counts.reduce((a, b) => a + b, 0);
  let offset = 0;
  const gradient = counts
    .map((count, i) => {
      const start = offset;
      offset += total ? (count / total) * 100 : 0;
      return `${colors[i]} ${start}% ${offset}%`;
    })
    .join(",");
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
            style={{
              background: total
                ? `conic-gradient(${gradient})`
                : "var(--rule, #85818f)",
            }}
            aria-label={`${total} ${page.title}`}
          >
            <span>{total}</span>
          </div>
          <ul>
            {statuses.map((status, i) => (
              <li key={status}>
                <i aria-hidden="true" style={{ background: colors[i] }} />
                {status}: {counts[i]}
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
    NonNullable<ExtensionContribution["dashboard"]>["events"]
  >;
  context: string;
  namespace: string;
  search: string;
  refresh: number;
}) {
  const result = useResource(
    () =>
      readExtension<{ events: EventSummary[] }>(
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
  return (
    <section className="extension-results">
      <h3 className="extension-message">
        Events <small>({events.length})</small>
      </h3>
      {events.length ? (
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
              {events.map((e) => (
                <tr key={e.name}>
                  <td>{e.type}</td>
                  <td title={e.message}>{e.message}</td>
                  <td>{e.namespace}</td>
                  <td>{e.object}</td>
                  <td>{e.source || "—"}</td>
                  <td>{e.count}</td>
                  <td>{e.firstAge || "—"}</td>
                  <td>{e.age}</td>
                </tr>
              ))}
            </tbody>
          </table>
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
}: {
  plugin: InstalledExtension;
  page: ExtensionContribution;
  context: string;
  namespace?: string;
  onPage?(id: string): void;
}) {
  const { Button, Combobox } = useContext(ExtensionControls);
  const [localPage, setLocalPage] = useState(page.id);
  const [namespace, setNamespace] = useState(initialNamespace);
  const [search, setSearch] = useState("");
  const [refresh, setRefresh] = useState(0);
  const namespaces = useResource(
    () => (context ? listNamespaces(context) : Promise.resolve(null)),
    [context],
  );
  const current = onPage
    ? page
    : (plugin.manifest.contributions.pages.find((p) => p.id === localPage) ??
      page);
  const navigate = (id: string) => {
    setSearch("");
    if (onPage) onPage(id);
    else setLocalPage(id);
  };
  const pages = plugin.manifest.contributions.pages;
  const groups = [...new Set(pages.map((p) => p.group ?? p.title))];
  if (!context)
    return (
      <p className="extension-message">
        Choose a cluster before opening an extension page.
      </p>
    );
  return (
    <div className="extension-workspace">
      <nav
        className="extension-toolbar extension-navigation"
        aria-label={`${plugin.manifest.name} pages`}
      >
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
      <div className="extension-toolbar extension-filters">
        <Combobox
          ariaLabel="Extension namespace"
          value={namespace}
          onValueChange={setNamespace}
          options={[
            { value: "", label: "All namespaces" },
            ...(namespaces.data && "namespaces" in namespaces.data
              ? (namespaces.data.namespaces ?? []).map((n) => ({
                  value: n,
                  label: n,
                }))
              : []),
          ]}
          placeholder={
            namespaces.status === "loading"
              ? "Loading namespaces…"
              : "Namespace"
          }
        />
        <input
          className="extension-search"
          aria-label="Search extension resources"
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
      {namespaces.data && "error" in namespaces.data && (
        <ErrorNotice
          cluster
          message={namespaces.data.error}
          retry={namespaces.reload}
        />
      )}
      {current.dashboard ? (
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
    </div>
  );
}
