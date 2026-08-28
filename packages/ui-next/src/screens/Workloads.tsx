import { useEffect, useMemo, useState } from "react";
import {
  ageSortValue,
  podStatus,
  rowInSelection,
  watchNamespaceForSelection,
  type ClusterContext,
  type CronJobSummary,
  type DaemonSetSummary,
  type DeploymentSummary,
  type StatefulSetSummary,
  type StatusVerdict,
} from "@srelens/core";
import { useNamespaceOptions } from "@srelens/core/react";
import {
  ColumnPicker,
  FilterBar,
  LiveSignal,
  LoadingState,
  Screen,
  StatusPill,
  Table,
  Tabs,
  filterTableData,
  type Column,
  type ContextMenuItem,
  type StatusKind,
  type TabItem,
} from "@srelens/ui-kit";
import { useConsole } from "../console";
import { getKubeconfigFiles, useActiveContext } from "../lib/clusters";
import { useHiddenColumns } from "../lib/columnPrefs";
import { detailRoute } from "../lib/detailRoute";
import { FailureAlert } from "../lib/errorCopy";
import {
  cronJobVerdict,
  daemonSetVerdict,
  deploymentVerdict,
  formatCpu,
  formatMemory,
  statefulSetVerdict,
  type PodRow,
} from "../lib/kinds/columns";
import { descriptorFor } from "../lib/kinds/descriptors";
import { withRowAffordances } from "../lib/kinds/rowAffordances";
import type { ListRow } from "../lib/kinds/types";
import { useResourceList, type ResourceList } from "../lib/resourceList";
import { describe } from "../lib/routes";
import { openTab } from "../lib/tabsStore";
import { setNamespaces, useNamespaces } from "../lib/workspace";
import { useRowMenu } from "./ResourceMenu";
import {
  NamespaceErrorAlert,
  NamespacePicker,
  NoClusterScreen,
  StaleSelectionAlert,
  columnOptionsFor,
  emptyTableCopy,
  toggleColumnVisibility,
  useResourceTabView,
} from "./resourceShell";

/** The row identifier: always shown, never offered to the column picker. */
const NAME_KEY = "name";

/**
 * One row of the union: whatever five differently-shaped kinds can all be
 * asked to answer, plus `kind` — the one column this screen has that a
 * single-kind `/k/<slug>` list must not, because interleaving five kinds
 * makes a row's kind otherwise unrecoverable.
 *
 * `flagged` is computed once, per row, from the same verdict its status word
 * and tone come from (see `fromVerdict`) — the affordance below only ever
 * reads it, rather than re-deriving health per kind. The typed lists'
 * `KindDescriptor.flagged` is that verdict's own `.flagged`, so the two hosts
 * cannot disagree about one row.
 */
interface WorkloadRow extends ListRow {
  kind: string;
  ready: string;
  statusLabel: string;
  statusKind: StatusKind;
  restarts?: number;
  cpu?: number;
  memory?: number;
  image?: string;
  age?: string;
  flagged: boolean;
  /**
   * CronJob only, and only for the row menu: `useRowMenu`'s own `isSuspended`
   * reads this straight off the row it's handed to decide Suspend vs. Resume.
   * Every other kind's menu reads nothing else off the row beyond
   * `name`/`namespace`, which `ListRow` already promises.
   */
  suspended?: boolean;
}

/** The design's segment vocabulary and order — not alphabetical, the mock's own. */
const SEGMENTS: TabItem[] = [
  { id: "All", label: "All" },
  { id: "Deployment", label: "Deployment" },
  { id: "StatefulSet", label: "StatefulSet" },
  { id: "DaemonSet", label: "DaemonSet" },
  { id: "Pod", label: "Pod" },
  { id: "CronJob", label: "CronJob" },
];

/**
 * The word, the tone and the dot for one row, from the one verdict its own
 * kind answers with.
 *
 * There is no table here pairing a label with a colour, and that is the point.
 * This screen used to carry `readyStatus`, which paired "Progressing" with
 * amber and "Available" with green by hand — a fourth such table, after core
 * was refactored so that no branch anywhere writes a tone and a flag side by
 * side except the six named verdicts in `k8sStatus`. It had already drifted
 * from all three others: a degraded Deployment read amber "Progressing" here
 * and red "Degraded" in the detail header a double-click away, with this row's
 * own red dot beside the amber word; a Deployment scaled to zero read green
 * "Available" against the header's neutral "Scaled down". No test asserted any
 * of those words. (#331)
 */
function fromVerdict(row: ListRow, kind: string, ready: string, verdict: StatusVerdict): WorkloadRow {
  return {
    name: row.name,
    namespace: row.namespace,
    kind,
    ready,
    statusLabel: verdict.status,
    statusKind: verdict.health,
    age: (row as { age?: string }).age,
    // From the same verdict as the word beside it — not from the descriptor's
    // own `flagged`, which is that verdict's `.flagged` anyway. One call, so
    // the dot and the word are two channels of one reading rather than two
    // readings that happen to agree.
    flagged: verdict.flagged,
  };
}

const fromDeployment = (row: ListRow): WorkloadRow => {
  const d = row as DeploymentSummary;
  return fromVerdict(d, "Deployment", d.ready, deploymentVerdict(d));
};

const fromStatefulSet = (row: ListRow): WorkloadRow => {
  const s = row as StatefulSetSummary;
  return fromVerdict(s, "StatefulSet", s.ready, statefulSetVerdict(s));
};

const fromDaemonSet = (row: ListRow): WorkloadRow => {
  const d = row as DaemonSetSummary;
  // DaemonSet reports `ready`/`desired` as two bare numbers rather than
  // Deployment/StatefulSet's own "N/M" string — recomposed here so the
  // union's Ready column reads the same shape for every kind.
  return fromVerdict(d, "DaemonSet", `${d.ready}/${d.desired}`, daemonSetVerdict(d));
};

function fromPod(row: ListRow): WorkloadRow {
  const p = row as PodRow;
  // Not `p.phase`: a pod whose container is in a back-off loop still reports
  // phase "Running", so a row reading the phase alone said "Running" beside
  // its own unhealthy dot — which comes from `podFlagged`, which asks this
  // same function. One reading, so the dot and the word cannot disagree.
  return {
    ...fromVerdict(p, "Pod", p.ready, podStatus(p)),
    restarts: p.restarts,
    cpu: p.cpu,
    memory: p.memory,
    image: p.image,
  };
}

/**
 * CronJob has no unhealthy state (Task 4's ruling: no sensible notion of
 * "unhealthy" for a schedule — core's `cronJobStatus` gives both its verdicts
 * `flagged: false`) and no ready count at all — its Ready cell is the one the
 * mock itself renders empty.
 */
function fromCronJob(row: ListRow): WorkloadRow {
  const c = row as CronJobSummary;
  return { ...fromVerdict(c, "CronJob", "—", cronJobVerdict(c)), suspended: c.suspended };
}

/**
 * The union's column set: what every one of the five kinds can be asked to
 * answer, plus Kind. READY, CPU, MEMORY, RESTARTS and IMAGE are only ever
 * answered by a subset of rows — the rest render an em dash rather than
 * being flattened away, per the controller ruling.
 */
const UNION_COLUMNS: Column<WorkloadRow>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "kind", header: "Kind", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "ready", header: "Ready", align: "end" },
  {
    key: "status",
    header: "Status",
    sortable: true,
    getValue: (r) => r.statusLabel,
    render: (r) => <StatusPill status={r.statusLabel} kind={r.statusKind} />,
  },
  {
    key: "restarts",
    header: "Restarts",
    sortable: true,
    align: "end",
    render: (r) => (r.restarts ?? "—"),
    getSortValue: (r) => r.restarts ?? -1,
  },
  {
    key: "cpu",
    header: "CPU",
    align: "end",
    render: (r) => (r.cpu == null ? "—" : formatCpu(r.cpu)),
    getSortValue: (r) => r.cpu ?? -1,
  },
  {
    key: "memory",
    header: "Memory",
    align: "end",
    render: (r) => (r.memory == null ? "—" : formatMemory(r.memory)),
    getSortValue: (r) => r.memory ?? -1,
  },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
  // Not sortable, same reason `podColumns` gives it none: a comma-joined list
  // of images has no single natural order.
  { key: "image", header: "Image", sortable: false, render: (r) => r.image || "—" },
];

/**
 * `/resources`: the design's Workloads view — five kinds (Deployment,
 * StatefulSet, DaemonSet, Pod, CronJob) interleaved in one list with a kind
 * segment control, rather than the kind-agnostic `/k/<slug>` screen's one
 * kind per tab.
 *
 * Split the same way `Resources.tsx` splits into an outer guard and an inner
 * list: with no cluster in focus there is no context name to watch, and a
 * hook cannot be skipped.
 */
export function Workloads({ route }: { route: string }) {
  const context = useActiveContext();
  const title = describe(route, context?.name).title;

  if (!context) {
    return <NoClusterScreen title={title} noun="workloads" />;
  }

  return <WorkloadList route={route} title={title} context={context} />;
}

/** One entry per fixed watch, bundled after the hooks below run — never used
 *  to decide *how many* hooks to call, only to summarize their results. */
interface KindEntry {
  key: string;
  label: string;
  list: ResourceList<ListRow>;
  toRow: (row: ListRow) => WorkloadRow;
}

function WorkloadList({
  route,
  title,
  context,
}: {
  route: string;
  title: string;
  context: ClusterContext;
}) {
  const name = context.name;
  const files = getKubeconfigFiles();
  const { ask } = useConsole();

  const selection = useNamespaces(context.stableId);
  const { namespaces, scope, error: namespaceError } = useNamespaceOptions(name, files);
  // A namespace-restricted credential watches its one namespace directly;
  // every workload kind here is namespaced, so there is no cluster-scoped
  // branch to take (unlike `KindList`, which serves cluster-scoped kinds
  // too).
  const namespaceFilter = watchNamespaceForSelection(selection);

  // Five watches at five fixed call sites — never a loop over a filtered
  // array, never conditional on the segment control. A hook count that
  // changes between renders is one React refuses, and the segment control
  // below narrows the concatenation, not the watches themselves.
  const deploymentsDescriptor = descriptorFor("deployments");
  const statefulSetsDescriptor = descriptorFor("statefulsets");
  const daemonSetsDescriptor = descriptorFor("daemonsets");
  const podsDescriptor = descriptorFor("pods");
  const cronJobsDescriptor = descriptorFor("cronjobs");
  const deploymentsList = useResourceList<ListRow>(name, "deployments", deploymentsDescriptor, namespaceFilter, files);
  const statefulSetsList = useResourceList<ListRow>(name, "statefulsets", statefulSetsDescriptor, namespaceFilter, files);
  const daemonSetsList = useResourceList<ListRow>(name, "daemonsets", daemonSetsDescriptor, namespaceFilter, files);
  const podsList = useResourceList<ListRow>(name, "pods", podsDescriptor, namespaceFilter, files);
  const cronJobsList = useResourceList<ListRow>(name, "cronjobs", cronJobsDescriptor, namespaceFilter, files);

  // `useRowMenu` is itself a hook — five fixed calls for the same reason the
  // five watches above are five fixed calls, one per kind's own descriptor
  // and actions, rather than one call sized to whichever kind a row happens
  // to be. A union row's actions genuinely differ by kind (Pod offers
  // shell/logs/evict, CronJob offers suspend/run-now, Deployment/StatefulSet
  // offer scale/restart, DaemonSet offers restart only), and this is what
  // keeps that parity with `/k/<kind>` without widening `useRowMenu` itself.
  const deploymentMenu = useRowMenu({
    context: name,
    kind: deploymentsDescriptor?.k8sKind ?? "Deployment",
    actions: deploymentsDescriptor?.actions ?? {},
  });
  const statefulSetMenu = useRowMenu({
    context: name,
    kind: statefulSetsDescriptor?.k8sKind ?? "StatefulSet",
    actions: statefulSetsDescriptor?.actions ?? {},
  });
  const daemonSetMenu = useRowMenu({
    context: name,
    kind: daemonSetsDescriptor?.k8sKind ?? "DaemonSet",
    actions: daemonSetsDescriptor?.actions ?? {},
  });
  const podMenu = useRowMenu({
    context: name,
    kind: podsDescriptor?.k8sKind ?? "Pod",
    actions: podsDescriptor?.actions ?? {},
  });
  const cronJobMenu = useRowMenu({
    context: name,
    kind: cronJobsDescriptor?.k8sKind ?? "CronJob",
    actions: cronJobsDescriptor?.actions ?? {},
  });

  /** Dispatched per row by `row.kind` — never by which kind the table was
   *  last sorted or filtered to, so a Pod row offers Pod actions even while
   *  the segment control sits on "All". */
  function rowMenuItems(row: WorkloadRow): ContextMenuItem[] {
    switch (row.kind) {
      case "Deployment":
        return deploymentMenu.items(row);
      case "StatefulSet":
        return statefulSetMenu.items(row);
      case "DaemonSet":
        return daemonSetMenu.items(row);
      case "Pod":
        return podMenu.items(row);
      case "CronJob":
        return cronJobMenu.items(row);
      default:
        return [];
    }
  }

  // A namespace-restricted credential has one namespace and no way to ask
  // for another — same rule `KindList` follows.
  useEffect(() => {
    if (scope) setNamespaces(context.stableId, [scope]);
  }, [scope, context.stableId]);

  // Plain data after the fixed hooks above, not another hook: summarizing
  // five results into a table is not itself something React needs to track
  // between renders.
  const kinds: KindEntry[] = [
    { key: "deployments", label: "Deployment", list: deploymentsList, toRow: fromDeployment },
    { key: "statefulsets", label: "StatefulSet", list: statefulSetsList, toRow: fromStatefulSet },
    { key: "daemonsets", label: "DaemonSet", list: daemonSetsList, toRow: fromDaemonSet },
    { key: "pods", label: "Pod", list: podsList, toRow: fromPod },
    { key: "cronjobs", label: "CronJob", list: cronJobsList, toRow: fromCronJob },
  ];

  // The union: every kind's rows, mapped and concatenated. One shared array,
  // one shared sort below it — sorting the concatenation is what makes the
  // table's sort cross kinds rather than run within each one.
  const rows = useMemo(
    () =>
      kinds.flatMap((k) =>
        k.list.rows
          .filter((row) => rowInSelection(row.namespace ?? "", selection))
          .map(k.toRow),
      ),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [deploymentsList.rows, statefulSetsList.rows, daemonSetsList.rows, podsList.rows, cronJobsList.rows, selection],
  );

  // The segment control filters this concatenation; it never touches the
  // five watches above, so switching segments cannot re-list.
  const [segment, setSegment] = useState("All");
  const segmented = useMemo(
    () => (segment === "All" ? rows : rows.filter((row) => row.kind === segment)),
    [rows, segment],
  );

  const hidden = useHiddenColumns("workloads");
  const columns = useMemo(
    () => UNION_COLUMNS.filter((column) => column.key === NAME_KEY || !hidden.has(column.key)),
    [hidden],
  );
  const renderedColumns = useMemo(
    () => withRowAffordances(columns, (row) => row.flagged, ask),
    [columns, ask],
  );

  const { tabId, sort, filter, filterKey, setFilter, setSort, setFilterKey } = useResourceTabView(route, columns);

  const filtered = useMemo(
    () => filterTableData(segmented, columns, filter, filterKey),
    [segmented, columns, filter, filterKey],
  );

  function onToggleColumn(key: string) {
    toggleColumnVisibility({ key, storageKey: "workloads", hidden, filterKey, tabId });
  }

  const columnOptions = columnOptionsFor(UNION_COLUMNS);

  const allLoading = kinds.every((k) => k.list.status === "loading");
  // Five watches means five ways to fail — a kind whose watch errored with
  // nothing cached contributes no rows and gets its own banner; the four
  // that answered stay on screen and keep being sorted and filtered with it.
  const failed = kinds.filter((k) => k.list.status === "error");
  const stale = kinds.filter((k) => k.list.status !== "error" && k.list.error);
  const anyReconnecting = kinds.some((k) => k.list.watch !== "live");

  const lower = title.toLocaleLowerCase();
  const segmentLower = segment === "All" ? lower : `${segment.toLocaleLowerCase()}s`;

  return (
    <Screen
      title={title}
      eyebrow={name}
      fill
      actions={
        <>
          <LiveSignal
            label={anyReconnecting ? "Stream lost" : "Live"}
            tone={anyReconnecting ? "warn" : "ok"}
          />
          <ColumnPicker
            columns={columnOptions}
            hidden={hidden}
            onToggle={onToggleColumn}
            pinnedKey={NAME_KEY}
          />
        </>
      }
    >
      <FilterBar
        value={filter}
        onValueChange={setFilter}
        label={`Filter ${lower}`}
        placeholder={`Filter ${lower}…`}
      >
        <Tabs tabs={SEGMENTS} active={segment} onChange={setSegment} label="Workload kind" />
        <NamespacePicker
          namespaces={namespaces}
          selection={selection}
          onChange={(next) => setNamespaces(context.stableId, next)}
        />
      </FilterBar>

      <NamespaceErrorAlert error={namespaceError} />

      <StaleSelectionAlert
        selection={selection}
        namespaces={namespaces}
        onReset={() => setNamespaces(context.stableId, [])}
      />

      {allLoading ? (
        <div className="scroll min-h-0 flex-1">
          <LoadingState label={`Loading ${lower}`} />
        </div>
      ) : (
        <>
          {/* Pinned above the scrolling table body, not inside it (whole-branch
              review) — a reader who scrolls the table must still see a kind
              that failed or went stale; a banner that scrolls away with the
              rows no longer warns anyone. */}
          {failed.map((k) => (
            <FailureAlert
              key={k.key}
              title={`Could not list ${k.label.toLocaleLowerCase()}s`}
              error={k.list.error}
              className="mx-3 mt-3 mb-3"
            />
          ))}
          {stale.map((k) => (
            <FailureAlert
              key={k.key}
              title={`These ${k.label.toLocaleLowerCase()}s are stale`}
              error={k.list.error}
              className="mx-3 mt-3 mb-3"
            />
          ))}
          <div className="scroll min-h-0 flex-1">
            <Table
              columns={renderedColumns}
              data={filtered}
              getRowKey={(row) => `${row.kind}/${row.namespace ?? ""}/${row.name}`}
              sort={sort}
              onSortChange={setSort}
              activeFilterKey={filterKey}
              onActiveFilterKeyChange={setFilterKey}
              onRowActivate={(row) =>
                openTab(detailRoute(row.kind, row.namespace ?? null, row.name), { clusterName: name })
              }
              rowMenu={rowMenuItems}
              rowMenuLabel={`${title} actions`}
              {...emptyTableCopy(segmented.length, segmentLower, name, " in the namespaces you are looking at")}
            />
          </div>
        </>
      )}
      {/* Outside the scrolling table body, same reason `KindList` keeps its
          one dialog there: a `ConfirmDialog` is a portal anyway, but a
          clipped ancestor is one fewer thing to reason about. Five, not one
          — but only the kind whose row menu is actually open ever has a
          non-null `pending`, so at most one of these five renders anything. */}
      {deploymentMenu.dialog}
      {statefulSetMenu.dialog}
      {daemonSetMenu.dialog}
      {podMenu.dialog}
      {cronJobMenu.dialog}
    </Screen>
  );
}
