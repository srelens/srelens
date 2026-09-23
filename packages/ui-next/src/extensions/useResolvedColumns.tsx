import { useEffect, useMemo, useState } from "react";
import { contributionKind, extensionEnabledFor, resolveExtensionColumns, type ExtensionColumnResult, type ExtensionTableColumn, type InstalledExtension } from "@srelens/core";
import { Badge, type Column } from "@srelens/ui-kit";
import type { ListRow } from "../lib/kinds/types";

type Result = { plugin: InstalledExtension; state: "loading" | "ready" | "error"; data?: ExtensionColumnResult; error?: string };
const EMPTY_RESULTS: Result[] = [];

function displayCell(value: string, format: ExtensionTableColumn["format"]) {
  let shown = value;
  if (format === "number") {
    const number = Number(value);
    if (Number.isFinite(number)) shown = new Intl.NumberFormat().format(number);
  } else if (format === "date") {
    const date = new Date(value);
    if (!Number.isNaN(date.getTime())) shown = new Intl.DateTimeFormat(undefined, { dateStyle:"medium", timeStyle:"short" }).format(date);
  } else if (format === "duration") {
    const seconds = Number(value);
    if (Number.isFinite(seconds) && seconds >= 0) shown = seconds >= 3600
      ? `${Math.floor(seconds / 3600)}h ${Math.floor(seconds % 3600 / 60)}m`
      : seconds >= 60 ? `${Math.floor(seconds / 60)}m ${Math.floor(seconds % 60)}s` : `${seconds}s`;
  }
  if (format === "status" || format === "badge") {
    const lower = value.toLowerCase();
    const tone = /^(ready|healthy|success|true|ok)$/.test(lower) ? "ok"
      : /^(error|critical|failed|false)$/.test(lower) ? "sev"
      : /^(warning|pending|unknown)$/.test(lower) ? "warn" : "muted";
    return <Badge tone={tone}>{shown}</Badge>;
  }
  return <span title={value} className="whitespace-nowrap">{shown}</span>;
}
function sortCell(value: string | null, format: ExtensionTableColumn["format"]): string | number {
  if (value === null) return Number.NEGATIVE_INFINITY;
  if (format === "number") { const number = Number(value); return Number.isFinite(number) ? number : Number.NEGATIVE_INFINITY; }
  if (format === "date") { const time = new Date(value).getTime(); return Number.isFinite(time) ? time : Number.NEGATIVE_INFINITY; }
  return value;
}

/** API group is part of a kind's identity; a CRD can reuse a built-in kind name. */
export function qualifiedTableKind(kind: string, customGroup?: string): string {
  if (customGroup !== undefined) return contributionKind(kind, customGroup);
  const groups: Record<string, string> = {
    Deployment:"apps", StatefulSet:"apps", DaemonSet:"apps", ReplicaSet:"apps",
    Job:"batch", CronJob:"batch", HorizontalPodAutoscaler:"autoscaling",
    PodDisruptionBudget:"policy", PriorityClass:"scheduling.k8s.io", RuntimeClass:"node.k8s.io",
    Lease:"coordination.k8s.io", MutatingWebhookConfiguration:"admissionregistration.k8s.io",
    ValidatingWebhookConfiguration:"admissionregistration.k8s.io", EndpointSlice:"discovery.k8s.io",
    Ingress:"networking.k8s.io", IngressClass:"networking.k8s.io", NetworkPolicy:"networking.k8s.io",
    StorageClass:"storage.k8s.io", Role:"rbac.authorization.k8s.io", RoleBinding:"rbac.authorization.k8s.io",
    ClusterRole:"rbac.authorization.k8s.io", ClusterRoleBinding:"rbac.authorization.k8s.io",
  };
  return contributionKind(kind, groups[kind] ?? "");
}

/** One host request per contributing app and list refresh, regardless of row count. */
export function useResolvedColumns<Row extends ListRow>(args: {
  plugins: InstalledExtension[]; context: string; contextId?: string; namespace: string;
  kind: string; rows: Row[]; refresh?: number;
}) {
  const { plugins, context, contextId, namespace, kind, rows, refresh = 0 } = args;
  const offers = plugins.filter((plugin) => plugin.enabled && !plugin.quarantined && !plugin.policyBlocked &&
    extensionEnabledFor(plugin, contextId) && plugin.manifest.contributions.tableColumns?.some((column) => column.forKinds.includes(kind)));
  const signature = JSON.stringify(offers.map((plugin) => [plugin.manifest.id, plugin.revision]));
  const [retry, setRetry] = useState(0);
  const scope = JSON.stringify([context, namespace, kind, signature, refresh, retry]);
  const [answer, setAnswer] = useState<{ scope: string; rows: Row[]; results: Result[] } | null>(null);
  useEffect(() => {
    let current = true;
    if (!context || offers.length === 0 || rows.length === 0) return;
    setAnswer({ scope, rows, results: offers.map((plugin) => ({ plugin, state: "loading" })) });
    const uids = rows.map((row) => {
      const uid = (row as ListRow & { uid?: unknown }).uid;
      return {
        uid: typeof uid === "string" ? uid : undefined,
        name: row.name, namespace: row.namespace ?? "", row: row as Record<string, unknown>,
      };
    });
    for (const plugin of offers) {
      resolveExtensionColumns(plugin.manifest.id, plugin.revision, context, namespace, kind, uids).then(
        (data) => {
          if (!current) return;
          setAnswer((previous) => previous?.scope === scope && previous.rows === rows
            ? { ...previous, results: previous.results.map((result) => result.plugin.manifest.id === plugin.manifest.id
              ? { plugin, state: "ready", data } : result) }
            : previous);
        },
        (error) => {
          if (!current) return;
          setAnswer((previous) => previous?.scope === scope && previous.rows === rows
            ? { ...previous, results: previous.results.map((result) => result.plugin.manifest.id === plugin.manifest.id
              ? { plugin, state: "error", error: error instanceof Error ? error.message : String(error) } : result) }
            : previous);
        },
      );
    }
    return () => { current = false; };
  }, [scope, rows]);
  const live = answer?.scope === scope && answer.rows === rows ? answer.results : EMPTY_RESULTS;
  const reload = () => setRetry((value) => value + 1);
  const columns = useMemo(() => offers.flatMap((plugin) => {
    const found = live.find((result) => result.plugin.manifest.id === plugin.manifest.id);
    const byIdentity = new Map(found?.data?.cells.map((cell) => [
      JSON.stringify([cell.uid ?? null, cell.namespace, cell.name]), cell,
    ]) ?? []);
    return (plugin.manifest.contributions.tableColumns ?? []).filter((column) => column.forKinds.includes(kind)).map((column): Column<Row> => {
      const cell = (row: Row) => {
        const uid = (row as ListRow & { uid?: unknown }).uid;
        return byIdentity.get(JSON.stringify([typeof uid === "string" ? uid : null, row.namespace ?? "", row.name]));
      };
      const value = (row: Row) => cell(row)?.values[column.id] ?? null;
      return {
        key: `extension:${plugin.manifest.id}:${column.id}`,
        header: column.title,
        sortable: column.sortable === true,
        filterable: column.filterable === true,
        getValue: (row) => value(row) ?? "",
        getSortValue: (row) => sortCell(value(row), column.format),
        render: (row) => found?.state === "error"
          ? <span title={found.error}>Couldn’t read</span>
          : found?.state === "ready"
            ? value(row) === null ? <span>—</span> : displayCell(value(row)!, column.format)
            : <span>Loading…</span>,
      };
    });
  }), [signature, live, kind]);
  const errors = live.filter((result) => result.state === "error").map((result) => ({
    id: result.plugin.manifest.id, title: result.plugin.manifest.name, message: result.error ?? "Read failed",
  }));
  return { columns, errors, reload };
}
