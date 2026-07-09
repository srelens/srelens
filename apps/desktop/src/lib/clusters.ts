import { invokeCapability, type Invoker } from "../transport/transport";

export interface ClusterContext {
  name: string;
  cluster: string;
  server: string;
  isCurrent: boolean;
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
