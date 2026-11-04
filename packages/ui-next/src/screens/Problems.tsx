import { useMemo, useState, type MouseEvent } from "react";
import {
  plural,
  rowInSelection,
  type ClusterContext,
  type NodeSummary,
} from "@srelens/core";
import { useNamespaceOptions } from "@srelens/core/react";
import {
  Alert,
  Button,
  EmptyState,
  Eyebrow,
  FilterBar,
  LoadingState,
  RawError,
  Screen,
  StatusPill,
  Table,
  Tabs,
  filterTableData,
  type Column,
  type TabItem,
} from "@srelens/ui-kit";
import { ContextLabel } from "../lib/contextLabel";
import { getKubeconfigFiles, useActiveContext } from "../lib/clusters";
import { detailRoute } from "../lib/detailRoute";
import { useNow, AgeCell } from "../lib/ageCell";
import { friendly, NamespaceFailuresAlert, StaleListAlert } from "../lib/errorCopy";
import { descriptorFor } from "../lib/kinds/descriptors";
import { EVENT_DESCRIPTOR, type EventRow } from "../lib/kinds/events";
import type { ListRow } from "../lib/kinds/types";
import type { PodRow } from "../lib/kinds/columns";
import { deriveProblems, type ProblemRow } from "../lib/problems";
import { useResourceList, type ResourceList } from "../lib/resourceList";
import { describe } from "../lib/routes";
import { openTab, useTabs } from "../lib/tabsStore";
import { useFallbackNamespace, useNamespaces } from "../lib/workspace";
import { CollectionScopeNotice, NamespaceScopePicker } from "./collections/NamespaceScope";
import { logsRoute } from "./Logs";
import { NamespaceErrorAlert, NoClusterScreen, PausedClusterScreen } from "./resourceShell";

const ALL = "all";
const SEGMENTS: TabItem[] = [
  { id: ALL, label: "All" },
  { id: "pod", label: "Pods" },
  { id: "node", label: "Nodes" },
  { id: "event", label: "Warnings" },
];

/** `/problems` — live triage for one cluster. */
export function Problems({ route }: { route: string }) {
  const context = useActiveContext();
  const { workspace } = useTabs();
  const title = describe(route, context?.name).title;

  if (!context) return <NoClusterScreen title={title} noun="problems" />;
  if (workspace.pausedClusters?.includes(context.stableId)) {
    return <PausedClusterScreen title={title} noun="problems" context={context} />;
  }
  return <ClusterProblems title={title} context={context} />;
}

function ClusterProblems({ title, context }: { title: string; context: ClusterContext }) {
  const name = context.name;
  const files = getKubeconfigFiles();
  const selection = useNamespaces(context.stableId);
  const namespaceOptions = useNamespaceOptions(name, files);
  useFallbackNamespace(context.stableId, namespaceOptions.scope);

  // Fixed hook sites: two live watches and one five-second poll. Namespace
  // changes reshape the pod/event views; Nodes stay cluster-scoped.
  const pods = useResourceList<ListRow>(name, "pods", descriptorFor("pods"), selection, files);
  const nodes = useResourceList<ListRow>(name, "nodes", descriptorFor("nodes"), [], files);
  const events = useResourceList<EventRow>(name, "events", EVENT_DESCRIPTOR, selection, files);
  const now = useNow();

  const rows = useMemo(() => deriveProblems({
    // The descriptors guarantee these shapes; each cast is at the same seam
    // Workloads uses before applying its per-kind verdict.
    pods: pods.rows.filter((row) => rowInSelection(row.namespace ?? "", selection)) as PodRow[],
    nodes: nodes.rows as NodeSummary[],
    events: events.rows.filter((row) => rowInSelection(row.namespace, selection)),
    now,
  }), [pods.rows, nodes.rows, events.rows, selection, now]);

  const [segment, setSegment] = useState(ALL);
  const [filter, setFilter] = useState("");
  const segmented = useMemo(
    () => segment === ALL ? rows : rows.filter((row) => row.source === segment),
    [rows, segment],
  );
  const columns = useMemo(() => problemColumns(name), [name]);
  const filtered = useMemo(
    () => filterTableData(segmented, columns, filter, null, false),
    [segmented, columns, filter],
  );

  const lists = [pods, nodes, events] as const;
  const pending = lists.filter((list) => list.status === "loading").length;
  const failed = lists.some((list) => Boolean(list.error));
  const completelyChecked = pending === 0 && !failed;
  const refresh = () => lists.forEach((list) => list.reload());

  return (
    <Screen
      title={title}
      eyebrow={<ContextLabel context={context} />}
      fill
      actions={
        <>
          <Eyebrow>{plural(rows.length, "current problem")}</Eyebrow>
          <Button variant="secondary" size="sm" aria-label="Refresh problems" onClick={refresh}>Refresh</Button>
        </>
      }
    >
      <p className="rule-b shrink-0 px-3 py-2 text-[0.8125rem] text-muted">
        Pod failures, node health and scheduling issues, and warning events from the last hour — one live view for triage.
      </p>
      <FilterBar
        value={filter}
        onValueChange={setFilter}
        label="Filter problems"
        placeholder="Filter by resource, reason, or namespace"
      >
        <Tabs tabs={SEGMENTS} active={segment} onChange={setSegment} label="Problem source" variant="segmented" />
        <NamespaceScopePicker context={context} options={namespaceOptions} />
      </FilterBar>

      <NamespaceErrorAlert error={namespaceOptions.error} />
      <CollectionScopeNotice context={context} />
      <ProblemFailure noun="pods" list={pods} />
      <ProblemFailure noun="nodes" list={nodes} />
      <ProblemFailure noun="warning events" list={events} />

      {pending > 0 && rows.length > 0 && (
        <p className="rule-b px-3 py-2 text-xs text-muted" role="status">
          Still checking {plural(pending, "source")}…
        </p>
      )}

      <div className="scroll min-h-0 flex-1">
        {rows.length === 0 && pending > 0 && !failed ? (
          <LoadingState label="Checking pods, nodes, and warning events" />
        ) : rows.length === 0 && completelyChecked ? (
          <EmptyState
            title="No problems detected"
            hint={`Checked pods, nodes, and warning events from the last hour on ${name}.`}
          />
        ) : rows.length === 0 ? null : (
          <Table
            columns={columns}
            data={filtered}
            getRowKey={(row) => row.key}
            onRowClick={(row) => openProblem(row, name)}
            onRowActivate={(row) => openProblem(row, name)}
            rowGroup={(row) => ({
              key: row.severity,
              label: row.severity === "critical" ? "Critical" : row.severity === "warning" ? "Warning" : "Unknown state",
            })}
            emptyText="No matching problems"
            emptyHint="Change the source, namespace, or text filter."
          />
        )}
      </div>
    </Screen>
  );
}

function ProblemFailure<Row>({ noun, list }: { noun: string; list: ResourceList<Row> }) {
  if (list.stale) {
    return <StaleListAlert what={noun} error={list.error} failures={list.namespaceFailures} className="mx-3 mt-3" />;
  }
  if (list.namespaceFailures.length > 0) {
    return <NamespaceFailuresAlert what={noun} failures={list.namespaceFailures} className="mx-3 mt-3" />;
  }
  if (!list.error) return null;
  const copy = friendly(list.error);
  return (
    <Alert tone="warn" title={`${capital(noun)} could not be checked`} className="mx-3 mt-3">
      {copy.detail}
      <RawError text={copy.raw ?? ""} className="mt-1" />
      <div className="mt-2">
        <Button type="button" variant="secondary" size="xs" aria-label={`Retry ${noun}`} onClick={list.reload}>Retry</Button>
      </div>
    </Alert>
  );
}

function capital(text: string): string {
  return text.charAt(0).toLocaleUpperCase() + text.slice(1);
}

function displayKind(row: ProblemRow): string {
  return row.group ? `${row.group}/${row.kind}` : row.kind;
}

function displayPath(row: ProblemRow): string {
  return row.namespace ? `${row.namespace}/${row.name}` : row.name;
}

function openProblem(row: ProblemRow, context: string) {
  openTab(
    detailRoute(row.kind, row.namespace || null, row.name, {
      cluster: context,
      ...(row.group ? { group: row.group } : {}),
    }),
    { clusterName: context },
  );
}

function stop(event: MouseEvent<HTMLButtonElement>) {
  event.stopPropagation();
}

function problemColumns(context: string): Column<ProblemRow>[] {
  return [
    {
      key: "reason",
      header: "Problem",
      sortable: true,
      render: (row) => <StatusPill status={row.reason} kind={row.health} tinted />,
      // A reader searches "cordoned" even though kubectl's exact status word
      // is SchedulingDisabled. Keep the status exact on screen and the common
      // operational word discoverable in search.
      getValue: (row) => `${row.reason}${row.reason.includes("SchedulingDisabled") ? " cordoned" : ""}`,
    },
    {
      key: "name",
      header: "Resource",
      sortable: true,
      render: (row) => (
        <span className="block min-w-36">
          <span className="block truncate font-medium" title={displayPath(row)}>{row.name}</span>
          <span className="block truncate text-xs text-muted">{displayKind(row)}</span>
        </span>
      ),
      getValue: (row) => `${displayKind(row)} ${displayPath(row)}`,
    },
    {
      key: "namespace",
      header: "Namespace",
      sortable: true,
      render: (row) => <span className="path">{row.namespace || "Cluster-scoped"}</span>,
      getValue: (row) => row.namespace || "Cluster-scoped",
    },
    {
      key: "detail",
      header: "Details",
      sortable: false,
      render: (row) => (
        <span className="block max-w-[30rem] truncate text-faint" title={row.detail}>
          {row.detail || "—"}{row.count && row.count > 1 ? ` · ${row.count} occurrences` : ""}
        </span>
      ),
    },
    {
      key: "age",
      header: "Age",
      sortable: true,
      align: "end",
      render: (row) => <span className="path text-faint"><AgeCell created={row.created} age={row.age} /></span>,
      getSortValue: (row) => row.created ? Date.parse(row.created) : Number.NEGATIVE_INFINITY,
    },
    {
      key: "actions",
      header: "",
      sortable: false,
      filterable: false,
      sticky: "end",
      render: (row) => {
        const kind = displayKind(row);
        const path = displayPath(row);
        return (
          <span className="flex justify-end gap-1">
            {row.kind === "Pod" && row.namespace && (
              <Button
                variant="secondary"
                size="xs"
                aria-label={`Follow logs for ${kind} ${path}`}
                onClick={(event) => {
                  stop(event);
                  openTab(logsRoute("Pod", row.namespace, row.name), { clusterName: context });
                }}
              >
                Logs
              </Button>
            )}
            <Button
              variant="secondary"
              size="xs"
              aria-label={`Open ${kind} ${path}`}
              onClick={(event) => {
                stop(event);
                openProblem(row, context);
              }}
            >
              Open
            </Button>
          </span>
        );
      },
    },
  ];
}
