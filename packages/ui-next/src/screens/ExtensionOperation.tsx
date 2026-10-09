import { useEffect, useRef, useState } from "react";
import { listContexts, callExtensionOperation, describeError, describeStreamEnd, extensionEnabledFor, extensionOperationRoute, openExtensionView, parseExtensionOperationRoute, relativeTime, type ExtensionOperation as Operation, type ExtensionView, type ExtensionStream } from "@srelens/core";
import { useNamespaceOptions } from "@srelens/core/react";
import { ActionBar, Badge, Button, Combobox, Popover, Screen, Table, TextInput, type BadgeTone } from "@srelens/ui-kit";
import { ExtensionLogo } from "../extensions/ExtensionLogo";
import { useExtensions } from "../extensions/inventoryStore";
import { ErrorNotice } from "../extensions/ExtensionResults";
import { plainText } from "../extensions/displayText";
import { getContexts, getKubeconfigFiles, setContexts, useContexts, useContextsError, useContextsStatus } from "../lib/clusters";
import type { RoutedScreenProps } from "../lib/routes";
import { openTab } from "../lib/tabsStore";
import { NamespaceChoice, NamespaceErrorAlert } from "./resourceShell";

const label = (key: string) => key.replace(/([a-z])([A-Z])/g, "$1 $2").replace(/[-_]/g, " ").replace(/^./, (c) => c.toUpperCase());
const prose = (key: string) => /(^|\.)(scope|reason|message|description|title)$/i.test(key);
const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === "object" && !Array.isArray(value);
const cell = (value: unknown) => value === undefined || value === null ? "—" : plainText(typeof value === "object" ? JSON.stringify(value) : String(value));
type Scalar = string | number | boolean;
const scalar = (value: unknown): value is Scalar => ["string", "number", "boolean"].includes(typeof value);
const flatten = (value: Record<string, unknown>, prefix = ""): Array<[string, unknown]> => Object.entries(value).flatMap(([key, v]) => object(v) && key !== "summary" ? flatten(v, prefix + key + ".") : [[prefix + key, v]]);
const technical = (key: string) => /(^|\.)(clusterId|uid|resourceUid|resourceVersion|reportId|imageDigest|databaseDigest|scannerImage|scope)$/i.test(key);
const firstColumns = ["severity", "id", "image", "name", "namespace", "container", "kind", "binding", "package", "installedVersion", "fixedVersion", "title"];
const severityTone: Record<string, BadgeTone> = { CRITICAL: "sev", HIGH: "sev", MEDIUM: "warn", LOW: "info", UNKNOWN: "muted" };
const severityName = (name: string) => name.replace(/Count$/i, "").toUpperCase();
const severityOrder = (name: string) => { const index = Object.keys(severityTone).indexOf(severityName(name)); return index < 0 ? 100 : index; };
type ResultRow = { index: number; data: Record<string, unknown> };
const reportRow = (value: unknown): value is Record<string, unknown> => object(value) && typeof value.reportId === "string" && typeof value.category === "string" && typeof value.subject === "string";
const rowActions = (operations: Operation[], current: string, data: Record<string, unknown>) => operations.filter((op) => op.name !== current && (op.inputs ?? []).some((input) => input.required && input.name !== "clusterId") && (op.inputs ?? []).every((input) => !input.required || input.name === "clusterId" || (scalar(data[input.name]) && data[input.name] !== ""))).sort((a,b)=>{const priority=(operation:Operation)=>(operation.inputs??[]).some(input=>input.required&&input.name==="reportId") ? 3 : (operation.inputs??[]).some(input=>input.required&&input.name==="image") ? 2 : 1;return priority(b)-priority(a);});
const rowParams = (operation: Operation, data: Record<string, unknown>) => Object.fromEntries((operation.inputs ?? []).filter((input) => input.name !== "clusterId" && scalar(data[input.name])).map((input) => [input.name, data[input.name] as Scalar]));

function ReportActions({ data, operations, current, onOpen }: { data: Record<string, unknown>; operations: Operation[]; current: string; onOpen: (operation: string, params: Record<string, Scalar>) => void }) {
 const [primary, ...secondary] = rowActions(operations, current, data);
 return <div className="app-report-actions">
  {primary && <Button variant="secondary" onClick={()=>onOpen(primary.name,rowParams(primary,data))}>{primary.title}</Button>}
  <Popover label="Report actions and metadata" align="end" trigger={<span className="app-report-more" title="More report actions">•••<span className="sr-only">More report actions</span></span>} className="app-report-inspector">
   {(close)=><>
    {secondary.map(operation=><Button key={operation.name} variant="ghost" className="app-report-menu-action" onClick={()=>{close();onOpen(operation.name,rowParams(operation,data));}}>{operation.title}</Button>)}
    <ResultDetails title="Report metadata" entries={Object.entries(data)}/>
   </>}
  </Popover>
 </div>;
}

function ReportTable({ rows, operations, current, onOpen }: { rows: ResultRow[]; operations: Operation[]; current: string; onOpen: (operation: string, params: Record<string, Scalar>) => void }) {
 return <Table<ResultRow> data={rows} getRowKey={({data,index})=>typeof data.reportId === "string" ? data.reportId : `row-${index}`} columns={[
  {key:"subject",header:"Report",getValue:({data})=>data.subject,render:({data})=><div className="app-report-subject"><strong tabIndex={0} title={cell(data.subject)}>{cell(data.subject || "Unnamed report")}</strong><span tabIndex={0} title={cell(data.image || data.category)}>{label(String(data.category))} report{data.image && data.image !== data.subject ? <> · <span>{cell(data.image)}</span></> : null}</span></div>},
  {key:"findings",header:"Findings",getValue:({data})=>data.findings,render:({data})=><div className="app-report-findings"><strong>{cell(data.findings)}</strong>{object(data.summary) && Object.values(data.summary).some(count=>typeof count==="number"&&count>0) ? <ResultValue field="summary" value={Object.fromEntries(Object.entries(data.summary).filter(([,count])=>typeof count==="number"&&count>0))}/> : <span className="text-muted">{data.findings===0 ? "No findings" : "Severity unavailable"}</span>}</div>},
  {key:"namespace",header:"Namespace",getValue:({data})=>data.namespace,render:({data})=><ResultValue field="namespace" value={data.namespace || "Cluster"}/>},
  {key:"source",header:"Source",getValue:({data})=>data.source,render:({data})=><div className="app-report-source"><span>{data.source==="app" ? "App scan" : data.source==="operator" ? "Operator" : cell(data.source)}</span><ResultValue field="freshness" value={data.freshness ?? "unknown"}/></div>},
  {key:"reportedAt",header:"Reported",getValue:({data})=>data.reportedAt,render:({data})=>{const date=typeof data.reportedAt==="string" ? new Date(data.reportedAt) : null;return date && Number.isFinite(date.getTime()) ? <time className="app-report-time" dateTime={String(data.reportedAt)} title={String(data.reportedAt)}><span>{date.toLocaleString(undefined,{month:"short",day:"numeric",hour:"2-digit",minute:"2-digit"})}</span><span>{relativeTime(date.getTime(),Date.now())}</span></time> : <span className="text-muted">Not reported</span>; }},
  {key:"actions",header:"Actions",sortable:false,sticky:"end",render:({data})=><ReportActions data={data} operations={operations} current={current} onOpen={onOpen}/>},
 ]} emptyText="No reports match these filters" emptyHint="Try another source or clear the search."/>;
}

function ResultValue({ field, value, compact = false }: { field: string; value: unknown; compact?: boolean }) {
  if (field === "image" && typeof value === "string") return <span tabIndex={0} className="app-image-reference" title={cell(value)}>{cell(value)}</span>;
  if (["name", "namespace", "container"].includes(field) && typeof value === "string") return <span tabIndex={0} className="app-resource-reference" title={cell(value)}>{cell(value)}</span>;
  if (compact && Array.isArray(value)) return <div className="flex flex-wrap gap-1">{value.map((item, index) => <Badge key={index}>{cell(item)}</Badge>)}</div>;
  if (field === "summary" && object(value)) return <div className="flex flex-wrap gap-1">{Object.entries(value).sort(([a],[b])=>severityOrder(a)-severityOrder(b)).map(([name, count]) => <Badge key={name} tone={severityTone[severityName(name)] ?? "muted"}>{label((severityTone[severityName(name)] ? severityName(name) : name).toLowerCase())} {cell(count)}</Badge>)}</div>;
  if (["severity", "state", "freshness", "source"].includes(field) && typeof value === "string") {
    const tone = field === "severity" ? severityTone[value.toUpperCase()] ?? "muted" : ["unknown", "stale", "failed"].includes(value) ? "warn" : value === "served" || value === "completed" ? "ok" : "muted";
    return <Badge tone={tone}>{label(value)}</Badge>;
  }
  if (object(value) || Array.isArray(value)) return <details><summary className="cursor-pointer whitespace-nowrap">View details</summary><pre className="max-h-48 max-w-96 overflow-auto whitespace-pre text-xs">{plainText(JSON.stringify(value, null, 2))}</pre></details>;
  return <span title={cell(value)} className={prose(field) ? "block min-w-48 max-w-96 whitespace-normal" : "whitespace-nowrap"}>{cell(value)}</span>;
}

function ResultDetails({ entries, title }: { entries: Array<[string, unknown]>; title: string }) {
  return <details className="min-w-0 text-xs"><summary className="cursor-pointer py-1 text-muted">{title}</summary><dl className="space-y-2 py-2">{entries.map(([key, value]) => <div key={key}><dt className="text-xs text-muted">{label(key)}</dt><dd className="max-w-full overflow-auto whitespace-nowrap text-[0.8125rem]">{cell(value)}</dd></div>)}</dl></details>;
}

function NamespaceInput({ context, value, onChange, required }: { context: string; value: string; onChange: (value: string) => void; required?: boolean }) {
  const options = useNamespaceOptions(context, []);
  useEffect(() => { if (options.scope && !value) onChange(options.scope); }, [options.scope]);
  return <div className="min-w-0"><NamespaceErrorAlert error={options.error} />
    {options.namespaces === null ? <NamespaceChoice namespaces={null} value={value} onChange={onChange} />
      : <Combobox ariaLabel="Namespace" searchPlaceholder="Find a namespace…" value={value} onValueChange={onChange} options={[
        ...(!options.scope && !required ? [{ value: "", label: "All namespaces" }] : []), ...options.namespaces.map((value) => ({ value })),
      ]} />}
  </div>;
}

/** Bounded data from a sidecar is rendered by the host, never as app HTML. */
function OperationResult({ value, operations, current, onOpen }: { value: unknown; operations: Operation[]; current: string; onOpen: (operation: string, params: Record<string, Scalar>) => void }) {
  const [filter, setFilter] = useState("");
  const [source, setSource] = useState("");
  const fields = object(value) ? Object.entries(value) : [["Result", value] as const];
  const reports = object(value) && !value.state && Array.isArray(value.items) && value.items.every(reportRow) && (typeof value.totalReports === "number" || value.items.length > 0);
  const compactList = (key: string, value: unknown) => !["items", "findings", "bindings", "warnings"].includes(key) && Array.isArray(value) && value.length > 0 && value.length <= 8 && value.every(scalar);
  const metadata = fields.filter(([key, value]) => key !== "nextCursor" && (!Array.isArray(value) || compactList(key, value))).flatMap(([key, value]) => object(value) && key !== "summary" ? flatten(value) : [[key, value] as [string, unknown]]);
  const identity = metadata.filter(([key]) => technical(key) && key !== "scope");
  const warnings = object(value) && Array.isArray(value.warnings) ? value.warnings.filter((warning): warning is string => typeof warning === "string") : [];
  const lists = fields.filter(([key, value]) => key !== "warnings" && Array.isArray(value) && !compactList(key, value));
  return <div className="app-results scroll min-h-0 min-w-0 flex-1">
    {warnings.length > 0 && <div className="extension-message" role="alert">{warnings.map((warning, index) => <p key={index}>{plainText(warning)}</p>)}</div>}
    {reports ? <div className="app-report-intro"><h2>Security reports <span>{object(value) ? cell(value.totalReports ?? (value.items as unknown[]).length) : ""}</span></h2><p>Scan history and Operator reports. Severity counts belong to each report.</p></div> : metadata.length > 0 && <dl className="flex flex-wrap gap-x-6 gap-y-2 border-b px-3 py-2" style={{ borderColor: "var(--rule)" }}>
      {metadata.filter(([key]) => !technical(key) || key === "scope").map(([key, value]) => <div key={key} className="min-w-0 max-w-full"><dt className="text-xs text-muted">{label(key)}</dt><dd className={`text-[0.8125rem] ${prose(key) ? "whitespace-normal" : "overflow-auto whitespace-nowrap"}`}><ResultValue field={key} value={value} compact={compactList(key, value)} /></dd></div>)}
      {identity.length > 0 && <div className="min-w-0 basis-full"><ResultDetails entries={identity} title="Technical details" /></div>}
    </dl>}
    {lists.length > 0 && <div className="app-result-filters"><TextInput aria-label="Filter results" placeholder={reports ? "Search reports, images or namespaces…" : "Filter results…"} value={filter} onValueChange={setFilter}/>{reports && <div className="app-source-filters" role="group" aria-label="Report source">{[["","All sources"],["app","App scans"],["operator","Operator reports"]].map(([value,title])=><Button key={value} variant="ghost" type="button" aria-pressed={source===value} onClick={()=>setSource(value)}>{title}</Button>)}</div>}</div>}
    {lists.map(([key, values]) => {
      const rows = (values as unknown[]).map((value, index) => ({ index, data: object(value) ? value : { value } })).filter(({ data }) => (!reports || !source || data.source===source) && JSON.stringify(data).toLowerCase().includes(filter.toLowerCase()));
      const columns = [...new Set(rows.flatMap(({ data }) => Object.keys(data)))].filter((key) =>
        !metadata.some(([field, shared]) => field === key && scalar(shared) &&
          (values as unknown[]).every((value) => object(value) && value[key] === shared)));
      const visible = columns.filter((key) => !technical(key) || key === "clusterId").sort((a, b) => (firstColumns.indexOf(a) < 0 ? 100 : firstColumns.indexOf(a)) - (firstColumns.indexOf(b) < 0 ? 100 : firstColumns.indexOf(b)));
      const details = columns.filter((key) => technical(key) && key !== "clusterId");
      const secondary = (key: string): string | undefined => key === "name" && visible.includes("kind") ? "kind" : key === "container" && visible.includes("containerType") ? "containerType" : undefined;
      const combined = new Set(visible.map(secondary).filter(Boolean));
      const actions = (data: Record<string, unknown>) => rowActions(operations,current,data);
      return <section key={key} className="min-w-0">
        <h2 className={reports ? "sr-only" : "border-b px-3 py-2 text-[0.8125rem] font-medium"} style={{ borderColor: "var(--rule)" }}>{key === "items" ? `${rows.length} ${rows.length === 1 ? "result" : "results"}` : `${label(key)} (${rows.length})`}{filter && <span className="text-muted"> of {(values as unknown[]).length}</span>}</h2>
        {reports && key==="items" ? <ReportTable rows={rows} operations={operations} current={current} onOpen={onOpen}/> : <Table<(typeof rows)[number]> data={rows} getRowKey={(row) => String(row.index)} columns={[
          ...visible.filter((key) => !combined.has(key)).map((key) => ({ key, header: key === "name" && secondary(key) ? "Resource" : label(key), getValue: ({ data }: (typeof rows)[number]) => data[key], render: ({ data }: (typeof rows)[number]) => <div><ResultValue field={key} value={data[key]} />{secondary(key) && <div className="text-xs text-muted"><ResultValue field={secondary(key)!} value={data[secondary(key)!]} /></div>}</div> })),
          ...(details.length > 0 ? [{ key: "identity", header: "Details", sortable: false, render: ({ data }: (typeof rows)[number]) => <ResultDetails title="Inspect" entries={details.map((key) => [key, data[key]])} /> }] : []),
          ...(rows.some(({ data }) => actions(data).length) ? [{ key: "open", header: "Actions", sortable: false, sticky: "end" as const, render: ({ data }: (typeof rows)[number]) => <ActionBar className="app-row-actions" label="Result actions" max={1} actions={actions(data).map(op=>({id:op.name,label:op.title,onSelect:()=>onOpen(op.name,rowParams(op,data))}))}/> }] : []),
        ]} emptyText={filter ? "No results match this filter" : `No ${label(key).toLowerCase()} returned`} />}
      </section>;
    })}
  </div>;
}

function OperationForm({ id, revision, context, contextKey, operation, operations, initial = {} }: { id: string; revision: number; context: string; contextKey: string; operation: Operation; operations: Operation[]; initial?: Record<string, Scalar> }) {
  const [values, setValues] = useState<Record<string, Scalar>>(() => ({ ...Object.fromEntries((operation.inputs ?? []).filter(input => input.type === "boolean").map(input => [input.name, false])), ...initial }));
  const [result, setResult] = useState<unknown>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [history, setHistory] = useState<string[]>([]);
  const cursor = useRef("");
  const view = useRef<ExtensionView | undefined>(undefined);
  const stream = useRef<ExtensionStream | undefined>(undefined);
  const completed = useRef(false);
  const generation = useRef(0);
  useEffect(() => () => { generation.current++; void view.current?.close(); }, []);
  const inputs = (operation.inputs ?? []).filter((input) => input.name !== "clusterId" && (initial[input.name] === undefined || input.name === "image" || input.name === "namespace") && input.name !== "cursor" && input.name !== "limit");
  const run = async (page = "", replaceBusy = false) => {
    if (busy && !replaceBusy) return;
    const params: Record<string, unknown> = {};
    for (const input of operation.inputs ?? []) {
      const value = input.name === "clusterId" ? context : input.name === "cursor" ? page : values[input.name];
      if (value === undefined || value === "") {
        if (input.required) { setError(`${input.title ?? label(input.name)} is required`); return; }
        continue;
      }
      if (input.type === "integer" || input.type === "number") {
        const number = Number(value);
        if (!Number.isFinite(number) || (input.type === "integer" && !Number.isSafeInteger(number))) {
          setError(`${input.title ?? label(input.name)} must be a valid ${input.type}`); return;
        }
        params[input.name] = number;
      } else params[input.name] = value;
    }
    const mine = ++generation.current;
    cursor.current = page; setBusy(true); setError(""); setResult(undefined); completed.current = false;
    try {
      if (operation.view?.stream) {
        view.current ??= openExtensionView(id, operation.title);
        stream.current = undefined;
        const opened = await view.current.open({ id, revision, context, namespace: String(params.namespace ?? ""), source: { kind: "operation", method: operation.name, params } }, {
          onData: (value) => { if (mine === generation.current) { setResult(value); completed.current = object(value) && value.state === "completed"; } },
          onEnd: (end) => { if (mine === generation.current) { setBusy(false); if (end.type === "error" || end.reason !== "completed") setError(describeStreamEnd(end)); else if (!completed.current) setError("The operation ended before returning a completed result."); } },
        });
        if (mine === generation.current) stream.current = opened;
        else await opened.cancel();
        return;
      }
      const answer = await callExtensionOperation({ id, revision, context, operation: operation.name, params });
      if (mine === generation.current) setResult(answer);
    } catch (error) {
      if (mine === generation.current) {
        const described = describeError(error);
        setError(`${described.title}: ${described.detail}`);
        setBusy(false);
      }
    } finally { if (mine === generation.current && !operation.view?.stream) setBusy(false); }
  };
  const namespace = values.namespace;
  useEffect(() => {
    if (operation.view?.autoRun && (operation.inputs ?? []).every((input) => !input.required || input.name === "clusterId" || values[input.name] !== undefined)) { setHistory([]); void run("", true); }
  }, [namespace]);
  const next = object(result) && typeof result.nextCursor === "string" ? result.nextCursor : "";
  return <>
    <form className="flex flex-wrap items-end gap-2 border-b px-3 py-2" style={{ borderColor: "var(--rule)" }} onSubmit={(event) => { event.preventDefault(); setHistory([]); void run(); }}>
      {inputs.map((input) => input.name === "namespace" ? <NamespaceInput key={input.name} required={input.required} context={context} value={String(values.namespace ?? "")} onChange={(namespace) => { if (!busy || operation.view?.autoRun) setValues({ ...values, namespace }); }} /> : <label key={input.name} className="flex min-w-40 flex-col gap-1 text-xs text-muted">
        {input.title ?? label(input.name)}
        {input.type === "boolean" ? <input type="checkbox" checked={values[input.name] === true} disabled={busy} onChange={(event) => setValues({ ...values, [input.name]: event.target.checked })} />
          : <TextInput value={String(values[input.name] ?? "")} type={input.type === "string" ? "text" : "number"} disabled={busy} onValueChange={(value) => setValues({ ...values, [input.name]: value })} />}
      </label>)}
      <Button type="submit" variant={operation.view?.autoRun ? "secondary" : "primary"} disabled={busy}>{busy ? "Running…" : operation.view?.autoRun ? "Refresh" : operation.title}</Button>
      {busy && operation.view?.stream && <Button type="button" variant="secondary" onClick={() => {
        if (stream.current) { void stream.current.cancel(); return; }
        generation.current++; setBusy(false); setError("The operation was cancelled.");
        const owned = view.current; view.current = undefined; void owned?.close();
      }}>Cancel</Button>}
    </form>
    {busy && <p className="extension-message" role="status">Running {operation.title.toLowerCase()}…</p>}
    {error && <div className="app-operation-error" role="alert"><strong>Could not load {operation.title.toLowerCase()}</strong><p>{error}</p>{error.includes("narrow the namespace") && <p>Select a namespace above to reduce the inventory.</p>}<Button variant="secondary" onClick={() => void run(cursor.current)}>Retry</Button>{history.length > 0 && <Button variant="secondary" onClick={() => {setHistory([]);void run();}}>Start from first page</Button>}</div>}
    {result !== undefined && <OperationResult value={result} operations={operations} current={operation.name} onOpen={(operation, params) => openTab(extensionOperationRoute(contextKey, id, revision, operation, params))} />}
    {!busy && !error && result === undefined && <p className="extension-message">Run {operation.title.toLowerCase()} to load its data.</p>}
    {(next || history.length > 0) && <div className="flex shrink-0 justify-end gap-2 border-t px-3 py-2" style={{ borderColor: "var(--rule)" }}><Button variant="secondary" disabled={busy || !history.length} onClick={() => { const previous = history.at(-1)!; setHistory(history.slice(0, -1)); void run(previous); }}>Previous page</Button><Button variant="secondary" disabled={busy || !next} onClick={() => { setHistory([...history, cursor.current]); void run(next); }}>Next page</Button></div>}
  </>;
}

export function ExtensionOperation({ route }: RoutedScreenProps) {
  const target = parseExtensionOperationRoute(route);
  const inventory = useExtensions();
  const contexts = useContexts();
  const status = useContextsStatus();
  const contextError = useContextsError();
  const retryContexts = async () => {
    try { const result = await listContexts(getKubeconfigFiles()); setContexts([...(result.contexts ?? getContexts())], result.error ?? ""); }
    catch (error) { setContexts(getContexts(), describeError(error).detail); }
  };
  if (!target) return null;
  const app = inventory.data?.plugins.find((app) => app.manifest.id === target.id);
  const cluster = contexts.find((context) => context.key === target.contextKey);
  const operation = app?.manifest.sidecar?.operations.find((operation) => operation.name === target.operation);
  const error = inventory.status === "error" ? inventory.error
    : !cluster && status === "failed" ? contextError
    : !cluster ? "This cluster is no longer in your kubeconfig files."
    : !cluster.pinnedId ? "This cluster has no pinned identity. Reload your cluster list."
    : !app?.enabled || !extensionEnabledFor(app, target.contextKey) ? "This app is not enabled for this cluster."
    : app.revision !== target.revision ? "This app was updated. Open its current screen from Apps."
    : !operation ? "This app does not declare this operation." : "";
  return <Screen title={<span className="flex min-w-0 items-center gap-2"><ExtensionLogo icon={app?.icon} name={app?.manifest.name ?? target.id} size={22} /><span className="text-muted">{app?.manifest.name ?? target.id}</span><span className="text-faint">/</span><span>{operation?.title ?? "App"}</span></span>} actions={<span className="block max-w-56 truncate text-xs text-muted" title={cluster?.name ?? target.contextKey}>{cluster?.name ?? target.contextKey}</span>} fill>
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      {inventory.status === "loading" || status === "loading" ? <p className="extension-message" role="status">Loading app…</p>
        : inventory.status === "error" ? <ErrorNotice title="Could not load apps" message={inventory.error} retry={inventory.reload} />
        : !cluster && status === "failed" ? <ErrorNotice title="Could not load clusters" message={contextError} retry={() => { void retryContexts(); }} />
        : error ? <p className="extension-message" role="alert">{error}</p>
        : <OperationForm key={route} id={target.id} revision={target.revision} contextKey={target.contextKey} context={cluster!.pinnedId!} operation={operation!} operations={app!.manifest.sidecar!.operations} initial={target.params} />}
    </div>
  </Screen>;
}
