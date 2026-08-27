import { invokeCapability, type Invoker } from "../transport/transport";

export interface ClusterContext {
  name: string;
  /** Identity that survives a rename (#265). The display `name` gains a
   *  `file/` prefix as soon as another kubeconfig declares the same context
   *  name, so anything persisted per context must key on this instead. */
  stableId: string;
  cluster: string;
  server: string;
  isCurrent: boolean;
  /**
   * Whether this context points at a local development cluster (kind, k3d,
   * minikube, docker-desktop, kiac, vind, …). Classified precision-first in the
   * Rust core: only a tool-generated name earns it and cloud auth always wins
   * as remote, so a production cluster is never marked local.
   */
  isLocal?: boolean;
  /** The detected local provider (e.g. "kind", "vind"), when `isLocal`. */
  provider?: string;
  /** The kubeconfig this context was declared in. */
  sourceFile: string;
  /** The credential MECHANISM — never the credential. See the Rust side. */
  authKind: string;
  /** The context's default namespace from the kubeconfig; empty/absent when unset. */
  namespace?: string;
}

export interface ContextsOutcome {
  contexts?: ClusterContext[];
  error?: string;
}

export interface ClusterInfo {
  context: string;
  reachable: boolean;
  version?: string | null;
  error?: string | null;
}

/**
 * Load the kube contexts via the `k8s.listContexts` capability, normalising
 * success/failure into a plain outcome. The invoker is injectable for tests.
 */
export async function listContexts(
  additionalPaths: string[] = [],
  invoke: Invoker = invokeCapability,
): Promise<ContextsOutcome> {
  try {
    const out = await invoke<{ contexts: ClusterContext[] }>("k8s.listContexts", { paths: additionalPaths });
    return { contexts: out.contexts };
  } catch (e) {
    return { error: String(e) };
  }
}

/**
 * Connect to a context via the `k8s.clusterInfo` capability and report the
 * server version / reachability. The capability never throws for an
 * unreachable cluster — it returns `reachable: false` with an error message —
 * but transport-level failures are caught and normalised here.
 */
export async function connectCluster(
  context: string,
  invoke: Invoker = invokeCapability,
): Promise<ClusterInfo> {
  try {
    return await invoke<ClusterInfo>("k8s.clusterInfo", { context });
  } catch (e) {
    return { context, reachable: false, error: String(e) };
  }
}

/**
 * Delete a context from its source kubeconfig file via the backend.
 */
export async function deleteContext(
  context: string,
  invoke: Invoker = invokeCapability,
): Promise<{ success: boolean }> {
  return invoke<{ success: boolean }>("k8s.deleteContext", { context });
}

/**
 * Whether the cluster serves `metrics.k8s.io`.
 *
 * `"absent"` is an answer and an important one — it means every meter on the
 * overview has no numerator. `"unknown"` means we could not ask (the cluster
 * was unreachable, or discovery itself failed), which is a different thing and
 * must never be drawn as an absence.
 */
export type MetricsServerState = "present" | "absent" | "unknown";

export interface MetricsServerFact {
  state: MetricsServerState;
  /** The group's preferred version (e.g. "v1beta1"). Empty unless present. */
  version: string;
}

/**
 * The overview rail's control-plane facts for one context.
 *
 * `provider` and `region` are **empty when the cluster named none**, and the
 * rail omits the row. A word like "unknown" would look like an answer: a
 * cluster whose nodes carry no region label has not told us it has no region,
 * it has told us nothing, and silence is the honest rendering.
 */
export interface ClusterFacts {
  context: string;
  provider: string;
  region: string;
  metricsServer: MetricsServerFact;
  /** Why the facts could not be read; absent on success. */
  error?: string;
}

/**
 * Read a cluster's provider, region and metrics-server availability via the
 * `k8s.clusterFacts` capability.
 *
 * A deliberate second round trip rather than more work on `connectCluster`:
 * the reachability probe runs for every cluster in the rail on every launch,
 * and one screen's facts must not make it heavier.
 *
 * A failure normalises to empty facts plus a reason, with the metrics server
 * `"unknown"` rather than `"absent"` — a cluster we could not reach has not
 * told us metrics-server is missing.
 */
export async function clusterFacts(
  context: string,
  invoke: Invoker = invokeCapability,
): Promise<ClusterFacts> {
  try {
    return await invoke<ClusterFacts>("k8s.clusterFacts", { context });
  } catch (e) {
    return {
      context,
      provider: "",
      region: "",
      metricsServer: { state: "unknown", version: "" },
      error: String(e),
    };
  }
}
