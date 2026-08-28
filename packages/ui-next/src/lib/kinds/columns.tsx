import {
  ageSortValue,
  cronJobStatus,
  formatStorageSize,
  jobStatus,
  nodeStatus,
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
} from "@srelens/core";
import { Badge, StatusPill, type Column, type Tone } from "@srelens/ui-kit";

export type PodRow = PodSummary & { cpu?: number; memory?: number };
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
  { key: "ready", header: "Ready", align: "end" },
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
  { key: "cpu", header: "CPU", sortable: true, align: "end", render: (p) => metric(p.cpu, formatCpu), getSortValue: (p) => metricSort(p.cpu) },
  { key: "memory", header: "Memory", sortable: true, align: "end", render: (p) => metric(p.memory, formatMemory), getSortValue: (p) => metricSort(p.memory) },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
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
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
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
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
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
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
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
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
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
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
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
        {n.taints > 0 && (
          <Badge tone={BADGE_TONE.neutral}>{n.taints > 1 ? `Tainted (${n.taints})` : "Tainted"}</Badge>
        )}
      </span>
    ),
  },
  { key: "roles", header: "Roles" },
  { key: "cpu", header: "CPU", sortable: true, align: "end", render: (n) => metric(n.cpu, formatCpu), getSortValue: (n) => metricSort(n.cpu) },
  { key: "memory", header: "Memory", sortable: true, align: "end", render: (n) => metric(n.memory, formatMemory), getSortValue: (n) => metricSort(n.memory) },
  { key: "version", header: "Version" },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

export const configMapColumns: Column<ConfigMapSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "keys", header: "Keys", sortable: true, align: "end", render: (c) => String(c.keys) },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

export const secretColumns: Column<SecretSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "type", header: "Type" },
  { key: "keys", header: "Keys", sortable: true, align: "end", render: (s) => String(s.keys) },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

export const resourceQuotaColumns: Column<ResourceQuotaSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "resources", header: "Resources", sortable: true, align: "end" },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

export const limitRangeColumns: Column<LimitRangeSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "limits", header: "Limits", sortable: true, align: "end" },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

export const serviceColumns: Column<ServiceSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "type", header: "Type" },
  { key: "clusterIP", header: "Cluster IP" },
  { key: "externalIP", header: "External IP", render: (s) => s.externalIP || "—" },
  { key: "ports", header: "Ports" },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

export const ingressColumns: Column<IngressSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "class", header: "Class" },
  { key: "hosts", header: "Hosts", render: (i) => i.hosts || "*" },
  { key: "address", header: "Address", render: (i) => i.address || "—" },
  { key: "ports", header: "Ports" },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

export const endpointSliceColumns: Column<EndpointSliceSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "addressType", header: "Address Type" },
  { key: "endpoints", header: "Endpoints", align: "end" },
  { key: "ports", header: "Ports", render: (e) => e.ports || "—" },
  { key: "service", header: "Service", render: (e) => e.service || "—" },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

export const networkPolicyColumns: Column<NetworkPolicySummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "podSelector", header: "Pod Selector" },
  { key: "ingress", header: "Ingress", sortable: true, align: "end" },
  { key: "egress", header: "Egress", sortable: true, align: "end" },
  { key: "policyTypes", header: "Policy Types", render: (n) => n.policyTypes || "—" },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
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
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
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
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

export const storageClassColumns: Column<StorageClassSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "provisioner", header: "Provisioner" },
  { key: "reclaimPolicy", header: "Reclaim", render: (s) => s.reclaimPolicy || "—" },
  { key: "volumeBindingMode", header: "Binding Mode", render: (s) => s.volumeBindingMode || "—" },
  { key: "default", header: "Default", render: (s) => (s.default ? <StatusPill status="Default" kind="success" /> : "—") },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

export const serviceAccountColumns: Column<ServiceAccountSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "secrets", header: "Secrets", sortable: true, align: "end" },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

export const roleColumns: Column<RoleSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "rules", header: "Rules", sortable: true, align: "end" },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

export const clusterRoleColumns: Column<ClusterRoleSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "rules", header: "Rules", sortable: true, align: "end" },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

export const roleBindingColumns: Column<RoleBindingSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "namespace", header: "Namespace", sortable: true },
  { key: "role", header: "Role" },
  { key: "subjects", header: "Subjects", sortable: true, align: "end" },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];

export const clusterRoleBindingColumns: Column<ClusterRoleBindingSummary>[] = [
  { key: "name", header: "Name", sortable: true },
  { key: "role", header: "Role" },
  { key: "subjects", header: "Subjects", sortable: true, align: "end" },
  { key: "age", header: "Age", sortable: true, align: "end", getSortValue: ageSortValue },
];
