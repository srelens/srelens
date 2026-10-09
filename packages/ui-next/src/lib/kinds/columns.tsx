import {
  ageSortValue,
  cronJobStatus,
  formatStorageSize,
  jobStatus,
  nodeStatus,
  containersSortValue,
  describeContainer,
  nodeUsage,
  podUsage,
  type PodResourceUsage,
  phaseKind,
  podStatus,
  scaledStatus,
  type ClusterRoleBindingSummary,
  type ClusterRoleSummary,
  type ConfigMapSummary,
  type CronJobSummary,
  type DaemonSetSummary,
  type DeploymentSummary,
  type EndpointSliceSummary,
  type IngressSummary,
  type JobSummary,
  type LimitRangeSummary,
  type NetworkPolicySummary,
  type NamespaceSummary,
  type NodeSummary,
  type PodSummary,
  type PvSummary,
  type PvcSummary,
  type ResourceQuotaSummary,
  type RoleBindingSummary,
  type RoleSummary,
  type SecretSummary,
  type ServiceAccountSummary,
  type ServiceSummary,
  type StatefulSetSummary,
  type StatusVerdict,
  type StorageClassSummary,
  TAINT_COLUMN_HINT,
  taintBadgeLabel,
  taintBadgeText,
  taintSortValue,
  taintTallyText,
  taintTooltip,
} from "@srelens/core";
import { AgeCell } from "../ageCell";
import { ContainerBlocks } from "./containerBlocks";
import type { UsageSample } from "../usageHistory";
import { Badge, loadTone, Meter, Sparkline, StatusPill, Tooltip, type Column, type Tone } from "@srelens/ui-kit";
import { NodeLink } from "../nodeLink";

export type PodRow = PodSummary & {
  cpu?: number;
  memory?: number;
  /** The last ten minutes of readings, oldest first, ending in the figures
   *  above — what this window has itself been given, see `lib/usageHistory`.
   *  Keyed by name, so it may reach back before this pod: read it through
   *  `ownHistory`. */
  usageHistory?: UsageSample[];
};

/**
 * A pod's readings that are its own.
 *
 * History is kept by namespace and name, and a StatefulSet's `web-0` deleted
 * and created again is a new pod under the old name. The readings carry no
 * identity to tell them apart by; the row does — it knows when its pod was
 * created — so whatever was read before then belonged to the pod before it,
 * and is not drawn as this one's past.
 */
export function ownHistory(row: Pick<PodRow, "created" | "usageHistory">): UsageSample[] {
  const samples = row.usageHistory ?? [];
  const born = row.created ? Date.parse(row.created) : NaN;
  return Number.isNaN(born) ? samples : samples.filter((s) => s.at >= born);
}

/** "3 minutes", "40 seconds" — how much past a graph is showing. */
function spanWords(ms: number): string {
  const minutes = Math.round(ms / 60_000);
  if (minutes >= 1) return `${minutes} minute${minutes === 1 ? "" : "s"}`;
  const seconds = Math.round(ms / 1000);
  return `${seconds} second${seconds === 1 ? "" : "s"}`;
}
export type NodeRow = NodeSummary & { cpu?: number; memory?: number };

/** A thin space (U+2009), not a locale comma — the design's CPU thousands separator. */
const THIN_SPACE = " ";

/**
 * CPU in millicores: a bare number under 1000 ("241m"), thousands-grouped
 * with a thin space at or above it ("2 410m") — the design's own grouping,
 * distinct from a locale-formatted comma and readable at four digits, where a
 * bare run of digits is not.
 */
export function formatCpu(value: number): string {
  const rounded = Math.round(value);
  const digits = Math.abs(rounded).toString();
  const grouped =
    digits.length > 3 ? digits.replace(/\B(?=(\d{3})+(?!\d))/g, THIN_SPACE) : digits;
  return `${rounded < 0 ? "-" : ""}${grouped}m`;
}

/** Node usage is easier to compare in cores; retain millicore precision below one core. */
export function formatNodeCpu(value: number): string {
  const cores = Number((value / 1000).toFixed(Math.abs(value) < 1000 ? 3 : 2));
  return `${cores} ${cores === 1 ? "core" : "cores"}`;
}

/**
 * Memory in Mi: a bare number under 1024 Mi ("412 Mi"), scaled to Gi with one
 * decimal place at or above it ("3.1 Gi") — the design shows both, and a
 * space before the unit either way (classic ran the two together: "988Mi").
 */
export function formatMemory(value: number): string {
  if (value >= 1024) return `${(value / 1024).toFixed(1)} Gi`;
  return `${value} Mi`;
}

/**
 * A reading metrics-server did not give us is not zero: an em dash says so,
 * and `-1` sorts it below every real reading rather than into the middle of
 * the idle pods. `getSortValue` reads this straight — never the string
 * `format` renders — so the raw Mi value orders "3.1 Gi" correctly against
 * "988 Mi", which a comparator pointed at the display text could not.
 */
const metric = (value: number | undefined, format: (value: number) => string) =>
  value == null ? "—" : format(value);
const metricSort = (value: number | undefined) => value ?? -1;

/**
 * How much room a usage cell asks for, whether or not it holds a bar.
 *
 * `Table` measures the natural column widths on the first render that has
 * rows and pins them, and the node list answers before the metrics do — so
 * the first render is all dashes. A width that arrived with the bars arrived
 * after the column had been fixed at the width of a dash, and the bars drew
 * across the column beside them. Overview's node table hit exactly this; see
 * `READING_WIDTH` there.
 */
const USAGE_CELL = "flex min-w-[13rem] items-center gap-2";

/**
 * The narrowest a usage column may be dragged: the cell's own 13rem (208px)
 * and the padding either side of it.
 *
 * `Column.minWidth` is the floor a resize stops at, and without one it is
 * 72px — well under what the cell above will shrink to. Dragged below 13rem
 * the column kept the width it was given and the bar ran on into the column
 * beside it, since a table cell does not clip what overflows it.
 */
const USAGE_COLUMN_MIN_WIDTH = 232;

/**
 * A pod's CPU and memory against what it was given — core's `podUsage`, so
 * this list and a pod's own page cannot mean different things by a percentage.
 *
 * The row carries the metric's two figures flattened onto it (`cpu`,
 * `memory`), as a node row does; they arrive together or not at all.
 */
function podLoad(p: PodRow) {
  const metric = p.cpu == null || p.memory == null ? undefined : { cpuMillicores: p.cpu, memoryMiB: p.memory };
  return podUsage(p, metric);
}

/**
 * One pod's CPU or memory in the Pods list: the amount, and how it stands
 * against what the pod was given (#864). CPU as a small graph of its last ten
 * minutes; memory as a bar.
 *
 * The two are drawn differently because they behave differently. CPU moves:
 * it spikes and idles from one reading to the next, and a single figure says
 * little about a pod that was pinned a minute ago. Where it has been is the
 * information. Memory mostly sits where it is or creeps, and the question a
 * reader has of it is "how full" — which a bar answers at a glance and a
 * nearly flat line does not.
 *
 * Both are measured against the pod's limit, or its request where it has no
 * whole limit; both are coloured by load against a limit and keep one quiet
 * tone against a request, since running past a request is ordinary; both say
 * which in the tooltip and in their own name.
 *
 * The graph is part of the row, not a widget in it: no frame, no axis, no
 * background of its own. Its top edge is the bound, so a pod idling under its
 * limit is a low line and one pressed against it is a line along the top. Its
 * past is only what this window has been given since the list was first
 * opened: a graph that has just started is one reading, drawn flat.
 *
 * States, kept apart:
 * - no metric: a dash, and nothing drawn — an empty bar or graph would read
 *   as an idle pod;
 * - a metric, but neither a limit nor a request: the amount; for CPU a muted
 *   graph scaled to its own peak, for memory no bar, since there is nothing
 *   to take a share of;
 * - a metric and a bound: the amount, and the graph or bar against the bound.
 */
function PodUsageCell({
  as,
  used,
  history,
  usage,
  format,
  what,
}: {
  as: "graph" | "bar";
  used: number | undefined;
  /** The readings so far, oldest first, and when each was taken. Only the
   *  graph draws them. */
  history?: { at: number; value: number }[];
  usage: PodResourceUsage | null;
  format: (value: number) => string;
  what: string;
}) {
  if (used == null) return <div className={USAGE_CELL}>—</div>;
  const amount = format(used);
  const share = usage === null ? null : `${Math.round(usage.percent)}%`;
  const against = usage === null ? "no request or limit set" : `${share} of ${format(usage.bound)} ${usage.of}`;
  // By load against a limit: near it is near being throttled or killed.
  // Against a request it is not a verdict at all, so one quiet tone however
  // far past it goes.
  const tone = usage === null ? "muted" : usage.of === "request" ? "info" : loadTone(usage.percent);
  if (as === "bar") {
    return (
      <div className={USAGE_CELL} title={`${amount}, ${against}`}>
        <span className="num w-[4.75rem] shrink-0 text-right">{amount}</span>
        {usage !== null && (
          <span className="min-w-0 flex-1">
            {/* Unrounded and unclamped: `Meter` clamps the bar and rounds what
                it shows, and a pod past its bound must not be drawn the same
                as one exactly at it. */}
            <Meter value={usage.percent} tone={tone} ariaLabel={`${what}, of ${usage.of}`} />
          </span>
        )}
      </div>
    );
  }
  // The figure itself when nothing older is held, so the graph is never empty
  // beside a reading.
  const drawn = history && history.length > 0 ? history : [{ at: 0, value: used }];
  const elapsed = drawn[drawn.length - 1].at - drawn[0].at;
  // What the graph actually covers, which is less than ten minutes until the
  // list has been open that long — and said as it is, not as the most it
  // could be.
  const over = elapsed > 0 ? ` over the last ${spanWords(elapsed)}` : "";
  return (
    <div className={USAGE_CELL} title={`${amount}, ${against}`}>
      <span className="num w-[4.75rem] shrink-0 text-right">{amount}</span>
      <span className="min-w-0 flex-1">
        <Sparkline
          points={drawn.map((s) => s.value)}
          // Placed by when they were read: a stretch with no readings is as
          // wide as it lasted, not one step like any other.
          at={drawn.map((s) => s.at)}
          height={USAGE_GRAPH_HEIGHT}
          // The line alone. The wash under it fills the box whenever the line
          // runs high, and a tinted block in a table cell reads as a frame.
          fill={false}
          ceiling={usage?.bound}
          tone={tone}
          ariaLabel={`${what}${over}, now ${amount}, ${against}`}
        />
      </span>
      <span className="num w-9 shrink-0 text-right text-[0.6875rem] text-muted">{share}</span>
    </div>
  );
}

/** Short enough to sit inside a row at every density without growing it. */
const USAGE_GRAPH_HEIGHT = 18;


/**
 * The design's unhealthy dot for a pod, and the pill beside it: both read
 * core's `podStatus`, which is the same function the detail header asks about
 * the same pod. Nothing here restates a rule, so a row and a header cannot
 * disagree — they once did, on a crash-looping pod, because this read
 * `row.phase` alone and a pod in `CrashLoopBackOff` still reports "Running".
 */
export const podFlagged = (row: PodRow): boolean => podStatus(row).flagged;

export const podColumns: Column<PodRow>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  // Kept in the list contract so a Node detail's "View all" hand-off can
  // scope this screen to `spec.nodeName` instead of writing a filter key no
  // column can read. Visible on purpose: a scoped search must say what it is
  // scoped by, and hiding this column would make useResourceTabView discard
  // the filter as soon as the Pods screen mounts.
  {
    key: "node",
    header: "Node",
    sortable: true,
    // The name is the way to the node (#822). A pod not yet scheduled has
    // none, and a dash that opened nothing would be a link to nowhere.
    render: (p) => (p.node ? <NodeLink name={p.node} /> : "—"),
  },
  // One square per container, in place of the `1/2` the Ready column printed
  // (#878): which container, in what state. Sorted worst first, so the pods
  // with a container in trouble gather at one end; searched by the names,
  // states and reasons the squares stand for, since a square holds no text.
  {
    key: "containers",
    header: "Containers",
    sortable: true,
    render: (p) => <ContainerBlocks containers={p.containers} fallback={p.ready} />,
    getSortValue: (p) => containersSortValue(p.containers ?? []),
    getValue: (p) =>
      p.containers && p.containers.length > 0 ? p.containers.map(describeContainer).join("; ") : p.ready,
  },
  {
    key: "phase", header: "Status", sortable: true,
    render: (p) => {
      const { status, health } = podStatus(p);
      return <StatusPill status={status} kind={health} />;
    },
    // Sorts on what the pill shows, not on the raw phase underneath it:
    // otherwise every waiting pod scatters under "Pending" and "Running"
    // instead of grouping with the other pods in the same trouble.
    getSortValue: (p) => podStatus(p).status,
  },
  { key: "restarts", header: "Restarts", sortable: true, align: "end" },
  // The amount, with CPU as a ten-minute graph and memory as a bar (#864).
  // Sorted by the
  // amount, as before: "which pod is using the most" is the question the
  // column was already answering, and a pod with no bound has no percentage
  // to sort by.
  {
    key: "cpu",
    header: "CPU",
    sortable: true,
    minWidth: USAGE_COLUMN_MIN_WIDTH,
    render: (p) => (
      <PodUsageCell
        as="graph"
        used={p.cpu}
        history={ownHistory(p).map((s) => ({ at: s.at, value: s.cpu }))}
        usage={podLoad(p).cpu}
        format={formatCpu}
        what={`${p.name} CPU`}
      />
    ),
    getSortValue: (p) => metricSort(p.cpu),
  },
  {
    key: "memory",
    header: "Memory",
    sortable: true,
    minWidth: USAGE_COLUMN_MIN_WIDTH,
    render: (p) => (
      <PodUsageCell as="bar" used={p.memory} usage={podLoad(p).memory} format={formatMemory} what={`${p.name} memory`} />
    ),
    getSortValue: (p) => metricSort(p.memory),
  },
  // #405: derived here against a ticking clock, from the summary's
  // `created` timestamp — the backend's `age` string is rendered once per
  // watch event and freezes for an object nothing is changing.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
  // Not sortable: a comma-joined list of container images (PodSummary.image)
  // has no single natural order, and the design mock renders a plain header
  // for it — no SortHeader. Left filterable-unset like every other column
  // here, so it still joins the toolbar's whole-row search.
  { key: "image", header: "Image", sortable: false, render: (p) => p.image || "—" },
];

/**
 * "N/M" as the two numbers behind it — how Deployment and StatefulSet both
 * report readiness, where DaemonSet reports a pair of bare numbers.
 *
 * An unparseable string yields `NaN`s, which {@link scaledStatus} reads as
 * neither zero-desired nor short — the same "no dot" answer the previous
 * `readyShort` gave for the same input.
 */
function readyCounts(ready: string): [ready: number, desired: number] {
  const [have, want] = ready.split("/").map(Number);
  return [have, want];
}

/**
 * The verdict for one workload row: its status word, the tone that word is
 * drawn in, and whether it earns the unhealthy dot — all three from core's
 * {@link scaledStatus}, which is the same function the detail header asks
 * about the same object.
 *
 * This is what the design's Workloads table got wrong. It kept its own table
 * pairing "Progressing" with amber and "Available" with green, so a degraded
 * Deployment read amber "Progressing" in the row and red "Degraded" in the
 * header a double-click away, with the row's own red dot beside the amber
 * word. One reading cannot disagree with itself. (#331)
 */
export const deploymentVerdict = (row: DeploymentSummary): StatusVerdict =>
  scaledStatus("Deployment", ...readyCounts(row.ready));

/** The design's unhealthy dot for a Deployment: fewer ready than desired. */
export const deploymentFlagged = (row: DeploymentSummary): boolean => deploymentVerdict(row).flagged;

export const deploymentColumns: Column<DeploymentSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "ready", header: "Ready", align: "end" },
  { key: "upToDate", header: "Up-to-date", sortable: true, align: "end" },
  { key: "available", header: "Available", sortable: true, align: "end" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

/** A StatefulSet's verdict — the same rule, off the same "N/M" string. */
export const statefulSetVerdict = (row: StatefulSetSummary): StatusVerdict =>
  scaledStatus("StatefulSet", ...readyCounts(row.ready));

/** The design's unhealthy dot for a StatefulSet: fewer ready than desired. */
export const statefulSetFlagged = (row: StatefulSetSummary): boolean => statefulSetVerdict(row).flagged;

export const statefulSetColumns: Column<StatefulSetSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "ready", header: "Ready", align: "end" },
  { key: "updated", header: "Updated", sortable: true, align: "end" },
  { key: "service", header: "Service", sortable: true, render: (s) => s.service || "—" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

/** A DaemonSet's verdict — numeric fields here, unlike Deployment/StatefulSet's
 *  "N/M" string, and its own zero word ("Not scheduled") which core supplies. */
export const daemonSetVerdict = (row: DaemonSetSummary): StatusVerdict =>
  scaledStatus("DaemonSet", row.ready, row.desired);

/** The design's unhealthy dot for a DaemonSet: fewer ready than desired. */
export const daemonSetFlagged = (row: DaemonSetSummary): boolean => daemonSetVerdict(row).flagged;

export const daemonSetColumns: Column<DaemonSetSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "desired", header: "Desired", sortable: true, align: "end" },
  { key: "current", header: "Current", sortable: true, align: "end" },
  { key: "ready", header: "Ready", sortable: true, align: "end" },
  { key: "upToDate", header: "Up-to-date", sortable: true, align: "end" },
  { key: "available", header: "Available", sortable: true, align: "end" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

/** A Job's verdict, through core's own rule: a failure outranks an in-flight
 *  pod, and a running Job is amber without earning a dot. */
export const jobVerdict = (row: JobSummary): StatusVerdict => jobStatus(row.failed, row.active);

/** The design's unhealthy dot for a Job: any failed pod. Unambiguous — the
 *  same `failed` count already drives the Status column's red pill below. */
export const jobFlagged = (row: JobSummary): boolean => jobVerdict(row).flagged;

export const jobColumns: Column<JobSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "completions", header: "Completions", align: "end" },
  {
    key: "status",
    header: "Status",
    render: (j) => {
      const { status, health } = jobVerdict(j);
      return <StatusPill status={status} kind={health} />;
    },
  },
  { key: "duration", header: "Duration", align: "end", render: (j) => j.duration || "—" },
  { key: "owner", header: "Owner", render: (j) => j.owner || "—" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

/** A CronJob's verdict: suspended or not, which is the whole of its health —
 *  core deliberately gives it no unhealthy state, the Jobs it spawns have it. */
export const cronJobVerdict = (row: CronJobSummary): StatusVerdict => cronJobStatus(row.suspended);

export const cronJobColumns: Column<CronJobSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "schedule", header: "Schedule" },
  {
    key: "suspended",
    header: "State",
    render: (c) => {
      const { status, health } = cronJobVerdict(c);
      return <StatusPill status={status} kind={health} />;
    },
  },
  { key: "active", header: "Active", align: "end" },
  { key: "lastSchedule", header: "Last run", render: (c) => c.lastSchedule || "—" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

/** "warning" / "neutral" classic badge variants, remapped onto the kit's `Tone`. */
const BADGE_TONE: Record<string, Tone> = { warning: "warn", neutral: "muted" };

/**
 * A Node's verdict, off the two facts a row carries: the readiness word the
 * backend already derived, and whether the node is cordoned.
 *
 * Its `flagged` is what the design's unhealthy dot and the row's `Ask` chip
 * turn on. Without it a NotReady node's row asked "What is X using right
 * now?" while its own detail pane — reading `resourceStatusLine`, the same
 * verdict — asked "Why is X unhealthy?". The pane's read is the right one.
 * (#331)
 */
export const nodeVerdict = (row: NodeRow): StatusVerdict => nodeStatus(row.status, row.unschedulable);

export const nodeFlagged = (row: NodeRow): boolean => nodeVerdict(row).flagged;

/**
 * The Tainted badge, carrying the count. A node with no taints renders no
 * badge at all, exactly as before — the caller guards on that.
 *
 * The wrapper, not the Badge, holds the hint and the accessible name: `Badge`
 * is a presentational span shared with five other callers, and widening its
 * props for one of them is how a kit component turns into a grab bag. `title`
 * is the tooltip host here rather than the kit's `Tooltip` because the content
 * is several lines and `Tooltip` renders a single nowrap line; `tabIndex`
 * keeps it reachable by keyboard rather than hover alone. (#426)
 */
function TaintBadge({ row }: { row: NodeRow }) {
  const taints = row.taintDetails ?? [];
  return (
    <span tabIndex={0} title={taintTooltip(taints)} aria-label={taintBadgeLabel(row.taints)}>
      <Badge tone={BADGE_TONE.neutral}>{taintBadgeText(row.taints)}</Badge>
    </span>
  );
}

/** The optional Taints column's cell: the per-effect tally, `0 / 0 / 0` for a
 *  node with none — a number, not a blank, so the column reads as a column. */
function TaintTally({ row }: { row: NodeRow }) {
  const taints = row.taintDetails ?? [];
  return (
    <span title={taints.length > 0 ? taintTooltip(taints) : TAINT_COLUMN_HINT}>
      {taintTallyText(taints)}
    </span>
  );
}

/**
 * A node's CPU and memory as a share of what it can allocate — core's
 * `nodeUsage`, the one Overview's node rows read, so the two screens cannot
 * come to mean different things by a percentage.
 *
 * The list row carries the metric's two figures flattened onto it (`cpu`,
 * `memory`); they arrive together or not at all, so one missing is no
 * reading for either.
 */
function nodeLoad(n: NodeRow) {
  const metric =
    n.cpu == null || n.memory == null ? undefined : { name: n.name, cpuMillicores: n.cpu, memoryMiB: n.memory };
  return nodeUsage(n, metric, undefined);
}

/**
 * One node's CPU or memory in the Nodes list: the amount, and the same bar
 * Overview draws for it (#830).
 *
 * The amount alone does not say whether a node is busy — `1.6 Gi` is nothing
 * on a 64 Gi node and nearly all of a 2 Gi one — and this is the screen a
 * reader comes to in order to compare nodes. The bar is the share of the
 * node's allocatable capacity in use, tinted by load, with the percentage
 * beside it.
 *
 * The amount stays, in front of the bar, because it is what the column showed
 * before and what a reader sizing a workload still wants; the capacity it is
 * measured against is in the tooltip (`0.19 cores of 12 cores`).
 *
 * Three states, kept apart:
 * - no metric (no metrics-server, or a node that joined since the last
 *   scrape): a dash, and no bar — an empty bar would read as an idle node;
 * - a metric, but the node reports no allocatable capacity: the amount, and
 *   no bar, since there is nothing to take a share of;
 * - both: the amount and the bar.
 */
function NodeUsageCell({
  used,
  allocatable,
  percent,
  format,
  what,
}: {
  used: number | undefined;
  allocatable: number;
  percent: number | null;
  format: (value: number) => string;
  what: string;
}) {
  if (used == null) return <div className={USAGE_CELL}>—</div>;
  const amount = format(used);
  return (
    <div className={USAGE_CELL} title={percent === null ? undefined : `${amount} of ${format(allocatable)}`}>
      <span className="num w-[4.75rem] shrink-0 text-right">{amount}</span>
      {percent !== null && (
        <span className="min-w-0 flex-1">
          {/* Unrounded and unclamped, as on Overview: `Meter` clamps the bar
              and rounds what it shows, and a node over its allocatable must
              not be drawn the same as one exactly at it. */}
          <Meter value={percent} ariaLabel={what} />
        </span>
      )}
    </div>
  );
}

export const nodeColumns: Column<NodeRow>[] = [
  { key: "name", header: "Name", sortable: true },
  {
    key: "status",
    header: "Status",
    sortable: true,
    // The tone comes from `nodeVerdict`, NOT from `phaseKind(n.status)`. The
    // word and the badges are the mock's, unchanged; only the tone moved. A
    // cordoned-but-Ready node is `warning`+flagged in core, so reading the
    // phase alone drew a GREEN "Ready" pill beside the red needs-attention dot
    // `withRowAffordances` had just given the same row — the exact pairing
    // `k8sStatus`'s own header says one reading exists to prevent. The dot and
    // the pill are two channels of one verdict again. (#331)
    render: (n) => (
      <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
        <StatusPill status={n.status} kind={nodeVerdict(n).health} />
        {n.unschedulable && <Badge tone={BADGE_TONE.warning}>SchedulingDisabled</Badge>}
        {n.taints > 0 && <TaintBadge row={n} />}
      </span>
    ),
  },
  { key: "roles", header: "Roles" },
  {
    key: "cpu",
    header: "CPU",
    sortable: true,
    minWidth: USAGE_COLUMN_MIN_WIDTH,
    // By the share of the node in use, which is what the bar draws: sorted on
    // the amount, a small node at 90% sat below a large one at 20%, and the
    // column's order contradicted its own bars.
    getSortValue: (n) => metricSort(nodeLoad(n).cpuPercent ?? undefined),
    render: (n) => (
      <NodeUsageCell
        used={n.cpu}
        allocatable={n.allocatableCpuMillicores}
        percent={nodeLoad(n).cpuPercent}
        format={formatNodeCpu}
        what={`${n.name} CPU`}
      />
    ),
  },
  {
    key: "memory",
    header: "Memory",
    sortable: true,
    minWidth: USAGE_COLUMN_MIN_WIDTH,
    getSortValue: (n) => metricSort(nodeLoad(n).memoryPercent ?? undefined),
    render: (n) => (
      <NodeUsageCell
        used={n.memory}
        allocatable={n.allocatableMemoryMiB}
        percent={nodeLoad(n).memoryPercent}
        format={formatMemory}
        what={`${n.name} memory`}
      />
    ),
  },
  { key: "version", header: "Version" },
  {
    key: "taints",
    header: "Taints",
    sortable: true,
    align: "end",
    // Off until asked for: a tally is what you go looking for when a pod will
    // not schedule, and noise on every other day. #411's Namespace columns are
    // the same bargain.
    defaultHidden: true,
    render: (n) => <TaintTally row={n} />,
    getSortValue: (n) => taintSortValue(n.taintDetails),
  },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

const AUTOMATIC_NAMESPACE_LABEL = "kubernetes.io/metadata.name";
const VISIBLE_NAMESPACE_LABELS = 2;

function namespaceLabelEntries(labels: Record<string, string>): [string, string][] {
  return Object.entries(labels)
    .filter(([key]) => key !== AUTOMATIC_NAMESPACE_LABEL)
    .sort(([left], [right]) => left.localeCompare(right));
}

function namespaceLabelText(namespace: NamespaceSummary, separator: string): string {
  return namespaceLabelEntries(namespace.labels)
    .map(([key, value]) => `${key}=${value}`)
    .join(separator);
}

function NamespaceLabelChips({ namespace }: { namespace: NamespaceSummary }) {
  const entries = namespaceLabelEntries(namespace.labels);
  if (entries.length === 0) return "—";
  const hidden = entries.length - VISIBLE_NAMESPACE_LABELS;
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
      {entries.slice(0, VISIBLE_NAMESPACE_LABELS).map(([key, value]) => (
        <Badge key={key} tone="muted">{`${key}=${value}`}</Badge>
      ))}
      {hidden > 0 && (
        <Badge tone="muted">
          <Tooltip label={namespaceLabelText(namespace, ", ")}>{`+${hidden}`}</Tooltip>
        </Badge>
      )}
    </span>
  );
}

export const namespaceColumns: Column<NamespaceSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  {
    key: "phase",
    header: "Status",
    sortable: true,
    filterable: true,
    minWidth: 132,
    render: (namespace) => (
      <StatusPill status={namespace.phase} kind={phaseKind(namespace.phase)} />
    ),
  },
  {
    key: "labels",
    header: "Labels",
    sortable: false,
    minWidth: 280,
    render: (namespace) => <NamespaceLabelChips namespace={namespace} />,
    getValue: (namespace) => namespaceLabelText(namespace, " "),
  },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

export const configMapColumns: Column<ConfigMapSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "keys", header: "Keys", sortable: true, align: "end", render: (c) => String(c.keys) },
  // #405: derived here against a ticking clock, from the summary's
  // `created` timestamp — the backend's `age` string is rendered once per
  // watch event and freezes for an object nothing is changing.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

export const secretColumns: Column<SecretSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "type", header: "Type" },
  { key: "keys", header: "Keys", sortable: true, align: "end", render: (s) => String(s.keys) },
  // #405: derived here against a ticking clock, from the summary's
  // `created` timestamp — the backend's `age` string is rendered once per
  // watch event and freezes for an object nothing is changing.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

export const resourceQuotaColumns: Column<ResourceQuotaSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "resources", header: "Resources", sortable: true, align: "end" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

export const limitRangeColumns: Column<LimitRangeSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "limits", header: "Limits", sortable: true, align: "end" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

export const serviceColumns: Column<ServiceSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "type", header: "Type" },
  { key: "clusterIP", header: "Cluster IP" },
  { key: "externalIP", header: "External IP", render: (s) => s.externalIP || "—" },
  { key: "ports", header: "Ports" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

export const ingressColumns: Column<IngressSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "class", header: "Class" },
  { key: "hosts", header: "Hosts", render: (i) => i.hosts || "*" },
  { key: "address", header: "Address", render: (i) => i.address || "—" },
  { key: "ports", header: "Ports" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

export const endpointSliceColumns: Column<EndpointSliceSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "addressType", header: "Address Type" },
  { key: "endpoints", header: "Endpoints", align: "end" },
  { key: "ports", header: "Ports", render: (e) => e.ports || "—" },
  { key: "service", header: "Service", render: (e) => e.service || "—" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

export const networkPolicyColumns: Column<NetworkPolicySummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "podSelector", header: "Pod Selector" },
  { key: "ingress", header: "Ingress", sortable: true, align: "end" },
  { key: "egress", header: "Egress", sortable: true, align: "end" },
  { key: "policyTypes", header: "Policy Types", render: (n) => n.policyTypes || "—" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

export const pvcColumns: Column<PvcSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  {
    key: "status", header: "Status", sortable: true,
    render: (p) => <StatusPill status={p.status} kind={phaseKind(p.status === "Bound" ? "Ready" : p.status)} />,
  },
  { key: "capacity", header: "Capacity", align: "end", render: (p) => formatStorageSize(p.capacity) },
  { key: "accessModes", header: "Access Modes", render: (p) => p.accessModes || "—" },
  { key: "storageClass", header: "Storage Class", render: (p) => p.storageClass || "—" },
  { key: "volume", header: "Volume", render: (p) => p.volume || "—" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

export const pvColumns: Column<PvSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "capacity", header: "Capacity", align: "end", render: (p) => formatStorageSize(p.capacity) },
  { key: "accessModes", header: "Access Modes", render: (p) => p.accessModes || "—" },
  { key: "reclaimPolicy", header: "Reclaim", render: (p) => p.reclaimPolicy || "—" },
  {
    key: "status", header: "Status", sortable: true,
    render: (p) => (
      <StatusPill status={p.status} kind={phaseKind(p.status === "Bound" || p.status === "Available" ? "Ready" : p.status)} />
    ),
  },
  { key: "claim", header: "Claim", render: (p) => p.claim || "—" },
  { key: "storageClass", header: "Storage Class", render: (p) => p.storageClass || "—" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

export const storageClassColumns: Column<StorageClassSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "provisioner", header: "Provisioner" },
  { key: "reclaimPolicy", header: "Reclaim", render: (s) => s.reclaimPolicy || "—" },
  { key: "volumeBindingMode", header: "Binding Mode", render: (s) => s.volumeBindingMode || "—" },
  { key: "default", header: "Default", render: (s) => (s.default ? <StatusPill status="Default" kind="success" /> : "—") },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

export const serviceAccountColumns: Column<ServiceAccountSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "secrets", header: "Secrets", sortable: true, align: "end" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

export const roleColumns: Column<RoleSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "rules", header: "Rules", sortable: true, align: "end" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

export const clusterRoleColumns: Column<ClusterRoleSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "rules", header: "Rules", sortable: true, align: "end" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

export const roleBindingColumns: Column<RoleBindingSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "role", header: "Role" },
  { key: "subjects", header: "Subjects", sortable: true, align: "end" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];

export const clusterRoleBindingColumns: Column<ClusterRoleBindingSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "role", header: "Role" },
  { key: "subjects", header: "Subjects", sortable: true, align: "end" },
  // #405: live age, derived against a ticking clock from `created`.
  { key: "age", header: "Age", sortable: true, align: "end", render: (r) => <AgeCell created={r.created} age={r.age} />, getSortValue: ageSortValue },
];
