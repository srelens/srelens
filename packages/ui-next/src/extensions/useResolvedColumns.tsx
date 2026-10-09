import { useEffect, useMemo, useState } from "react";
import { contributionKind, extensionEnabledFor, resolveExtensionColumns, type ExtensionColumnResult, type ExtensionTableColumn, type InstalledExtension } from "@srelens/core";
import { Badge, type Column } from "@srelens/ui-kit";
import type { ListRow } from "../lib/kinds/types";
import { plainText } from "./displayText";
import { StatusBadge } from "./StatusBadge";
import { useLiveApps } from "./liveReaders";

type Result = { plugin: InstalledExtension; state: "loading" | "ready" | "error"; pending?: boolean; data?: ExtensionColumnResult; error?: string };
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

/** The host's scalar JSONPath reader starts at one top-level key. */
function rootKey(path: string): string {
  const text = path.slice(1);
  if (text.startsWith("[")) {
    const end = text.indexOf("]");
    const bracket = text.slice(1, end);
    return (bracket.startsWith("'") || bracket.startsWith('"')) ? bracket.slice(1, -1) : bracket;
  }
  let key = "";
  for (let index = 0; index < text.length; index++) {
    const ch = text[index];
    if (ch === "." || ch === "[") break;
    if (ch === "\\" && index + 1 < text.length) key += text[++index];
    else key += ch;
  }
  return key;
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
    extensionEnabledFor(plugin, contextId) && (plugin.manifest.contributions.tableColumns?.some((column) => column.forKinds.includes(kind)) ||
      plugin.manifest.contributions.badges?.some((badge) => badge.forKinds.includes(kind))));
  const signature = JSON.stringify(offers.map((plugin) => [plugin.manifest.id, plugin.revision]));
  const [retry, setRetry] = useState(0);
  // A joined column's value lives on another kind (#566): follow each reader a
  // column or badge for this kind joins through, and resolve again, in place,
  // when one changes. Values from the row itself follow the row.
  const watched = offers.map((plugin) => {
    const contributions = plugin.manifest.contributions;
    const joinIds = new Set([
      ...(contributions.tableColumns ?? []).filter((column) => column.forKinds.includes(kind)).map((column) => column.source.join),
      ...(contributions.badges ?? []).filter((badge) => badge.forKinds.includes(kind)).map((badge) => badge.join),
    ].filter((id): id is string => !!id));
    return { plugin, capabilities: (contributions.joins ?? []).filter((join) => joinIds.has(join.id)).map((join) => join.capability) };
  });
  const [pulse, setPulse] = useState(0);
  const liveState = useLiveApps({ apps: watched, context, namespace, label: `columns:${kind}`, onChange: () => setPulse((n) => n + 1) });
  const scope = JSON.stringify([context, namespace, kind, signature, refresh, retry]);
  const [answer, setAnswer] = useState<{ scope: string; rows: Row[]; results: Result[] } | null>(null);
  useEffect(() => {
    let current = true;
    if (!context || offers.length === 0 || rows.length === 0) return;
    setAnswer((previous) => ({ scope, rows, results: offers.map((plugin) => {
      const old = previous?.scope === scope
        ? previous.results.find((result) => result.plugin.manifest.id === plugin.manifest.id)
        : undefined;
      return old?.state === "ready" ? { ...old, plugin, pending: true } : { plugin, state: "loading", pending: true };
    }) }));
    const rowKeys = new Set(offers.flatMap((plugin) => (plugin.manifest.contributions.tableColumns ?? [])
      .filter((column) => column.forKinds.includes(kind) && !column.source.join)
      .map((column) => rootKey(column.source.jsonPath))));
    const uids = rows.map((row) => {
      const uid = (row as ListRow & { uid?: unknown }).uid;
      return {
        uid: typeof uid === "string" ? uid : undefined,
        name: row.name, namespace: row.namespace ?? "",
        row: Object.fromEntries([...rowKeys].filter((key) => Object.hasOwn(row, key))
          .map((key) => [key, (row as Record<string, unknown>)[key]])),
      };
    });
    for (const plugin of offers) {
      resolveExtensionColumns(plugin.manifest.id, plugin.revision, context, namespace, kind, uids).then(
        (data) => {
          if (!current) return;
          setAnswer((previous) => previous?.scope === scope && previous.rows === rows
            ? { ...previous, results: previous.results.map((result) => result.plugin.manifest.id === plugin.manifest.id
              ? { plugin, state: "ready", data, pending: false } : result) }
            : previous);
        },
        (error) => {
          if (!current) return;
          setAnswer((previous) => previous?.scope === scope && previous.rows === rows
            ? { ...previous, results: previous.results.map((result) => result.plugin.manifest.id === plugin.manifest.id
              ? { plugin, state: "error", pending: false, error: error instanceof Error ? error.message : String(error) } : result) }
            : previous);
        },
      );
    }
    return () => { current = false; };
  }, [scope, rows, pulse]);
  const live = answer?.scope === scope ? answer.results : EMPTY_RESULTS;
  const rowsPending = answer?.scope === scope && answer.rows !== rows;
  const reload = () => setRetry((value) => value + 1);
  const columns = useMemo(() => offers.flatMap((plugin) => {
    const found = live.find((result) => result.plugin.manifest.id === plugin.manifest.id);
    const byIdentity = new Map(found?.data?.cells.map((cell) => [
      JSON.stringify([cell.uid ?? null, cell.namespace, cell.name]), cell,
    ]) ?? []);
    const cellFor = (row: Row) => {
      const uid = (row as ListRow & { uid?: unknown }).uid;
      return byIdentity.get(JSON.stringify([typeof uid === "string" ? uid : null, row.namespace ?? "", row.name]));
    };
    // One column per app for its badges (#541): the words its rules put on
    // this row. No badge is an answer ("—"); a badge the host could not
    // answer is "Couldn't read", never folded into the answer.
    const badgeColumns: Column<Row>[] = plugin.manifest.contributions.badges?.some((badge) => badge.forKinds.includes(kind)) ? [{
      // Its own prefix: every table column's key is `extension:<app>:<id>`, and
      // `badges` is a valid column id, so no suffix alone could keep them apart.
      key: `extension-badges:${plugin.manifest.id}`,
      header: plainText(plugin.manifest.name),
      sortable: false,
      filterable: true,
      getValue: (row) => (cellFor(row)?.badges ?? []).map((badge) => badge.label).join(" "),
      render: (row) => {
        if (found?.state === "error") return <span title={found.error}>Couldn’t read</span>;
        const cell = found?.state === "ready" ? cellFor(row) : undefined;
        if (!cell) return found?.state === "ready" && !found.pending && !rowsPending ? <span>—</span> : <span>Loading…</span>;
        const failed = Object.entries(cell.badgeErrors ?? {});
        const shown = cell.badges ?? [];
        if (!shown.length && !failed.length) return <span>—</span>;
        return <span className="extension-badges">
          {shown.map((badge) => <StatusBadge key={badge.id} resolved={badge} />)}
          {failed.map(([id, why]) => <span key={id} className="extension-badge-error" title={plainText(why)}>Couldn’t read</span>)}
        </span>;
      },
    }] : [];
    return [...badgeColumns, ...(plugin.manifest.contributions.tableColumns ?? []).filter((column) => column.forKinds.includes(kind)).map((column): Column<Row> => {
      const cell = (row: Row) => {
        const uid = (row as ListRow & { uid?: unknown }).uid;
        return byIdentity.get(JSON.stringify([typeof uid === "string" ? uid : null, row.namespace ?? "", row.name]));
      };
      const value = (row: Row) => cell(row)?.values[column.id] ?? null;
      const error = (row: Row) => cell(row)?.errors?.[column.id];
      return {
        key: `extension:${plugin.manifest.id}:${column.id}`,
        header: column.title,
        sortable: column.sortable === true,
        filterable: column.filterable === true,
        getValue: (row) => value(row) ?? "",
        getSortValue: (row) => sortCell(value(row), column.format),
        render: (row) => found?.state === "error"
          ? <span title={found.error}>Couldn’t read</span>
          : found?.state === "ready" && cell(row)
            ? error(row) ? <span title={error(row)} className="whitespace-nowrap">{error(row)}</span>
              : value(row) === null ? <span>—</span> : displayCell(value(row)!, column.format)
            : found?.state === "ready" && !found.pending && !rowsPending ? <span>—</span> : <span>Loading…</span>,
      };
    })];
  }), [signature, live, kind, rowsPending]);
  const errors = live.filter((result) => result.state === "error").map((result) => ({
    id: result.plugin.manifest.id, title: result.plugin.manifest.name, message: result.error ?? "Read failed",
  }));
  return { columns, errors, reload, live: liveState };
}
