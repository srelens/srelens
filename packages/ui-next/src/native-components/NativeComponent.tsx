import { type ReactNode } from "react";
import { Badge, CodeEditor, EmptyState, ErrorState, KV, LoadingState, MetricTile, Table } from "@srelens/ui-kit";
import { describeError } from "@srelens/core/lib/errors";
import { validateNativeComponent, type NativeComponentPayload, type NativeValue } from "@srelens/core/lib/nativeComponents";
import { NativeMarkdown } from "./NativeMarkdown";
import { NativeRows } from "./NativeRows";
import { NativeTimeseries } from "./NativeTimeseries";
import "./native-components.css";

/** Request lifecycle belongs to the host, never to the extension payload. */
export type NativeComponentState = { status: "loading" } | { status: "error"; error: unknown } | { status: "empty" };
export interface NativeComponentProps {
  payload?: unknown;
  /** Host-owned accessible region label. */
  label: string;
  state?: NativeComponentState;
  onRetry?: () => void;
  /** The host decides whether a Code component fills its allocated region. */
  fill?: boolean;
}
const show = (value: NativeValue) => value === null ? "Unknown" : typeof value === "boolean" ? value ? "Yes" : "No" : String(value);


function Content({ component, label, fill }: { component: NativeComponentPayload; label: string; fill?: boolean }) {
  switch (component.type) {
    case "KeyValue": return <NativeRows items={component.data.items}>{items => items.map((item, index) => <KV key={index} k={item.label} v={<span className="native-machine">{show(item.value)}</span>}/>)}</NativeRows>;
    case "Badge": return <div className="native-component-inset"><Badge tone={component.data.tone}>{component.data.label}</Badge></div>;
    case "Metric": return <MetricTile label={component.data.label} value={<span className="native-machine">{show(component.data.value)}{component.data.unit && ` ${component.data.unit}`}</span>} description={component.data.description}/>;
    case "Conditions": return <NativeRows items={component.data.items}>{items => items.map((item, index) => <article className="native-component-entry" key={index}>
      <div className="native-machine"><strong>{item.type}</strong> <Badge tone={item.status === "Unknown" ? "warn" : "muted"}>{item.status}</Badge></div>
      {item.reason && <div className="native-machine">{item.reason}</div>}
      {item.message && <p>{item.message}</p>}
      {item.observedGeneration !== undefined && <div className="native-machine">Observed generation: {item.observedGeneration}</div>}
      {item.lastTransitionTime && <time className="native-machine" dateTime={item.lastTransitionTime}>{item.lastTransitionTime}</time>}
    </article>)}</NativeRows>;
    case "Events": return <NativeRows items={component.data.items}>{items => items.map((item, index) => <article className="native-component-entry" key={index}>
      <div className="native-machine"><Badge tone={item.type === "Warning" ? "warn" : "info"}>{item.type}</Badge> <strong>{item.reason}</strong></div>
      <p>{item.message}</p><div className="native-machine">Count: {item.count ?? 1}{item.time && <> · <time dateTime={item.time}>{item.time}</time></>}</div>
    </article>)}</NativeRows>;
    case "Table": return <NativeRows items={component.data.rows}>{rows => <Table columns={component.data.columns.map((column, index) => ({ key: column.key, header: column.label, getValue: (row: NativeValue[]) => row[index], render: (row: NativeValue[]) => <span className="native-machine">{show(row[index])}</span> }))} data={rows} getRowKey={row => String(component.data.rows.indexOf(row))}/>}</NativeRows>;
    case "Timeline": return <NativeRows items={component.data.items}>{items => <ol className="native-component-timeline">{items.map((item, index) => <li className="native-component-entry" key={index}>
      <time className="native-machine" dateTime={item.time}>{item.time}</time><div className="native-machine"><Badge tone={item.tone ?? "muted"}>{item.title}</Badge></div>{item.detail && <p>{item.detail}</p>}
    </li>)}</ol>}</NativeRows>;
    case "Markdown": return <NativeMarkdown text={component.data.text}/>;
    case "Code": return <CodeEditor value={component.data.text} language={component.data.language} readOnly copy flush fill={fill} ariaLabel={label} minHeight={160} maxHeight={fill ? undefined : 480}/>;
    case "Timeseries": return <NativeTimeseries data={component.data}/>;
  }
}

function isEmpty(component: NativeComponentPayload) {
  // A chart whose provider answered with no samples in range: absence, not failure.
  if (component.type === "Timeseries") return component.data.series.every(entry => entry.values.every(value => value === null));
  if ("items" in component.data) return component.data.items.length === 0;
  if ("rows" in component.data) return component.data.rows.length === 0;
  if ("text" in component.data) return component.data.text.length === 0;
  return false;
}

/** The production rendering boundary accepts unknown JSON and fails closed. */
export function NativeComponent({ payload, label, state, onRetry, fill }: NativeComponentProps) {
  let content: ReactNode;
  if (state?.status === "loading") content = <LoadingState label={`Loading ${label}…`}/>;
  else if (state?.status === "error") {
    const error = describeError(state.error);
    content = <ErrorState title={`Could not load ${label}`} detail={error.detail} raw={error.raw} onRetry={onRetry}/>;
  } else if (state?.status === "empty") content = <EmptyState title="No data reported." compact/>;
  else {
    const result = validateNativeComponent(payload);
    if (!result.ok) content = <ErrorState title={`Could not display ${label}`} detail={result.error} onRetry={onRetry}/>;
    else content = isEmpty(result.value) ? <EmptyState title="No data reported." compact/> : <Content component={result.value} label={label} fill={fill}/>;
  }
  return <section className="native-component" data-fill={fill || undefined} aria-label={label}>{content}</section>;
}
