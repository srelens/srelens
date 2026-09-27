import { useState } from "react";
import { Button, EmptyState, Eyebrow, Section, Select } from "@srelens/ui-kit";
import {
  contributionKind,
  extensionEnabledFor,
  providersFor,
  queryExtensionProvider,
  type ExtensionProviderResult,
  type ExtensionTrace,
  type InstalledExtension,
} from "@srelens/core";
import { NativeComponent } from "../native-components/NativeComponent";
import { plainText } from "./displayText";
import { useContextLookup } from "./contextIds";
import { extensionLabel, useExtensions } from "./inventoryStore";
import { useResource } from "../lib/useResource";

/**
 * Metric and trace providers (#569) on a workload's or a pod's overview: each
 * provider an enabled app declares for the kind, asked through the host for this
 * resource, and drawn by the host — a metric provider's answer as the timeseries
 * chart (#570), a trace provider's as a table of text. A provider sends only data.
 */

/** The ranges a reader picks from; the host asks for 300 s to 7 days. */
const RANGES = [
  { value: "900", label: "15 minutes" },
  { value: "3600", label: "1 hour" },
  { value: "21600", label: "6 hours" },
  { value: "86400", label: "24 hours" },
  { value: "604800", label: "7 days" },
];
const DEFAULT_RANGE = "3600";
/** The most traces the host lists for one search. */
const MAX_TRACES = 50;

type Resource = { apiVersion: string; kind: string; metadata: { name: string; namespace?: string } };

function isResource(value: unknown): value is Resource {
  if (!value || typeof value !== "object") return false;
  const resource = value as Partial<Resource>;
  return typeof resource.kind === "string" && typeof resource.apiVersion === "string"
    && !!resource.metadata && typeof resource.metadata.name === "string";
}

/** What one panel asks: the provider, and the view's resource and range. */
interface Ask {
  plugin: InstalledExtension;
  provider: { id: string; title: string };
  context: string;
  namespace: string;
  kind: string;
  name: string;
  range: number;
  tick: number;
}

function useProvider(ask: Ask) {
  const { plugin, provider, context, namespace, kind, name, range, tick } = ask;
  return useResource<ExtensionProviderResult>(
    () => queryExtensionProvider({
      id: plugin.manifest.id, revision: plugin.revision, provider: provider.id,
      context, namespace, resourceKind: kind, name, rangeSeconds: range,
    }),
    [plugin.manifest.id, plugin.revision, provider.id, context, namespace, kind, name, range, tick],
    () => false,
  );
}

/** "CPU from Observability": what the panel is, and which app asked for it, so an app cannot pass its chart off as the host's. */
function panelName(ask: Ask) {
  return `${plainText(ask.provider.title)} from ${plainText(extensionLabel(ask.plugin))}`;
}

/** Which app a panel came from, after what it shows. */
function Origin({ plugin }: { plugin: InstalledExtension }) {
  return <p className="extension-provider-origin">From {plainText(extensionLabel(plugin))}</p>;
}

function MetricPanel(ask: Ask) {
  const answer = useProvider(ask);
  const name = panelName(ask);
  const chart = answer.data?.kind === "metrics" ? answer.data.chart : undefined;
  const title = plainText(ask.provider.title);
  // The chart draws its own title; an empty one is named here, since "No data reported"
  // alone would not say which of an app's panels matched nothing.
  const empty = chart !== undefined && chart.series.every((series) => series.values.every((value) => value === null));
  return (
    <section className="extension-provider" aria-label={name}>
      {empty ? (
        <EmptyState title={`No data reported for ${title}`} hint="The query matched no samples in this range." compact />
      ) : (
        <NativeComponent
          label={title}
          payload={chart && { version: 1, type: "Timeseries", data: chart }}
          state={answer.status === "loading" ? { status: "loading" }
            : answer.status === "error" ? { status: "error", error: answer.error }
            : chart ? undefined : { status: "error", error: "The provider did not answer a chart" }}
          onRetry={answer.reload}
        />
      )}
      <Origin plugin={ask.plugin} />
    </section>
  );
}

const when = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", second: "2-digit" });

/**
 * One trace as a row of text: a backend's names with invisible characters shown, as every
 * app value is. What a reader scans first leads — the operation, its service and how long
 * it took — and the long identifier is last, so a narrow Inspector shows the readable part.
 */
function traceRow(trace: ExtensionTrace) {
  return [
    trace.rootName === undefined ? "—" : plainText(trace.rootName),
    trace.rootService === undefined ? "—" : plainText(trace.rootService),
    trace.durationMs === undefined ? "—" : `${trace.durationMs.toLocaleString("en-US")} ms`,
    trace.start === undefined ? "—" : when.format(new Date(trace.start)),
    plainText(trace.traceId),
  ];
}

function TracePanel(ask: Ask) {
  const answer = useProvider(ask);
  const name = panelName(ask);
  const found = answer.data?.kind === "traces" ? answer.data : undefined;
  return (
    <section className="extension-provider" aria-label={name}>
      <h5 className="extension-provider-title">{plainText(ask.provider.title)}</h5>
      <NativeComponent
        label={plainText(ask.provider.title)}
        payload={found && {
          version: 1,
          type: "Table",
          data: {
            columns: [
              { key: "operation", label: "Operation" },
              { key: "service", label: "Service" },
              { key: "duration", label: "Duration" },
              { key: "started", label: "Started" },
              { key: "trace", label: "Trace ID" },
            ],
            rows: found.traces.map(traceRow),
          },
        }}
        state={answer.status === "loading" ? { status: "loading" }
          : answer.status === "error" ? { status: "error", error: answer.error }
          : found ? undefined : { status: "error", error: "The provider did not answer a list of traces" }}
        onRetry={answer.reload}
      />
      {found?.truncated && (
        <p className="extension-provider-origin">
          The search found more traces than the {MAX_TRACES} listed; the newest are shown.
        </p>
      )}
      <Origin plugin={ask.plugin} />
    </section>
  );
}

/** The metric and trace panels of every enabled app for this resource's kind, after the host's own sections. */
export function ExtensionProviderSlot({ context, resource }: { context: string; resource: unknown }) {
  const inventory = useExtensions();
  const lookup = useContextLookup(context);
  const [range, setRange] = useState(DEFAULT_RANGE);
  const [tick, setTick] = useState(0);
  if (!isResource(resource)) return null;
  const group = resource.apiVersion.includes("/") ? resource.apiVersion.split("/")[0] : "";
  const kind = contributionKind(resource.kind, group);
  const contextKey = lookup.status === "found" ? lookup.id : undefined;
  const plugins = (inventory.status === "ready" ? inventory.data?.plugins ?? [] : []).filter(
    (plugin) => plugin.enabled && !plugin.quarantined && !plugin.policyBlocked && extensionEnabledFor(plugin, contextKey),
  );
  const base = {
    context,
    namespace: resource.metadata.namespace ?? "",
    kind,
    name: resource.metadata.name,
    range: Number(range),
    tick,
  };
  const metrics = plugins.flatMap((plugin) => providersFor(plugin.manifest, "metrics", kind).map((provider) => ({ ...base, plugin, provider })));
  const traces = plugins.flatMap((plugin) => providersFor(plugin.manifest, "traces", kind).map((provider) => ({ ...base, plugin, provider })));
  if (metrics.length + traces.length === 0) return null;
  return (
    <Section title="Metrics and traces from apps" padded={false} className="extension-providers">
      <div className="extension-providers-controls">
        <Eyebrow>range</Eyebrow>
        <Select value={range} onValueChange={setRange} options={RANGES} aria-label="range" />
        <Button type="button" variant="secondary" size="xs" aria-label="Refresh app metrics and traces"
          onClick={() => setTick((count) => count + 1)}>
          Refresh
        </Button>
      </div>
      {metrics.map((ask) => <MetricPanel key={`${ask.plugin.manifest.id}/${ask.plugin.revision}/${ask.provider.id}`} {...ask} />)}
      {traces.map((ask) => <TracePanel key={`${ask.plugin.manifest.id}/${ask.plugin.revision}/${ask.provider.id}`} {...ask} />)}
    </Section>
  );
}
