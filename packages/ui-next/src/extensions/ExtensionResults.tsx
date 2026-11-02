import { AgeCell } from "../lib/ageCell";
import { ExtensionResourceDetails } from "./ExtensionResourceDetails";
import { ExtensionBulkActions, type BulkActionAvailability } from "./ExtensionBulkActions";
import { bulkResourceKey } from "./bulkActions";
import { Button, ColumnPicker, ResizeHandle, StatusPill, Table, filterTableData, type Column, type TableSort } from "@srelens/ui-kit";
import { clampPeekWidth, savePeekWidth, setPeekWidth, usePeekBounds, usePeekWidth } from "../lib/peekWidth";
import { ExtensionResourceNavigation } from "./resourceNavigation";
import { useContext, useMemo, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  ageSortValue,
  describeError,
  onExtensionResourceChanged,
  readExtension,
  rowInSelection,
  type ExtensionResourceResult,
  type InstalledExtension,
} from "@srelens/core";
import { useResource } from "../lib/useResource";
import { useResolvedColumns } from "./useResolvedColumns";
import { contributionKind } from "@srelens/core";
import { useContextId } from "./contextIds";
import { AppStatus, statusWord } from "./AppStatus";
import { LiveNotice, LiveStatus, useLiveReaders } from "./liveReaders";
import { readAcross, readerKind, readerNamespaces } from "./readAcross";
import { nameFilterAnswered, useCollectionNameFilter } from "../lib/collectionNameFilter";
import { NamespaceFailuresAlert } from "../lib/errorCopy";
import { CollectionNameNotice } from "../screens/collections/NamespaceScope";
import { ListCount } from "../screens/ListCount";
import { columnOptionsFor, emptyTableCopy } from "../screens/resourceShell";
import { toggleColumn, useHiddenColumns } from "../lib/columnPrefs";
import { useRowRefocus } from "../lib/kinds/namespaceCell";

type Row = ExtensionResourceResult["items"][number];

/** The identifier: never hidden, as on a built-in list. */
const NAME_KEY = "name";
const NO_COLUMNS: { name: string }[] = [];

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
/** A printer column's value: the standard status where it names a state, and text otherwise. */
function ResultValue({ value, column }: { value?: string; column: string }) {
  const raw = value || "—";
  const boolean = raw.toLowerCase();
  if (boolean === "true" || boolean === "false") {
    const yes = boolean === "true";
    if (column === "Ready") return <StatusPill status={yes ? "Ready" : "Not ready"} kind={yes ? "success" : "warning"} />;
    if (column === "Suspended" || column === "Reconciling") {
      const on = column === "Reconciling" ? "In progress" : "Yes";
      return <StatusPill status={yes ? on : "No"} kind={yes ? "warning" : "neutral"} />;
    }
  }
  const revision = raw.match(/^(.*?)@sha(?:1|256):([a-f0-9]{16,})$/i);
  return (
    <span className="extension-value" title={raw} aria-label={revision ? raw : undefined}>
      {revision ? `${revision[1]}@${revision[2].slice(0, 8)}` : raw}
    </span>
  );
}

export function ExtensionResults({
  plugin,
  capability,
  context,
  clusterId,
  namespaces: chosen = [],
  search = "",
  regex = false,
  refresh = 0,
  hideToolbar = false,
  actionAvailability,
  card,
  previewRows,
  noun = "resources",
  header,
  bulkBar,
}: {
  plugin: InstalledExtension;
  capability: string;
  context: string;
  /** The cluster whose tab narrows this list, by its collections' name filters. */
  clusterId?: string;
  /** None is all namespaces. With a card, what it counted in; otherwise one read each. */
  namespaces?: string[];
  search?: string;
  /** Whether `search` is a regular expression. */
  regex?: boolean;
  refresh?: number;
  hideToolbar?: boolean;
  /**
   * Optional availability override for the embedding surface. By default the
   * bulk bar evaluates the shared predicates against inspected resources.
   */
  actionAvailability?: BulkActionAvailability;
  /** A dashboard card whose rows alone are read (#540). */
  card?: string;
  /** A dashboard list preview: declared ordering and limit, without bulk selection. */
  previewRows?: {name:string;namespace:string}[];
  /** Lowercase and plural: what the list's count and failures call its rows. */
  noun?: string;
  /** Where the page wants the count and live state, in its header. */
  header?: HTMLElement | null;
  /** Where the page wants the bulk bar, in its filter row. */
  bulkBar?: HTMLElement | null;
}) {
  // What this reader's rows are in, which its columns and watch follow. Only a
  // card's own read keeps the namespaces it counted in, whatever kind they are.
  const namespaces = readerNamespaces(plugin, capability, chosen);
  const openResource = useContext(ExtensionResourceNavigation);
  const refocus = useRowRefocus();
  const listRow = usePeekBounds();
  const peekWidth = clampPeekWidth(usePeekWidth(), listRow.bounds);
  // The card is part of what is on screen: the filtered and whole lists keep no rows of each other.
  const scope = JSON.stringify([plugin.manifest.id,plugin.revision,capability,context,chosen,card ?? ""]);
  const [selected,setSelected] = useState<{scope:string;name:string;namespace:string}|null>(null);
  const preview = previewRows !== undefined;
  const [columnSort, setColumnSort] = useState<TableSort|null>(null);
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
    async () => {
      if (!context) return null;
      const { id } = plugin.manifest;
      // A card's rows are one host call, however many namespaces it counted in.
      if (card) {
        const several = chosen.length > 1 ? ([chosen] as [string[]]) : ([] as []);
        const answer = await readExtension(id, plugin.revision, capability, context, chosen.length === 1 ? chosen[0] : "", true, card, ...several);
        return { ...answer, failures: [] };
      }
      const { answers, failures } = await readAcross(namespaces, (namespace) => readExtension(id, plugin.revision, capability, context, namespace, true));
      return { ...answers[0], items: answers.flatMap((answer) => answer.items), truncated: answers.some((answer) => answer.truncated), failures };
    },
    [
      plugin.manifest.id,
      plugin.revision,
      capability,
      context,
      chosen.join("\u0000"),
      refresh,
      card,
    ],
  );
  const { reload, refresh: reread } = data;
  // Follow the reader's kind (#566): every change the watch reports reads the
  // list again in place, through the same read Refresh makes.
  const live = useLiveReaders({
    plugin, capabilities: [capability], context, namespaces, label: `page:${capability}`, onChange: reread,
  });
  // Refresh when an action on one of this list's resources is accepted, from any view.
  useEffect(
    () =>
      onExtensionResourceChanged((changed) => {
        if (
          changed.id === plugin.manifest.id &&
          changed.context === context &&
          changed.capability === capability &&
          rowInSelection(changed.namespace, namespaces)
        )
          reload();
      }),
    [plugin.manifest.id, context, capability, namespaces.join("\u0000"), reload],
  );
  const binding = plugin.manifest.capabilities.find(
    (b) => b.name === capability,
  );
  if (data.data?.items) lastRows.current = data.data.items;
  const allRows = data.data?.items ?? lastRows.current;
  const listed = useMemo(() => previewRows ? allRows.filter(row=>previewRows.some(wanted=>row.name===wanted.name && row.namespace===wanted.namespace)) : allRows, [allRows,previewRows]);
  // Narrowed ahead of everything that reads the rows, so the count, the columns,
  // the bulk bar and the table agree. A card's rows are what it counted.
  const nameFilter = useCollectionNameFilter(card ? undefined : clusterId, readerKind(plugin, capability), listed);
  const narrow = nameFilter.apply;
  const sourceRows = useMemo(() => narrow(listed), [narrow, listed]);
  const columnKind = contributionKind(
    typeof binding?.arguments.kind === "string" ? binding.arguments.kind : "",
    typeof binding?.arguments.group === "string" ? binding.arguments.group : "",
  );
  // The host resolves each row's status from the app's rules for this kind
  // (#541); a kind without a resolver has no status column rather than one
  // guessed from printer-column names.
  const resolvesStatus = binding?.target === "k8s.listCustomResource" &&
    !!plugin.manifest.contributions.statusResolvers?.some((resolver) => resolver.forKinds.includes(columnKind));
  const contextId = useContextId(context);
  const appColumns = useResolvedColumns({
    plugins: [plugin], context, contextId, namespaces, kind: columnKind, rows: sourceRows, refresh,
  });
  const printer = preview ? NO_COLUMNS : data.data?.printerColumns ?? (Array.isArray(binding?.arguments.printerColumns)
    ? (binding.arguments.printerColumns as Array<{ name: string }>)
    : NO_COLUMNS);
  const allColumns = useMemo(() => {
    const columns: Column<Row>[] = [
      { key: NAME_KEY, header: "Name" },
      { key: "namespace", header: "Namespace", render: (row) => row.namespace || "—" },
      ...(resolvesStatus ? [{
        key: "status",
        header: "Status",
        getValue: (row: Row) => row.status ? statusWord(row.status) : "",
        render: (row: Row) => row.status ? <AppStatus resolved={row.status} showReason /> : <span title="The host returned no status for this resource">—</span>,
      }] : []),
      ...printer.map((column, index): Column<Row> => ({
        key: `printer:${index}:${column.name}`,
        header: column.name,
        getValue: (row) => row.columns?.[index] ?? "",
        render: (row) => <ResultValue value={row.columns?.[index]} column={column.name} />,
      })),
      ...appColumns.columns,
      ...(preview ? [] : [{
        key: "age",
        header: "Age",
        align: "end" as const,
        getSortValue: ageSortValue,
        render: (row: Row) => <AgeCell created={row.created} age={row.age} />,
      }]),
    ];
    // A preview keeps the order its card declared.
    return preview ? columns.map((column) => ({ ...column, sortable: false })) : columns;
  }, [preview, resolvesStatus, printer, appColumns.columns]);
  const columnsKey = `extension:${plugin.manifest.id}:${capability}`;
  const hiddenColumns = useHiddenColumns(columnsKey);
  const shownColumns = useMemo(
    () => allColumns.filter((column) => column.key === NAME_KEY || !hiddenColumns.has(column.key)),
    [allColumns, hiddenColumns],
  );
  const rows = useMemo(() => filterTableData(sourceRows, shownColumns, search, null, regex), [sourceRows, shownColumns, search, regex]);
  const ordered = useMemo(
    () => previewRows ? previewRows.flatMap((wanted) => rows.filter((row) => row.name === wanted.name && row.namespace === wanted.namespace)) : rows,
    [previewRows, rows],
  );
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
    // A reader fixes one version or accepts several, the first served (#547).
    const versions = binding?.versions?.length
      ? binding.versions
      : typeof args?.version === "string"
        ? [args.version]
        : [];
    const guidance =
      notFound &&
      typeof args?.group === "string" &&
      versions.length > 0 &&
      typeof args.plural === "string" &&
      typeof args.kind === "string"
        ? {
            title: `${args.kind} API unavailable`,
            detail: `This extension reads ${args.plural} from ${args.group}/${versions.join(" or ")}. Check that the selected cluster serves ${versions.length > 1 ? "one of these API versions" : "this API version"}. Installing an extension does not install its Kubernetes APIs.`,
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
  // Only a binding the host runs actions against gets a selection column:
  // checkboxes over a table with nothing to do on it are furniture.
  const selectable = !preview && binding?.target === "k8s.listCustomResource";
  // Resolved back to rows, never counted out of the set: a key the current
  // filter no longer shows is a resource the bar cannot act on, and a count
  // that includes it would promise a write that never happens.
  const pickedRows = selectable ? rows.filter((row) => picked.has(bulkResourceKey(row))) : [];
  const peekable = binding?.target === "k8s.listCustomResource";
  const peek = (row: Row) => setSelected({ scope, name: row.name, namespace: row.namespace });
  const activate = (row: Row) => openResource
    ? openResource({ id: plugin.manifest.id, revision: plugin.revision, capability, context, namespace: row.namespace, name: row.name })
    : peek(row);
  const empty = sourceRows.length
    ? emptyTableCopy(sourceRows.length, noun, "", "")
    : { emptyText: `No ${noun}`, emptyHint: listed.length ? "None are in this cluster's collection." : "No resources returned by this app." };
  const picker = !preview && (
    <ColumnPicker columns={columnOptionsFor(allColumns)} hidden={hiddenColumns} onToggle={(key) => toggleColumn(columnsKey, key)} pinnedKey={NAME_KEY} />
  );
  return (
    <section className="extension-results" ref={listRow.ref}>
      <div className="extension-resource-list">
      {appColumns.errors.map((error) => (
        <ErrorNotice key={error.id} cluster title={`Couldn’t read ${error.title} columns`} message={error.message} retry={appColumns.reload} />
      ))}
      <NamespaceFailuresAlert what={noun} failures={data.data?.failures ?? []} />
      <CollectionNameNotice filter={nameFilter} noun={noun} />
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
              : namespaces.length
                ? `Namespace: ${namespaces.join(", ")}`
                : "All namespaces"}
          </span>
          <LiveStatus live={live} />
          {picker}
          <Button variant="secondary" onClick={data.reload}>
            Refresh
          </Button>
        </div>
      )}
      {hideToolbar && (header
        ? createPortal(<>{nameFilterAnswered(nameFilter) && <ListCount total={sourceRows.length} shown={rows.length} truncated={data.data?.truncated} noun={noun} />}<LiveStatus live={live} />{picker}</>, header)
        : <div className="extension-live-row"><LiveStatus live={live} />{picker}</div>)}
      <LiveNotice live={live} what="list" />
      {/* One notice when both are down for one reason; the columns' own only when they differ. */}
      {appColumns.live.state !== live.state && <LiveNotice live={appColumns.live} what="app column values" />}
      {selectable && (
        <ExtensionBulkActions
          key={scope}
          target={{id:plugin.manifest.id,revision:plugin.revision,capability,context}}
          selection={pickedRows}
          onClear={()=>setPicked(new Set())}
          available={actionAvailability}
          barSlot={bulkBar}
        />
      )}
      {data.status === "loading" ? <p className="extension-message" role="status">Refreshing resources…</p> : data.status === "error" ? <ErrorNotice cluster message={data.error} retry={data.reload}/> : !nameFilterAnswered(nameFilter) ? null : (
        <div ref={refocus.scope} className="extension-table-scroll" data-stale={live.state === "reconnecting" || undefined}>
          <Table
            columns={shownColumns}
            data={ordered}
            getRowKey={bulkResourceKey}
            selection={selectable ? { selected: picked, onChange: setPicked } : undefined}
            selectedKey={selected?.scope === scope ? bulkResourceKey(selected) : undefined}
            sort={columnSort}
            onSortChange={setColumnSort}
            onRowClick={peekable ? peek : undefined}
            onRowActivate={peekable ? activate : undefined}
            {...empty}
          />
        </div>
      )}
      </div>
      {selected?.scope === scope && <div className="extension-detail-peek" style={{width:peekWidth}}>
        <ResizeHandle label="the resource details" width={peekWidth} minWidth={listRow.bounds.minWidth} maxWidth={listRow.bounds.maxWidth} edge="left" onResize={setPeekWidth} onCommit={savePeekWidth}/>
        <ExtensionResourceDetails key={`${scope}/${selected.namespace}/${selected.name}`} selection={{id:plugin.manifest.id,revision:plugin.revision,capability,context,namespace:selected.namespace,name:selected.name}} onClose={()=>{refocus.remember(bulkResourceKey(selected));setSelected(null);}} />
      </div>}
    </section>
  );
}
