import { invokeCapability, type Invoker } from "../transport/transport";

/**
 * Fleet's per-cluster pod tally: pods in the `Running` phase over every pod
 * that is still meant to be running. A liveness figure, not a health one — a
 * `Running` pod can still be crash-looping.
 */
export interface PodCount {
  running: number;
  /**
   * **Excludes `Succeeded` pods**, and nothing else. A completed Job's pods
   * stay in the cluster until something reaps them, and they are done rather
   * than down: counting them would make a cluster whose Jobs all finished read
   * as one with pods missing. `Failed` stays counted — a failed pod genuinely
   * is a problem worth seeing. The exclusion is server-side, in the backend's
   * field selector; see `STILL_RUNNING` in `crates/kube/src/pod_count.rs`.
   */
  total: number;
}

export interface PodCountOutcome {
  counts?: PodCount;
  error?: string;
}

/**
 * Load a cluster's running/total pod counts via the `k8s.podCount`
 * capability. The backend counts without listing pod bodies and carries its
 * own short timeout (3s — see `POD_COUNT_TIMEOUT` in `crates/kube/src/metrics.rs`),
 * so a slow or unreachable cluster surfaces as `error`, never as `counts`
 * with zeros: a cluster that didn't answer has not told us it has no pods.
 */
export async function podCount(
  context: string,
  invoke: Invoker = invokeCapability,
): Promise<PodCountOutcome> {
  try {
    const counts = await invoke<PodCount>("k8s.podCount", { context });
    return { counts };
  } catch (e) {
    return { error: String(e) };
  }
}
