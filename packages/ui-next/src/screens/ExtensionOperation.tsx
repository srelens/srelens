import { useEffect, useRef, useState } from "react";
import { callExtensionOperation, describeError, describeStreamEnd, extensionEnabledFor, extensionOperationRoute, openExtensionView, parseExtensionOperationRoute, type ExtensionOperation as Operation, type ExtensionView, type ExtensionStream } from "@srelens/core";
import { useNamespaceOptions } from "@srelens/core/react";
import { Button, Combobox, Screen, Table, TextInput } from "@srelens/ui-kit";
import { useExtensions } from "../extensions/inventoryStore";
import { plainText } from "../extensions/displayText";
import { useContexts, useContextsError, useContextsStatus } from "../lib/clusters";
import type { RoutedScreenProps } from "../lib/routes";
import { openTab } from "../lib/tabsStore";
import { NamespaceChoice, NamespaceErrorAlert } from "./resourceShell";

const label = (key: string) => key.replace(/([a-z])([A-Z])/g, "$1 $2").replace(/[-_]/g, " ").replace(/^./, (c) => c.toUpperCase());
const prose = (key: string) => /(^|\.)(scope|reason|message|description|title)$/i.test(key);
const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === "object" && !Array.isArray(value);
const cell = (value: unknown) => value === undefined || value === null ? "—" : plainText(typeof value === "object" ? JSON.stringify(value) : String(value));
type Scalar = string | number | boolean;
const scalar = (value: unknown): value is Scalar => ["string", "number", "boolean"].includes(typeof value);
const flatten = (value: Record<string, unknown>, prefix = ""): Array<[string, unknown]> => Object.entries(value).flatMap(([key, v]) => object(v) ? flatten(v, prefix + key + ".") : [[prefix + key, v]]);

function NamespaceInput({ context, value, onChange }: { context: string; value: string; onChange: (value: string) => void }) {
  const options = useNamespaceOptions(context, []);
  useEffect(() => { if (options.scope && !value) onChange(options.scope); }, [options.scope]);
  return <div className="min-w-44"><NamespaceErrorAlert error={options.error} />
    {options.namespaces === null ? <NamespaceChoice namespaces={null} value={value} onChange={onChange} />
      : <Combobox ariaLabel="Namespace" searchPlaceholder="Find a namespace…" value={value} onValueChange={onChange} options={[
        ...(!options.scope ? [{ value: "", label: "All namespaces" }] : []), ...options.namespaces.map((value) => ({ value })),
      ]} />}
  </div>;
}

/** Bounded data from a sidecar is rendered by the host, never as app HTML. */
function OperationResult({ value, operations, current, onOpen }: { value: unknown; operations: Operation[]; current: string; onOpen: (operation: string, params: Record<string, Scalar>) => void }) {
  const [filter, setFilter] = useState("");
  const fields = object(value) ? Object.entries(value) : [["Result", value] as const];
  const metadata = fields.filter(([key, value]) => key !== "nextCursor" && !Array.isArray(value)).flatMap(([key, value]) => object(value) ? flatten(value) : [[key, value] as [string, unknown]]);
  const lists = fields.filter(([, value]) => Array.isArray(value));
  return <div className="scroll min-h-0 min-w-0 flex-1">
    {metadata.length > 0 && <dl className="flex flex-wrap gap-x-6 gap-y-2 border-b px-3 py-2" style={{ borderColor: "var(--rule)" }}>
      {metadata.map(([key, value]) => <div key={key} className="min-w-0 max-w-full"><dt className="text-xs text-muted">{label(key)}</dt><dd className={`text-[0.8125rem] ${prose(key) ? "whitespace-normal" : "overflow-auto whitespace-nowrap"}`}>{cell(value)}</dd></div>)}
    </dl>}
    {lists.length > 0 && <div className="border-b px-3 py-2" style={{ borderColor: "var(--rule)" }}><TextInput aria-label="Filter results" placeholder="Filter results…" value={filter} onValueChange={setFilter} /></div>}
    {lists.map(([key, values]) => {
      const rows = (values as unknown[]).map((value, index) => ({ index, data: object(value) ? value : { value } })).filter(({ data }) => JSON.stringify(data).toLowerCase().includes(filter.toLowerCase()));
      const columns = [...new Set(rows.flatMap(({ data }) => Object.keys(data)))];
      const actions = (data: Record<string, unknown>) => operations.filter((op) => op.name !== current && !op.view?.stream && (op.inputs ?? []).some((input) => input.required && input.name !== "clusterId") && (op.inputs ?? []).every((input) => !input.required || input.name === "clusterId" || scalar(data[input.name])));
      return <section key={key} className="min-w-0">
        <h2 className="border-b px-3 py-2 text-[0.8125rem] font-medium" style={{ borderColor: "var(--rule)" }}>{label(key)} <span className="text-muted">{rows.length}{filter && ` of ${(values as unknown[]).length}`}</span></h2>
        <Table<(typeof rows)[number]> data={rows} getRowKey={(row) => String(row.index)} columns={[
          ...(rows.some(({ data }) => actions(data).length) ? [{ key: "open", header: "Details", render: ({ data }: (typeof rows)[number]) => <div className="flex gap-2">{actions(data).map((op) => <Button key={op.name} variant="secondary" onClick={() => onOpen(op.name, Object.fromEntries((op.inputs ?? []).filter((input) => input.name !== "clusterId" && scalar(data[input.name])).map((input) => [input.name, data[input.name] as Scalar])))}>{op.title}</Button>)}</div> }] : []),
          ...columns.map((key) => ({ key, header: label(key), render: ({ data }: (typeof rows)[number]) => object(data[key]) || Array.isArray(data[key]) ? <details><summary className="cursor-pointer whitespace-nowrap">View details</summary><pre className="max-h-48 max-w-96 overflow-auto whitespace-pre text-xs">{plainText(JSON.stringify(data[key], null, 2))}</pre></details> : <span title={cell(data[key])} className={prose(key) ? "block min-w-48 max-w-96 whitespace-normal" : "whitespace-nowrap"}>{key === "reportId" && String(data[key]).length > 20 ? `${String(data[key]).slice(0, 12)}…` : cell(data[key])}</span> })),
        ]} emptyText={filter ? "No results match this filter" : `No ${label(key).toLowerCase()} returned`} />
      </section>;
    })}
  </div>;
}

function OperationForm({ id, revision, context, contextKey, operation, operations, initial = {} }: { id: string; revision: number; context: string; contextKey: string; operation: Operation; operations: Operation[]; initial?: Record<string, Scalar> }) {
  const [values, setValues] = useState<Record<string, Scalar>>(initial);
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
  const inputs = (operation.inputs ?? []).filter((input) => input.name !== "clusterId" && initial[input.name] === undefined && input.name !== "cursor" && input.name !== "limit");
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
        stream.current = await view.current.open({ id, revision, context, namespace: String(params.namespace ?? ""), source: { kind: "operation", method: operation.name, params } }, {
          onData: (value) => { if (mine === generation.current) { setResult(value); completed.current = object(value) && value.state === "completed"; } },
          onEnd: (end) => { if (mine === generation.current) { setBusy(false); if (end.type === "error" || end.reason !== "completed") setError(describeStreamEnd(end)); else if (!completed.current) setError("The operation ended before returning a completed result."); } },
        });
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
      {inputs.map((input) => input.name === "namespace" ? <NamespaceInput key={input.name} context={context} value={String(values.namespace ?? "")} onChange={(namespace) => { if (!busy || operation.view?.autoRun) setValues({ ...values, namespace }); }} /> : <label key={input.name} className="flex min-w-40 flex-col gap-1 text-xs text-muted">
        {input.title ?? label(input.name)}
        {input.type === "boolean" ? <input type="checkbox" checked={values[input.name] === true} disabled={busy} onChange={(event) => setValues({ ...values, [input.name]: event.target.checked })} />
          : <TextInput value={String(values[input.name] ?? "")} type={input.type === "string" ? "text" : "number"} disabled={busy} onValueChange={(value) => setValues({ ...values, [input.name]: value })} />}
      </label>)}
      <Button type="submit" disabled={busy}>{busy ? "Running…" : operation.view?.autoRun ? "Refresh" : operation.title}</Button>
      {busy && operation.view?.stream && <Button type="button" variant="secondary" onClick={() => void stream.current?.cancel()}>Cancel</Button>}
    </form>
    {busy && <p className="extension-message" role="status">Running {operation.title.toLowerCase()}…</p>}
    {error && <div className="extension-message" role="alert">{error} <Button onClick={() => void run(cursor.current)}>Retry</Button></div>}
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
  return <Screen title={operation?.title ?? "App"} eyebrow={`${app?.manifest.name ?? target.id} · ${cluster?.name ?? target.contextKey}`} fill>
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      {inventory.status === "loading" || status === "loading" ? <p className="extension-message" role="status">Loading app…</p>
        : error ? <p className="extension-message" role="alert">{error}</p>
        : <OperationForm key={route} id={target.id} revision={target.revision} contextKey={target.contextKey} context={cluster!.pinnedId!} operation={operation!} operations={app!.manifest.sidecar!.operations} initial={target.params} />}
    </div>
  </Screen>;
}
