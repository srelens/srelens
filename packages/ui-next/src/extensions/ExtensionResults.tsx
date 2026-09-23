import { AgeCell } from "../lib/ageCell";
import { ExtensionResourceDetails } from "./ExtensionResourceDetails";
import { ExtensionBulkActions, type BulkActionAvailability } from "./ExtensionBulkActions";
import { bulkResourceKey } from "./bulkActions";
import { Checkbox, ResizeHandle } from "@srelens/ui-kit";
import { clampPeekWidth, savePeekWidth, setPeekWidth, usePeekBounds, usePeekWidth } from "../lib/peekWidth";
import { ExtensionResourceNavigation } from "./resourceNavigation";
import { useContext, useEffect, useRef, useState } from "react";
import {
  describeError,
  onExtensionResourceChanged,
  readExtension,
  type ExtensionResourceResult,
  type InstalledExtension,
} from "@srelens/core";
import { ExtensionControls } from "./ExtensionControls";
import { useResource } from "../lib/useResource";
import { useResolvedColumns } from "./useResolvedColumns";
import { contributionKind } from "@srelens/core";
import { useContextId } from "./contextIds";

export function ErrorNotice({
  message,
  retry,
  cluster = false,
  title,
  guidance,
}: {
  message?: string;
  retry: () => void;
  cluster?: boolean;
  /** Names what failed, keeping the reason `describeError` gives. */
  title?: string;
  guidance?: { title: string; detail: string };
}) {
  const { Button } = useContext(ExtensionControls);
  const error = describeError(message, {
    domain: cluster ? "cluster" : "local",
  });
  return (
    <div className="extension-error" role="alert">
      <div>
        <strong>{guidance?.title ?? title ?? error.title}</strong>
        <p>{guidance?.detail ?? error.detail}</p>
        {error.raw !== (guidance?.detail ?? error.detail) && (
          <details>
            <summary>Original error</summary>
            <pre>{error.raw}</pre>
          </details>
        )}
      </div>
      <Button variant="secondary" onClick={retry}>
        Retry
      </Button>
    </div>
  );
}
function ResultValue({ value, column }: { value?: string; column: string }) {
  const raw = value || "—";
  const boolean = raw.toLowerCase();
  let label = raw;
  let tone = "neutral";
  if (column === "Ready" && (boolean === "true" || boolean === "false")) {
    label = boolean === "true" ? "Ready" : "Not ready";
    tone = boolean === "true" ? "ready" : "warning";
  } else if (
    ["Suspended", "Reconciling"].includes(column) &&
    ["true", "false"].includes(boolean)
  ) {
    label =
      boolean === "false"
        ? "No"
        : column === "Reconciling"
          ? "In progress"
          : "Yes";
    tone = boolean === "true" ? "warning" : "muted";
  }
  const revision = raw.match(/^(.*?)@sha(?:1|256):([a-f0-9]{16,})$/i);
  if (revision) label = `${revision[1]}@${revision[2].slice(0, 8)}`;
  return (
    <span
      className="extension-value"
      data-tone={tone}
      title={raw}
      aria-label={revision ? raw : undefined}
    >
      {label}
    </span>
  );
}

export function ExtensionResults({
  plugin,
  capability,
  context,
  namespace = "",
  search = "",
  refresh = 0,
  hideToolbar = false,
  actionAvailability,
  card,
}: {
  plugin: InstalledExtension;
  capability: string;
  context: string;
  namespace?: string;
  search?: string;
  refresh?: number;
  hideToolbar?: boolean;
  /**
   * Optional availability override for the embedding surface. By default the
   * bulk bar evaluates the shared predicates against inspected resources.
   */
  actionAvailability?: BulkActionAvailability;
  /** A dashboard card whose rows alone are read (#540). */
  card?: string;
}) {
  const { Button } = useContext(ExtensionControls);
  const openResource = useContext(ExtensionResourceNavigation);
  const rowButtons = useRef(new Map<string,HTMLButtonElement>());
  const listRow = usePeekBounds();
  const peekWidth = clampPeekWidth(usePeekWidth(), listRow.bounds);
  // The card is part of what is on screen: the filtered and whole lists keep no rows of each other.
  const scope = JSON.stringify([plugin.manifest.id,plugin.revision,capability,context,namespace,card ?? ""]);
  const [selected,setSelected] = useState<{scope:string;name:string;namespace:string}|null>(null);
  const [columnSort, setColumnSort] = useState<{key:string;direction:"asc"|"desc"}|null>(null);
  useEffect(() => setColumnSort(null), [scope]);
  // The rows a bulk action would run against, by key. Cleared whenever the
  // scope moves: a rail switch behind the bar must not leave prod's rows
  // selected on staging, and a key from another namespace's list resolves to
  // no row here — a count the bar could not act on.
  const [picked,setPicked] = useState<Set<string>>(new Set());
  useEffect(()=>{setPicked(new Set());},[scope]);
  // Whether this scope has answered once. A new scope starts over: its first
  // read has nothing to keep on screen and everything on screen belongs to a
  // cluster or namespace the reader has left.
  const loaded = useRef(false);
  // The rows of the last answered read, kept across a refresh. `useResource`
  // drops its data the moment a reload starts, and a selection the reader made
  // is resolved back to rows — so without this, every refresh emptied the
  // selection for as long as the read was out, which is exactly while a bulk
  // action's own accepted writes are refreshing the list.
  const lastRows = useRef<ExtensionResourceResult["items"]>([]);
  useEffect(()=>{loaded.current=false;lastRows.current=[];},[scope]);
  const data = useResource(
    async () =>
      context
        ? readExtension(
            plugin.manifest.id,
            plugin.revision,
            capability,
            context,
            namespace,
            true,
            // Only a card's target narrows the read; the whole list is called as before.
            ...(card ? [card] : []),
          )
        : null,
    [
      plugin.manifest.id,
      plugin.revision,
      capability,
      context,
      namespace,
      refresh,
      card,
    ],
  );
  const { reload } = data;
  // Refresh when an action on one of this list's resources is accepted, from any view.
  useEffect(
    () =>
      onExtensionResourceChanged((changed) => {
        if (
          changed.id === plugin.manifest.id &&
          changed.context === context &&
          changed.capability === capability &&
          (!namespace || changed.namespace === namespace)
        )
          reload();
      }),
    [plugin.manifest.id, context, capability, namespace, reload],
  );
  const binding = plugin.manifest.capabilities.find(
    (b) => b.name === capability,
  );
  if (data.data?.items) lastRows.current = data.data.items;
  const sourceRows = data.data?.items ?? lastRows.current;
  const columnKind = contributionKind(
    typeof binding?.arguments.kind === "string" ? binding.arguments.kind : "",
    typeof binding?.arguments.group === "string" ? binding.arguments.group : "",
  );
  const contextId = useContextId(context);
  const appColumns = useResolvedColumns({
    plugins: [plugin], context, contextId, namespace, kind: columnKind, rows: sourceRows, refresh,
  });
  const columns = data.data?.printerColumns ?? (Array.isArray(binding?.arguments.printerColumns)
    ? (binding.arguments.printerColumns as Array<{ name: string }>)
    : []);
  // Bound how many matching rows enter the DOM; Load more reveals the next page
  // of the already-capped backend result (#609).
  const PAGE = 100;
  const [visible, setVisible] = useState(PAGE);
  useEffect(() => {
    setVisible(PAGE);
  }, [scope, search, refresh, data.status, columnSort]);
  useEffect(() => {
    if (data.status !== "loading") loaded.current = true;
  }, [data.status]);
  if (!context)
    return (
      <p className="extension-message">
        Choose a cluster before opening an app page.
      </p>
    );
  if (data.status === "error" && selected?.scope !== scope) {
    // A 404 identifies an unavailable endpoint, not why it is unavailable.
    // Name the required API without claiming that discovery proved it absent.
    const args = binding?.arguments;
    const notFound =
      /\bApiError:\s*404\b|\bcode:\s*404\b|\b404 page not found\b/i.test(
        data.error ?? "",
      );
    const guidance =
      notFound &&
      typeof args?.group === "string" &&
      typeof args.version === "string" &&
      typeof args.plural === "string" &&
      typeof args.kind === "string"
        ? {
            title: `${args.kind} API unavailable`,
            detail: `This extension reads ${args.plural} from ${args.group}/${args.version}. Check that the selected cluster serves this API version. Installing an extension does not install its Kubernetes APIs.`,
          }
        : undefined;
    return (
      <ErrorNotice
        cluster
        message={data.error}
        retry={data.reload}
        guidance={guidance}
      />
    );
  }
  // Only the FIRST load of a scope replaces the section. A later refresh is
  // reported inside it ("Refreshing resources…"), because the section is not
  // only the table: an accepted write announces its resource and reloads this
  // list, so tearing the section down on every refresh took a running bulk
  // action's progress and its result away with it at the first acceptance —
  // and, before that, flashed the whole list away after every single write.
  if (data.status === "loading" && !loaded.current && selected?.scope !== scope)
    return (
      <p role="status" className="extension-message">
        Loading app resources…
      </p>
    );
  const rows = (data.data?.items ?? lastRows.current).filter((row) =>
    [row.name, row.namespace, ...row.columns,
      ...appColumns.columns.filter((column) => column.filterable === true).map((column) => column.getValue?.(row) ?? "")]
      .join(" ")
      .toLowerCase()
      .includes(search.toLowerCase()),
  );
  const sortColumn = appColumns.columns.find((column) => column.key === columnSort?.key && column.sortable);
  const collator = new Intl.Collator(undefined, {numeric:true,sensitivity:"base"});
  const ordered = sortColumn && columnSort ? [...rows].sort((left, right) => {
    const a = sortColumn.getSortValue?.(left) ?? sortColumn.getValue?.(left) ?? "";
    const b = sortColumn.getSortValue?.(right) ?? sortColumn.getValue?.(right) ?? "";
    const comparison = typeof a === "number" && typeof b === "number"
      ? a - b : collator.compare(String(a), String(b));
    return comparison * (columnSort.direction === "asc" ? 1 : -1);
  }) : rows;
  const shown = ordered.slice(0, visible);
  const hidden = Math.max(0, rows.length - shown.length);
  // Only a binding the host runs actions against gets a selection column:
  // checkboxes over a table with nothing to do on it are furniture.
  const selectable = binding?.target === "k8s.listCustomResource";
  // Resolved back to rows, never counted out of the set: a key the current
  // filter no longer shows is a resource the bar cannot act on, and a count
  // that includes it would promise a write that never happens.
  const pickedRows = selectable ? rows.filter((row) => picked.has(bulkResourceKey(row))) : [];
  const visibleKeys = shown.map((row) => bulkResourceKey(row));
  const allVisiblePicked = visibleKeys.length > 0 && visibleKeys.every((key) => picked.has(key));
  const toggleAllVisible = () =>
    setPicked((prev) => {
      const next = new Set(prev);
      for (const key of visibleKeys) {
        if (allVisiblePicked) next.delete(key);
        else next.add(key);
      }
      return next;
    });
  const toggleRow = (key: string) =>
    setPicked((prev) => {
      const next = new Set(prev);
      if (!next.delete(key)) next.add(key);
      return next;
    });
  return (
    <section className="extension-results" ref={listRow.ref}>
      <div className="extension-resource-list">
      {appColumns.errors.map((error) => (
        <ErrorNotice key={error.id} cluster title={`Couldn’t read ${error.title} columns`} message={error.message} retry={appColumns.reload} />
      ))}
      {data.data?.columnsError && <p className="extension-message" role="status">Could not load CRD columns: {data.data.columnsError}. Showing app-defined columns.</p>}
      {data.data?.truncated && (
        <p className="extension-message" role="status">
          Showing the first {data.data.items.length.toLocaleString()} resources; more remain on the cluster.
        </p>
      )}
      {!hideToolbar && (
        <div className="extension-toolbar">
          <span>
            {binding?.arguments.namespaced === false
              ? "Cluster-scoped resources"
              : namespace
                ? `Namespace: ${namespace}`
                : "All namespaces"}
          </span>
          <Button variant="secondary" onClick={data.reload}>
            Refresh
          </Button>
        </div>
      )}
      {selectable && (
        <ExtensionBulkActions
          key={scope}
          target={{id:plugin.manifest.id,revision:plugin.revision,capability,context}}
          selection={pickedRows}
          onClear={()=>setPicked(new Set())}
          available={actionAvailability}
        />
      )}
      {data.status === "loading" ? <p className="extension-message" role="status">Refreshing resources…</p> : data.status === "error" ? <ErrorNotice cluster message={data.error} retry={data.reload}/> : shown.length ? (
        <div className="extension-table-scroll">
          <table>
            <thead>
              <tr>
                {selectable && (
                  <th className="extension-check" scope="col">
                    <Checkbox
                      checked={allVisiblePicked}
                      indeterminate={!allVisiblePicked && visibleKeys.some((key)=>picked.has(key))}
                      onChange={toggleAllVisible}
                      ariaLabel="Select all"
                    />
                  </th>
                )}
                <th>Name</th>
                <th>Namespace</th>
                {columns.map((c, i) => (
                  <th key={i}>{c.name}</th>
                ))}
                {appColumns.columns.map((column) => <th key={column.key} aria-sort={columnSort?.key === column.key ? (columnSort.direction === "asc" ? "ascending" : "descending") : undefined}>
                  {column.sortable ? <button type="button" className="extension-column-sort" aria-label={`Sort by ${column.header}`}
                    onClick={() => setColumnSort((current) => ({key:column.key,direction:current?.key === column.key && current.direction === "asc" ? "desc" : "asc"}))}>
                    {column.header}{columnSort?.key === column.key && <span aria-hidden="true"> {columnSort.direction === "asc" ? "↑" : "↓"}</span>}
                  </button> : column.header}
                </th>)}
                <th>Age</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((row) => (
                <tr key={`${row.namespace}/${row.name}`} aria-selected={selected?.scope===scope && selected.name===row.name && selected.namespace===row.namespace} onDoubleClick={openResource && binding?.target === "k8s.listCustomResource" ? ()=>openResource({id:plugin.manifest.id,revision:plugin.revision,capability,context,namespace:row.namespace,name:row.name}):undefined} onClick={binding?.target === "k8s.listCustomResource" ? ()=>setSelected({scope,name:row.name,namespace:row.namespace}):undefined}>
                  {selectable && (
                    // Checking a box picks a row for a bulk action; it does not
                    // also open the detail peek behind it.
                    <td className="extension-check" onClick={(event)=>event.stopPropagation()}>
                      <Checkbox
                        checked={picked.has(bulkResourceKey(row))}
                        onChange={()=>toggleRow(bulkResourceKey(row))}
                        ariaLabel={`Select ${bulkResourceKey(row)}`}
                      />
                    </td>
                  )}
                  <td>
                    {binding?.target === "k8s.listCustomResource" ? <button className="extension-resource-link" ref={node=>{const key=`${row.namespace}/${row.name}`;if(node)rowButtons.current.set(key,node);else rowButtons.current.delete(key);}} onKeyDown={e=>{if(e.key==="Enter" && openResource){e.preventDefault();openResource({id:plugin.manifest.id,revision:plugin.revision,capability,context,namespace:row.namespace,name:row.name});}}} onClick={()=>setSelected({scope,name:row.name,namespace:row.namespace})}>{row.name}</button> : <span className="extension-resource-name" title={row.name}>{row.name}</span>}
                  </td>
                  <td className="extension-namespace">
                    {row.namespace || "—"}
                  </td>
                  {columns.map((column, i) => (
                    <td key={i}>
                      <ResultValue
                        value={row.columns?.[i]}
                        column={column.name}
                      />
                    </td>
                  ))}
                  {appColumns.columns.map((column) => <td key={column.key}>{column.render?.(row)}</td>)}
                  <td><AgeCell created={row.created} age={row.age} /></td>
                </tr>
              ))}
            </tbody>
          </table>
          {hidden > 0 && (
            <p className="extension-message">
              <Button variant="secondary" onClick={() => setVisible((n) => n + PAGE)}>
                Show {Math.min(PAGE, hidden).toLocaleString()} more
              </Button>
              <span> · {hidden.toLocaleString()} matching rows not shown</span>
            </p>
          )}
        </div>
      ) : (
        <p className="extension-message">
          {data.data?.items.length ? "No matching resources." : "No resources returned by this app."}
        </p>
      )}
      </div>
      {selected?.scope === scope && <div className="extension-detail-peek" style={{width:peekWidth}}>
        <ResizeHandle label="the resource details" width={peekWidth} minWidth={listRow.bounds.minWidth} maxWidth={listRow.bounds.maxWidth} edge="left" onResize={setPeekWidth} onCommit={savePeekWidth}/>
        <ExtensionResourceDetails key={`${scope}/${selected.namespace}/${selected.name}`} selection={{id:plugin.manifest.id,revision:plugin.revision,capability,context,namespace:selected.namespace,name:selected.name}} onClose={()=>{setSelected(null);rowButtons.current.get(`${selected.namespace}/${selected.name}`)?.focus();}} />
      </div>}
    </section>
  );
}
