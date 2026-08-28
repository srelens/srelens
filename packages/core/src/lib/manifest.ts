import { parse } from "yaml";
import { invokeCapability, type Invoker } from "../transport/transport";

export interface NodeSummary {
  name: string;
  /** Readiness: "Ready", "NotReady", or "Unknown". */
  status: string;
  /** Cordoned (`spec.unschedulable`) — surfaced as "SchedulingDisabled". */
  unschedulable: boolean;
  /** Number of taints, excluding the auto-added unschedulable taint. */
  taints: number;
  version: string;
  roles: string;
  age: string;
}

/** Dynamic GVK + plural for a custom resource (CRD-backed kinds). */
export interface DynamicGvk {
  group: string;
  version: string;
  plural: string;
}

/** Fetch a resource's manifest as YAML via `k8s.getManifest`. */
export async function getManifest(
  context: string,
  kind: string,
  namespace: string | null,
  name: string,
  invoke: Invoker = invokeCapability,
  crd?: DynamicGvk,
): Promise<{ yaml?: string; error?: string }> {
  try {
    const out = await invoke<{ yaml: string }>("k8s.getManifest", {
      context,
      kind,
      namespace,
      name,
      ...(crd && { group: crd.group, version: crd.version, plural: crd.plural }),
    });
    return { yaml: out.yaml };
  } catch (e) {
    return { error: String(e) };
  }
}

/** A Kubernetes object as returned by `k8s.getObject` (loosely typed JSON). */
export type K8sObject = {
  apiVersion?: string;
  kind?: string;
  metadata?: {
    name?: string;
    namespace?: string;
    uid?: string;
    creationTimestamp?: string;
    labels?: Record<string, string>;
    annotations?: Record<string, string>;
    ownerReferences?: { kind: string; name: string }[];
    [k: string]: unknown;
  };
  spec?: Record<string, unknown>;
  status?: Record<string, unknown>;
  [k: string]: unknown;
};

/** Fetch a resource as a structured JSON object via `k8s.getObject`. */
export async function getObject(
  context: string,
  kind: string,
  namespace: string | null,
  name: string,
  invoke: Invoker = invokeCapability,
): Promise<{ object?: K8sObject; error?: string }> {
  try {
    const out = await invoke<{ object: K8sObject }>("k8s.getObject", {
      context,
      kind,
      namespace,
      name,
    });
    return { object: out.object };
  } catch (e) {
    return { error: String(e) };
  }
}

/**
 * Read a Secret's values via the dedicated, consent-gateable `k8s.getSecret`.
 * `k8s.getObject` redacts Secret data, so this is the only structured path to
 * the (base64-encoded) values — fetched lazily, only when the user reveals a
 * key.
 */
export async function getSecret(
  context: string,
  namespace: string,
  name: string,
  invoke: Invoker = invokeCapability,
): Promise<{ data?: Record<string, string>; error?: string }> {
  try {
    const out = await invoke<{ data: Record<string, string> }>("k8s.getSecret", {
      context,
      namespace,
      name,
    });
    return { data: out.data };
  } catch (e) {
    return { error: String(e) };
  }
}

export interface Conflict {
  managers: string[];
  fields: string[];
  message: string;
}

export interface ApplyDoc {
  kind: string;
  name: string;
  applied: boolean;
  conflict?: Conflict | null;
  error?: string | null;
}

export interface DiffRow {
  tag: "same" | "insert" | "delete" | "replace";
  left: string | null;
  right: string | null;
}

export interface DiffDoc {
  kind: string;
  name: string;
  namespace: string | null;
  exists: boolean;
  changed: boolean;
  rows: DiffRow[];
  currentResourceVersion: string | null;
}

/** Server-side apply one or more YAML documents via `k8s.applyManifest`. */
export async function applyManifest(
  context: string,
  yaml: string,
  force = false,
  invoke: Invoker = invokeCapability,
): Promise<{ documents?: ApplyDoc[]; applied?: boolean; error?: string }> {
  try {
    const out = await invoke<{ documents: ApplyDoc[]; applied: boolean }>("k8s.applyManifest", {
      context,
      yaml,
      force,
    });
    return { documents: out.documents, applied: out.applied };
  } catch (e) {
    return { error: String(e) };
  }
}

/** Diff a manifest against the cluster (dry-run) via `k8s.diffManifest`. */
export async function diffManifest(
  context: string,
  yaml: string,
  invoke: Invoker = invokeCapability,
): Promise<{ documents?: DiffDoc[]; error?: string }> {
  try {
    const out = await invoke<{ documents: DiffDoc[] }>("k8s.diffManifest", { context, yaml });
    return { documents: out.documents };
  } catch (e) {
    return { error: String(e) };
  }
}

/** Read `metadata.resourceVersion` from a manifest, for stale-edit detection. */
export function parseResourceVersion(yaml: string): string | null {
  try {
    const doc = parse(yaml) as { metadata?: { resourceVersion?: unknown } } | null;
    const rv = doc?.metadata?.resourceVersion;
    return rv == null ? null : String(rv);
  } catch {
    return null;
  }
}

/** A single validation error, tagged with the (empty-doc-skipped) document it
 *  came from so the editor can place it on the right `---`-separated document. */
export interface ValidateError {
  docIndex: number;
  message: string;
}

/**
 * Validate a manifest against the API server (server-side dry-run, strict).
 * Returns the server's verdict + error messages. `error` is only set when the
 * call itself fails (not for validation failures, which come back as `errors`).
 */
export async function validateManifest(
  context: string,
  yaml: string,
  invoke: Invoker = invokeCapability,
): Promise<{ valid?: boolean; errors?: ValidateError[]; error?: string }> {
  try {
    const out = await invoke<{ valid: boolean; errors: ValidateError[] }>("k8s.validateManifest", {
      context,
      yaml,
    });
    return { valid: out.valid, errors: out.errors };
  } catch (e) {
    return { error: String(e) };
  }
}

export interface ResourceRow {
  name: string;
  namespace: string;
  age: string;
}

export interface EventSummary {
  name: string;
  type: string;
  reason: string;
  object: string;
  message: string;
  age: string;
}

export interface EventObjectFilter {
  kind: string;
  name: string;
}

/** List events via `k8s.listEvents`. */
export async function listEvents(
  context: string,
  namespace: string | null,
  object?: EventObjectFilter,
  invoke: Invoker = invokeCapability,
): Promise<{ events?: EventSummary[]; error?: string }> {
  try {
    const out = await invoke<{ events: EventSummary[] }>("k8s.listEvents", {
      context,
      namespace: namespace ?? "",
      objectKind: object?.kind ?? "",
      objectName: object?.name ?? "",
    });
    return { events: out.events };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List any supported kind generically via `k8s.listResource`. */
export async function listResource(
  context: string,
  kind: string,
  namespace: string | null,
  invoke: Invoker = invokeCapability,
): Promise<{ items?: ResourceRow[]; error?: string }> {
  try {
    const out = await invoke<{ items: ResourceRow[] }>("k8s.listResource", {
      context,
      kind,
      namespace,
    });
    return { items: out.items };
  } catch (e) {
    return { error: String(e) };
  }
}

export interface NodeMetric {
  name: string;
  cpuMillicores: number;
  memoryMiB: number;
}

/** Per-node CPU/memory usage via `k8s.nodeMetrics` (needs metrics-server). */
export async function nodeMetrics(
  context: string,
  invoke: Invoker = invokeCapability,
): Promise<{ metrics?: NodeMetric[]; error?: string }> {
  try {
    const out = await invoke<{ metrics: NodeMetric[] }>("k8s.nodeMetrics", { context });
    return { metrics: out.metrics };
  } catch (e) {
    return { error: String(e) };
  }
}

/** List cluster nodes via `k8s.listNodes`. */
export async function listNodes(
  context: string,
  invoke: Invoker = invokeCapability,
): Promise<{ nodes?: NodeSummary[]; error?: string }> {
  try {
    const out = await invoke<{ nodes: NodeSummary[] }>("k8s.listNodes", { context });
    return { nodes: out.nodes };
  } catch (e) {
    return { error: String(e) };
  }
}
