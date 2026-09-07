import { invokeCapability, type Invoker } from "../transport/transport";
import type { PodSummary } from "./workloads";

/** How many pods one node is running. */
export interface NodePodCount {
  node: string;
  pods: number;
}

/**
 * The cluster overview's three pod facts, from one backend call that never
 * lists the cluster's pod bodies — see `crates/kube/src/pod_overview.rs` for
 * how each is fetched, and why `k8s.listPods` cannot serve them.
 */
export interface PodOverview {
  /** Every pod in the cluster, whatever phase it is in. */
  total: number;
  /**
   * One entry per node running at least one pod. **Complete**: a node absent
   * from this list runs no pods, and that is a reading rather than a gap —
   * which is what lets a node's Pods column show a truthful `0`.
   */
  byNode: NodePodCount[];
  /**
   * Every pod that is not simply running — a SUPERSET of the unhealthy ones,
   * and deliberately so. `Succeeded` pods are in it and `podStatus` flags none
   * of them. Whether a pod needs attention stays core's to decide, from the
   * `phase` and `waitingReason` on each summary; nothing upstream judges.
   */
  unsettled: PodSummary[];
  /**
   * Whether {@link unsettled} is shorter than the truth — the backend's cap
   * bit, or a pod it singled out could not be read. A short list presented as
   * a whole one is the failure the cap would otherwise introduce.
   */
  truncated: boolean;
}

export interface PodOverviewOutcome {
  pods?: PodOverview;
  error?: string;
}

/**
 * Load a cluster's pod totals, per-node counts and not-running pods through
 * the `k8s.podOverview` capability.
 *
 * A failure surfaces as `error` with no `pods` at all, never as a `total` of
 * zero: a cluster that did not answer has not told us it has no pods.
 */
export async function podOverview(
  context: string,
  invoke: Invoker = invokeCapability,
): Promise<PodOverviewOutcome> {
  try {
    const pods = await invoke<PodOverview>("k8s.podOverview", { context });
    return { pods };
  } catch (e) {
    return { error: String(e) };
  }
}
