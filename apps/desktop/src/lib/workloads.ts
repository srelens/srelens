import { invokeCapability, type Invoker } from "../transport/transport";

export interface PodSummary {
  name: string;
  namespace: string;
  phase: string;
  ready: string;
  restarts: number;
  node: string;
  age: string;
}

export interface NamespacesOutcome {
  namespaces?: string[];
  error?: string;
}

export interface PodsOutcome {
  pods?: PodSummary[];
  error?: string;
}

export interface LogsOutcome {
  logs?: string;
  error?: string;
}

export interface DeploymentSummary {
  name: string;
  namespace: string;
  ready: string;
  upToDate: number;
  available: number;
  age: string;
}

export interface ServiceSummary {
  name: string;
  namespace: string;
  type: string;
  clusterIP: string;
  ports: string;
  age: string;
}

/** List namespaces in a connected context via `k8s.listNamespaces`. */
export async function listNamespaces(
  context: string,
  invoke: Invoker = invokeCapability,
): Promise<NamespacesOutcome> {
  try {
    const out = await invoke<{ namespaces: string[] }>("k8s.listNamespaces", { context });
    return { namespaces: out.namespaces };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List pods in a namespace of a connected context via `k8s.listPods`. */
export async function listPods(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<PodsOutcome> {
  try {
    const out = await invoke<{ pods: PodSummary[] }>("k8s.listPods", { context, namespace });
    return { pods: out.pods };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List deployments in a namespace via `k8s.listDeployments`. */
export async function listDeployments(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ deployments?: DeploymentSummary[]; error?: string }> {
  try {
    const out = await invoke<{ deployments: DeploymentSummary[] }>("k8s.listDeployments", {
      context,
      namespace,
    });
    return { deployments: out.deployments };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List services in a namespace via `k8s.listServices`. */
export async function listServices(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ services?: ServiceSummary[]; error?: string }> {
  try {
    const out = await invoke<{ services: ServiceSummary[] }>("k8s.listServices", {
      context,
      namespace,
    });
    return { services: out.services };
  } catch (e) {
    return { error: String(e) };
  }
}

export interface ReplicaSetSummary {
  name: string;
  revision: string;
  desired: number;
  ready: number;
  current: number;
  age: string;
}

export interface PodMetric {
  name: string;
  namespace: string;
  cpuMillicores: number;
  memoryMiB: number;
}

/** ReplicaSets owned by a Deployment (its revisions) via `k8s.listReplicaSets`. */
export async function listReplicaSets(
  context: string,
  namespace: string,
  ownerName: string,
  invoke: Invoker = invokeCapability,
): Promise<{ replicasets?: ReplicaSetSummary[]; error?: string }> {
  try {
    const out = await invoke<{ replicasets: ReplicaSetSummary[] }>("k8s.listReplicaSets", {
      context,
      namespace,
      ownerName,
    });
    return { replicasets: out.replicasets };
  } catch (e) {
    return { error: String(e) };
  }
}

/** Pods matching a label selector (a workload's pods) via `k8s.podsForSelector`. */
export async function podsForSelector(
  context: string,
  namespace: string,
  selector: Record<string, string>,
  invoke: Invoker = invokeCapability,
): Promise<PodsOutcome> {
  try {
    const out = await invoke<{ pods: PodSummary[] }>("k8s.podsForSelector", {
      context,
      namespace,
      selector,
    });
    return { pods: out.pods };
  } catch (e) {
    return { error: String(e) };
  }
}

/** Per-pod CPU/memory usage in a namespace via `k8s.podMetrics`. */
export async function podMetrics(
  context: string,
  namespace: string,
  invoke: Invoker = invokeCapability,
): Promise<{ metrics?: PodMetric[]; error?: string }> {
  try {
    const out = await invoke<{ metrics: PodMetric[] }>("k8s.podMetrics", { context, namespace });
    return { metrics: out.metrics };
  } catch (e) {
    return { error: String(e) };
  }
}

/** Delete a pod via `k8s.deletePod` (destructive). */
export async function deletePod(
  context: string,
  namespace: string,
  pod: string,
  invoke: Invoker = invokeCapability,
): Promise<{ deleted?: boolean; error?: string }> {
  try {
    const out = await invoke<{ deleted: boolean }>("k8s.deletePod", { context, namespace, pod });
    return { deleted: out.deleted };
  } catch (e) {
    return { error: String(e) };
  }
}

/** Evict a pod gracefully via `k8s.evictPod` (respects PDBs; destructive). */
export async function evictPod(
  context: string,
  namespace: string,
  pod: string,
  invoke: Invoker = invokeCapability,
): Promise<{ ok?: boolean; error?: string }> {
  try {
    const out = await invoke<{ ok: boolean }>("k8s.evictPod", { context, namespace, pod });
    return { ok: out.ok };
  } catch (e) {
    return { error: String(e) };
  }
}

/** Options for a one-shot log fetch beyond the pod itself. */
export interface PodLogsOptions {
  /** Container name (defaults to the pod's only/first container). */
  container?: string;
  /** Logs from the previous, terminated instance (post-crash triage). */
  previous?: boolean;
  /** Prefix each line with an RFC 3339 timestamp. */
  timestamps?: boolean;
  /** Only lines newer than this many seconds ago. */
  sinceSeconds?: number;
  /** Number of trailing lines to return. */
  tailLines?: number;
}

/** Fetch recent logs for a pod (optionally a specific container) via `k8s.podLogs`. */
export async function podLogs(
  context: string,
  namespace: string,
  pod: string,
  invoke: Invoker = invokeCapability,
  options: PodLogsOptions = {},
): Promise<LogsOutcome> {
  const { container, previous, timestamps, sinceSeconds, tailLines } = options;
  try {
    // `k8s.podLogs` deserialises snake_case field names (no serde rename).
    const out = await invoke<{ logs: string }>("k8s.podLogs", {
      context,
      namespace,
      pod,
      ...(container ? { container } : {}),
      ...(previous ? { previous: true } : {}),
      ...(timestamps ? { timestamps: true } : {}),
      ...(sinceSeconds != null ? { since_seconds: sinceSeconds } : {}),
      ...(tailLines != null ? { tail_lines: tailLines } : {}),
    });
    return { logs: out.logs };
  } catch (e) {
    return { error: String(e) };
  }
}
