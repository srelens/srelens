import { isMap, isScalar, parse, parseDocument, visit } from "yaml";
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
  /** `status.allocatable.cpu`, in millicores — the unit metrics-server uses. */
  allocatableCpuMillicores: number;
  /** `status.allocatable.memory`, in MiB — the unit metrics-server uses. */
  allocatableMemoryMiB: number;
  /** `status.allocatable.pods`. */
  allocatablePods: number;
  /**
   * The node's machine type, read from its `node.kubernetes.io/instance-type`
   * label, falling back to the deprecated `beta.kubernetes.io/instance-type`.
   * Empty when the node carries neither — e.g. on kind, whose nodes are
   * containers rather than cloud machines — not a guessed placeholder.
   */
  instanceType: string;
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

/**
 * What a redacted Secret value is replaced with. Deliberately fixed-width and
 * self-describing: it names itself as a removal rather than implying a value,
 * and it is the same string for every key, so the rendered manifest leaks
 * neither the value nor its length.
 */
const REDACTION_MARKER = "REDACTED";

/** The only two top-level maps of a Secret that carry material. */
const SECRET_VALUE_KEYS = ["data", "stringData"] as const;

/** Opening of every failure message, so they read as one refusal. */
const REDACTION_FAILED = "This Secret's manifest is not shown, because it could not be redacted:";

/**
 * Blank the values of a Secret manifest's top-level `data` and `stringData`
 * maps and of its `metadata.annotations`, keeping every key.
 *
 * `k8s.getObject` redacts Secret values in the backend; `k8s.getManifest`
 * deliberately does NOT (see `crates/kube/src/manifest.rs`) — it is the raw
 * serialisation of the object, and classic's YAML view shows it unredacted.
 * The new design's Secret Details pane gates its values behind an explicit
 * reveal, so the YAML pane beside it runs the manifest through here first;
 * otherwise the gate would be worth nothing to anyone who clicks one tab
 * over. This is a deliberate divergence from classic, not an oversight.
 *
 * Only the two top-level keys, and only at the top level: a `data` nested
 * under `spec`, or one that happens to be an annotation's name, belongs to
 * something else and is left exactly as the cluster returned it.
 *
 * AND every value under `metadata.annotations`, keys kept. THE SCOPE RULE IS
 * "every annotation on a Secret", not "the one annotation kubectl writes",
 * and the difference is the whole point:
 *
 * - `kubectl.kubernetes.io/last-applied-configuration` is the known carrier:
 *   on an `apply`-managed Secret it holds the entire applied manifest, base64
 *   `data` map included, so blanking `data` while leaving it alone puts the
 *   same value back on screen two lines further down. It was the finding.
 * - It is not the only carrier. Any controller can echo a Secret's material
 *   into an annotation of its own — a copy for a sidecar, a "previous value"
 *   left behind by a rotation, a checksum computed over the plaintext — and
 *   the pane cannot tell those from a harmless one by looking at the key. A
 *   rule naming a single well-known key would read as complete while covering
 *   one carrier out of many, which is exactly how this was missed once.
 * - So the annotation's KEY is printed and its VALUE never is. The reader
 *   still sees which controllers touched the Secret, which is what the key is
 *   for; the value can be read on the object's own YAML for a kind where an
 *   annotation is not the material. This mirrors the Details pane, which gates
 *   a Secret's whole annotation map behind a reveal rather than one key of it.
 *
 * WHAT THIS DOES NOT COVER, and must not be read as covering:
 * - Only `Secret` manifests, because only its caller passes one. Nothing here
 *   inspects `kind`; a ConfigMap must never be routed through this, and its
 *   YAML tab still shows its own annotations in full.
 * - Only annotations. `metadata.labels`, `metadata.name`, `ownerReferences`,
 *   `managedFields` and every other field are printed as the cluster returned
 *   them. A label's 63-character value could in principle hold a short secret;
 *   this does not blank it, and widening to all of `metadata` would blank the
 *   Secret's own name and leave nothing worth showing.
 * - Nothing outside `metadata`. A `type`, or a future top-level field carrying
 *   material, is untouched — the two value maps are named explicitly above,
 *   and a new one would have to be added here.
 *
 * Parsing goes through `parseDocument` rather than `parse` so key order and
 * comments survive: the reader is meant to be looking at the manifest the
 * cluster has, not a re-emitted approximation of it.
 *
 * FAILS CLOSED. Anything unexpected — a parse error (which includes a
 * multi-document source), a document that is not a map, a `data`, `metadata`
 * or `annotations` that is not a map, or an alias that could re-expose a value
 * from somewhere else in the document — returns `{ error }` and no `yaml` at
 * all. Passing the input
 * through on a shape this does not understand would be worse than having no
 * redactor, because the caller would believe it had worked.
 */
export function redactSecretManifest(yaml: string): { yaml?: string; error?: string } {
  let doc;
  try {
    doc = parseDocument(yaml);
  } catch {
    // Deliberately says nothing about what was thrown. Every message this
    // function returns is rendered on screen, and both the `yaml` package's
    // parse errors and a stringified throw quote the offending source line —
    // which, for a Secret, IS the value. A redactor whose failure path prints
    // the material it exists to hide is not a redactor.
    return { error: `${REDACTION_FAILED} it could not be parsed.` };
  }
  if (doc.errors.length > 0) {
    // `code` and `linePos` are safe to name: an enum and a number, never
    // source text. `message` is not — see above.
    const first = doc.errors[0];
    const at = first.linePos ? ` at line ${first.linePos[0].line}` : "";
    return { error: `${REDACTION_FAILED} it could not be parsed (${first.code}${at}).` };
  }
  if (!isMap(doc.contents)) {
    return { error: `${REDACTION_FAILED} it is not a single YAML mapping.` };
  }

  // An anchored value under `data` can be referenced by an alias anywhere
  // else in the document; replacing the anchored node would leave that alias
  // pointing at nothing, and leaving it would re-emit the value. serde_yaml
  // (what the backend serialises with) never emits either, so a document
  // that has them is already outside what this understands.
  let hasAlias = false;
  visit(doc, {
    Alias() {
      hasAlias = true;
      return visit.BREAK;
    },
  });
  if (hasAlias) {
    return {
      error: `${REDACTION_FAILED} it uses YAML aliases, which could re-expose a redacted value.`,
    };
  }

  /**
   * Blank every value of the map at `path`, keeping its keys. Absent or
   * explicitly empty is nothing to do; anything that is not a mapping is a
   * shape this does not understand, so it fails closed rather than guessing.
   */
  const blankValuesAt = (path: string[]): string | null => {
    const node = doc.getIn(path, true);
    if (node === undefined || node === null) return null;
    if (isScalar(node) && node.value === null) return null;
    if (!isMap(node)) return `${REDACTION_FAILED} its \`${path.join(".")}\` is not a mapping.`;
    for (const pair of node.items) {
      pair.value = doc.createNode(REDACTION_MARKER);
    }
    return null;
  };

  for (const key of SECRET_VALUE_KEYS) {
    const failure = blankValuesAt([key]);
    if (failure) return { error: failure };
  }

  // `metadata` itself must be a mapping before its annotations can be read: a
  // `metadata` that is a scalar or a sequence is a document this does not
  // understand, and reading no annotations out of it would silently pass an
  // unredacted manifest through.
  const metadata = doc.get("metadata", true);
  if (metadata !== undefined && metadata !== null && !(isScalar(metadata) && metadata.value === null)) {
    if (!isMap(metadata)) {
      return { error: `${REDACTION_FAILED} its \`metadata\` is not a mapping.` };
    }
    const failure = blankValuesAt(["metadata", "annotations"]);
    if (failure) return { error: failure };
  }

  try {
    return { yaml: doc.toString() };
  } catch {
    return { error: `${REDACTION_FAILED} it could not be re-serialised.` };
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
  /**
   * Which namespace the event came from; empty for a cluster-scoped one.
   *
   * A field of its own rather than something read back out of `name`, whose
   * `<namespace>/<name>` shape is there to key the table, not to be parsed.
   */
  namespace: string;
  type: string;
  reason: string;
  object: string;
  message: string;
  age: string;
  /** How many times this event has fired. The backend sends 1 when absent. */
  count: number;
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
