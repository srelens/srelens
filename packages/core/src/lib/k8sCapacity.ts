import type { NodeSummary, NodeMetric } from "./manifest";
import type { PodSummary } from "./workloads";

/**
 * A node's usage against its allocatable capacity, computed once so the
 * cluster-overview screen, the nodes list and the status bar's usage readout
 * can never disagree about what a percentage means.
 *
 * `null` is not `0`. A cluster with no metrics-server has no numerator, and
 * `0%` is a measurement — it says the cluster is idle. Absence must read as
 * "no reading", never as an empty (0%) meter. The same holds for `pods`: an
 * unknown count is `null`, not `{ used: 0, ... }`.
 */
export interface NodeUsage {
  cpuPercent: number | null;
  memoryPercent: number | null;
  pods: { used: number; allocatable: number } | null;
}

/**
 * `usage / allocatable`, as a percentage — deliberately unrounded.
 *
 * Not clamped to 100. A pod exceeding its request, or a node under memory
 * pressure, genuinely uses more than it was allocated, and that is exactly
 * the case a reader needs to see. The kit's `Meter` already clamps the bar
 * it draws while keeping `aria-valuetext` truthful, so clamping here would
 * lie in the one direction that hides a problem: a node at 140% would
 * silently render as a full bar reading 100%, indistinguishable from a node
 * exactly at its limit. Do not "tidy" this into a clamp.
 *
 * Rounding is left to the caller too — `Meter` rounds only what it shows and
 * keeps full precision internally for its own clamping math (see its
 * comment); rounding here first would throw that precision away before it
 * gets there.
 */
function percentOf(used: number, allocatable: number): number | null {
  if (allocatable === 0) return null;
  return (used / allocatable) * 100;
}

/**
 * A single node's usage against its own allocatable capacity.
 *
 * @param metric - This node's reading from `nodeMetrics`, or `undefined` if
 *   metrics-server is absent, or the node joined since the last scrape.
 * @param podsOnNode - The number of pods scheduled on this node, or
 *   `undefined` if that count is not known to the caller.
 */
export function nodeUsage(
  node: NodeSummary,
  metric: NodeMetric | undefined,
  podsOnNode: number | undefined,
): NodeUsage {
  return {
    cpuPercent: metric ? percentOf(metric.cpuMillicores, node.allocatableCpuMillicores) : null,
    memoryPercent: metric ? percentOf(metric.memoryMiB, node.allocatableMemoryMiB) : null,
    pods: podsOnNode === undefined ? null : { used: podsOnNode, allocatable: node.allocatablePods },
  };
}

/** What a pod's usage is measured against. */
export type PodBound = "limit" | "request";

/** One resource of one pod: how much of its bound is in use, and which bound. */
export interface PodResourceUsage {
  /** Unrounded and unclamped, for the reason {@link percentOf} gives. */
  percent: number;
  of: PodBound;
  /** The bound itself, in the unit usage is reported in. */
  bound: number;
}

/**
 * A pod's usage against what it was given, computed once so the pods list and
 * a pod's own page cannot come to mean different things by a percentage.
 *
 * `null` for a resource is "nothing to measure against" or "no reading" —
 * never `0%`, which is a measurement.
 */
export interface PodUsage {
  cpu: PodResourceUsage | null;
  memory: PodResourceUsage | null;
}

/** The request and limit fields of a pod summary that {@link podUsage} reads. */
export type PodBounds = Pick<
  PodSummary,
  "cpuReqMillicores" | "cpuLimMillicores" | "memReqMiB" | "memLimMiB" | "cpuLimAll" | "memLimAll"
>;

function against(
  used: number,
  limit: number | undefined,
  limitIsWhole: boolean | undefined,
  request: number | undefined,
): PodResourceUsage | null {
  // The limit first: it is the line past which the pod is throttled or
  // killed, which is the thing a reader scanning a list wants to see coming.
  // Only when it is the pod's WHOLE ceiling — see `PodSummary.cpuLimAll`.
  const [bound, of]: [number, PodBound] =
    limitIsWhole === true && (limit ?? 0) > 0 ? [limit!, "limit"] : [request ?? 0, "request"];
  const percent = percentOf(used, bound);
  return percent === null ? null : { percent, of, bound };
}

/**
 * A single pod's usage against its own limit, or its request where it has no
 * whole limit.
 *
 * A node's bar is a share of what the node can allocate. A pod has no such
 * single number: it has what it asked for and what it may not pass, either of
 * which may be missing. A pod with neither has nothing to take a share of, and
 * says so with `null` rather than an empty or a full bar.
 *
 * Against a request the figure passes 100 routinely and that is not a fault —
 * a request is a floor for scheduling, not a ceiling. `of` travels with the
 * percentage so a caller can say which it is showing.
 *
 * @param metric - This pod's reading, or `undefined` if metrics-server is
 *   absent or has no sample for it yet.
 */
export function podUsage(
  pod: PodBounds,
  metric: { cpuMillicores: number; memoryMiB: number } | undefined,
): PodUsage {
  if (!metric) return { cpu: null, memory: null };
  return {
    cpu: against(metric.cpuMillicores, pod.cpuLimMillicores, pod.cpuLimAll, pod.cpuReqMillicores),
    memory: against(metric.memoryMiB, pod.memLimMiB, pod.memLimAll, pod.memReqMiB),
  };
}

/**
 * Cluster-wide totals for the capacity strip: `312 / 460 cores`, `1.4 / 1.9
 * TiB`. `null` when nothing can be said — an empty cluster, or one with no
 * metrics-server anywhere.
 *
 * `nodesReporting` / `nodesTotal` travel with the sums because the sums are
 * partial whenever they differ: `cpu`/`memory` are computed only from nodes
 * that returned a metric (see `clusterCapacity`'s comment for why), so a
 * cluster where two of three nodes report gives a percentage over those two
 * — and a consumer that shows the resulting number without also showing
 * `nodesReporting` of `nodesTotal` is presenting a partial answer as if it
 * were a whole one. This was originally left for call sites to reassemble
 * from a separate count; that convention is exactly what let one screen
 * qualify a percentage while another didn't, so the qualifier now travels
 * with the number instead of being left to each call site's discipline.
 */
export interface ClusterCapacity {
  cpu: { usedMillicores: number; allocatableMillicores: number } | null;
  memory: { usedMiB: number; allocatableMiB: number } | null;
  /** How many nodes contributed a metric to the sums above. */
  nodesReporting: number;
  /** How many nodes there are in total, reporting or not. */
  nodesTotal: number;
}

/**
 * Sums usage and allocatable across nodes.
 *
 * The interesting case is a cluster where some nodes have a metric and
 * others do not (metrics-server races the node list; a node can also just be
 * down). This function's answer: a node without a metric is excluded
 * entirely, from *both* the numerator and the denominator, not folded in as
 * an allocatable node with zero usage. Adding its allocatable capacity to the
 * denominator alone would silently understate the percentage — inventing a
 * "used: 0" reading for a node nobody actually measured, which is the exact
 * null-is-not-zero mistake this module exists to prevent, just moved up one
 * level to the sum.
 *
 * The consequence: the total this returns describes only the nodes that
 * reported, not the whole cluster's capacity. `nodesReporting` and
 * `nodesTotal` carry that fact on the return value itself, so a caller
 * cannot show the percentage without the shortfall being right there to
 * show alongside it.
 *
 * When no node reports a metric, the sum would be over an empty set — that
 * is `null`, the same "no answer" rule as a single node, not a `0` total.
 * The counts are still reported in that case (`nodesReporting: 0`), since
 * they are not part of the "no answer" the sums represent.
 */
export function clusterCapacity(nodes: NodeSummary[], metrics: NodeMetric[]): ClusterCapacity {
  const byName = new Map(metrics.map((m) => [m.name, m]));

  let usedMillicores = 0;
  let allocatableMillicores = 0;
  let usedMiB = 0;
  let allocatableMiB = 0;
  let reporting = 0;

  for (const node of nodes) {
    const metric = byName.get(node.name);
    if (!metric) continue;
    reporting += 1;
    usedMillicores += metric.cpuMillicores;
    allocatableMillicores += node.allocatableCpuMillicores;
    usedMiB += metric.memoryMiB;
    allocatableMiB += node.allocatableMemoryMiB;
  }

  const nodesTotal = nodes.length;

  if (reporting === 0) {
    return { cpu: null, memory: null, nodesReporting: 0, nodesTotal };
  }

  return {
    cpu: { usedMillicores, allocatableMillicores },
    memory: { usedMiB, allocatableMiB },
    nodesReporting: reporting,
    nodesTotal,
  };
}
