import { invokeCapability, type Invoker } from "../transport/transport";

interface ActionResult {
  ok?: boolean;
  error?: string;
}

async function run(
  id: string,
  input: Record<string, unknown>,
  invoke: Invoker,
): Promise<ActionResult> {
  try {
    const out = await invoke<{ ok: boolean }>(id, input);
    return { ok: out.ok };
  } catch (e) {
    return { error: String(e) };
  }
}

/** Delete any supported resource via `k8s.deleteResource`. */
export function deleteResource(
  context: string,
  kind: string,
  namespace: string | null,
  name: string,
  invoke: Invoker = invokeCapability,
): Promise<ActionResult> {
  return run("k8s.deleteResource", { context, kind, namespace: namespace ?? "", name }, invoke);
}

/** Set replica count via `k8s.scale`. */
export function scaleResource(
  context: string,
  kind: string,
  namespace: string,
  name: string,
  replicas: number,
  invoke: Invoker = invokeCapability,
): Promise<ActionResult> {
  return run("k8s.scale", { context, kind, namespace, name, replicas }, invoke);
}

/** Trigger a rolling restart via `k8s.rolloutRestart`. */
export function rolloutRestart(
  context: string,
  kind: string,
  namespace: string,
  name: string,
  invoke: Invoker = invokeCapability,
): Promise<ActionResult> {
  return run("k8s.rolloutRestart", { context, kind, namespace, name }, invoke);
}

/**
 * Update ConfigMap/Secret values in place via `k8s.updateConfigData`. `data`
 * holds plaintext values for the keys being changed; other keys are untouched.
 * For Secrets the backend writes via `stringData`, so the caller passes
 * plaintext and the apiserver base64-encodes it.
 */
export function updateConfigData(
  context: string,
  kind: string,
  namespace: string,
  name: string,
  data: Record<string, string>,
  invoke: Invoker = invokeCapability,
): Promise<ActionResult> {
  return run("k8s.updateConfigData", { context, kind, namespace, name, data }, invoke);
}

/** Cordon/uncordon a node via `k8s.cordonNode`. */
export function cordonNode(
  context: string,
  name: string,
  unschedulable: boolean,
  invoke: Invoker = invokeCapability,
): Promise<ActionResult> {
  return run("k8s.cordonNode", { context, name, unschedulable }, invoke);
}

/** Suspend or resume a CronJob via `k8s.cronjobSetSuspend`. */
export function cronjobSetSuspend(
  context: string,
  namespace: string,
  name: string,
  suspend: boolean,
  invoke: Invoker = invokeCapability,
): Promise<ActionResult> {
  return run("k8s.cronjobSetSuspend", { context, namespace, name, suspend }, invoke);
}

/**
 * Run a CronJob immediately via `k8s.cronjobTriggerNow`. The unique Job-name
 * suffix is generated here (a timestamp) so the backend handler stays
 * deterministic. Returns the created Job's name.
 */
export async function cronjobTriggerNow(
  context: string,
  namespace: string,
  name: string,
  invoke: Invoker = invokeCapability,
): Promise<{ jobName?: string; error?: string }> {
  try {
    const suffix = String(Date.now());
    const out = await invoke<{ jobName: string }>("k8s.cronjobTriggerNow", {
      context,
      namespace,
      name,
      suffix,
    });
    return { jobName: out.jobName };
  } catch (e) {
    return { error: String(e) };
  }
}

/** Cordon + evict a node's pods via `k8s.drainNode`. */
export async function drainNode(
  context: string,
  name: string,
  invoke: Invoker = invokeCapability,
): Promise<{ evicted?: number; skipped?: number; error?: string }> {
  try {
    const out = await invoke<{ evicted: number; skipped: number }>("k8s.drainNode", { context, name });
    return { evicted: out.evicted, skipped: out.skipped };
  } catch (e) {
    return { error: String(e) };
  }
}

/**
 * Attach an ephemeral debug container to a running pod via `k8s.debugPod`
 * (destructive). Returns the created container's name to exec into. `target`
 * shares another container's process namespace (for distroless pods).
 */
export async function debugPod(
  context: string,
  namespace: string,
  pod: string,
  image: string,
  target: string | null = null,
  invoke: Invoker = invokeCapability,
): Promise<{ container?: string; error?: string }> {
  try {
    const out = await invoke<{ container: string }>("k8s.debugPod", {
      context,
      namespace,
      pod,
      image,
      target_container: target ?? null,
    });
    return { container: out.container };
  } catch (e) {
    return { error: String(e) };
  }
}

/**
 * Create a privileged node debug pod via `k8s.createNodeDebugPod` (destructive).
 * Returns the pod's namespace + generated name; delete it (deletePod) when the
 * shell closes.
 */
export async function createNodeDebugPod(
  context: string,
  node: string,
  image: string | null = null,
  namespace: string | null = null,
  invoke: Invoker = invokeCapability,
): Promise<{ namespace?: string; pod?: string; error?: string }> {
  try {
    const out = await invoke<{ namespace: string; pod: string }>("k8s.createNodeDebugPod", {
      context,
      node,
      image: image ?? null,
      namespace: namespace ?? null,
    });
    return { namespace: out.namespace, pod: out.pod };
  } catch (e) {
    return { error: String(e) };
  }
}
