import { K8S_KIND, WATCHABLE_KINDS, listNodes, listResource, nodeMetrics, podMetrics, type ResourceKind } from "@srelens/core";
import type { Column } from "@srelens/ui-kit";
import {
  clusterRoleBindingColumns,
  clusterRoleColumns,
  configMapColumns,
  cronJobColumns,
  daemonSetColumns,
  daemonSetFlagged,
  deploymentColumns,
  deploymentFlagged,
  endpointSliceColumns,
  ingressColumns,
  jobColumns,
  jobFlagged,
  limitRangeColumns,
  networkPolicyColumns,
  nodeColumns,
  nodeFlagged,
  podColumns,
  podFlagged,
  pvColumns,
  pvcColumns,
  resourceQuotaColumns,
  roleBindingColumns,
  roleColumns,
  secretColumns,
  serviceAccountColumns,
  serviceColumns,
  statefulSetColumns,
  statefulSetFlagged,
  storageClassColumns,
  type NodeRow,
  type PodRow,
} from "./columns";
import { genericClusterColumns, genericColumns } from "./generic";
import type { KindDescriptor, ListRow } from "./types";

/** Classic's list, unchanged: the kinds that have no namespace. */
export const CLUSTER_SCOPED: readonly ResourceKind[] = [
  "nodes", "namespaces", "persistentvolumes", "storageclasses", "priorityclasses",
  "runtimeclasses", "mutatingwebhookconfigurations", "validatingwebhookconfigurations",
  "ingressclasses", "clusterroles", "clusterrolebindings",
];

const isWatchable = (kind: string) => (WATCHABLE_KINDS as readonly string[]).includes(kind);
const isClusterScoped = (kind: string) => (CLUSTER_SCOPED as readonly string[]).includes(kind);

/**
 * The typed entries. A kind absent from here is served by the generic
 * descriptor below — deliberately, and only for the kinds the backend has
 * nothing more to say about. `descriptors.test.ts` asserts the whole sidebar
 * resolves, so a kind that *should* be typed cannot slip through as generic.
 *
 * Keyed on `ListRow` rather than the brief's shorthand `never`: `never` makes
 * every entry's row type bottom, which types the empty table below but leaves
 * no room for Tasks 4 and 5 to add a `KindDescriptor<PodRow>` here without a
 * cast at the call site. `ListRow` is the actual lower bound `descriptorFor`
 * promises callers — the widening is confined to this table and to
 * `descriptorFor`'s per-kind casts below; nothing exported gets looser.
 */
/**
 * Merges `listNodes` with `nodeMetrics` by name. Best-effort on the metrics
 * half: a cluster with no metrics-server (so `nodeMetrics` errors) must still
 * list its nodes, just without CPU/memory readings — only `list.error`
 * propagates.
 */
const loadNodes = async (context: string) => {
  const [list, metrics] = await Promise.all([listNodes(context), nodeMetrics(context)]);
  const byName = new Map((metrics.metrics ?? []).map((m) => [m.name, m]));
  const rows: NodeRow[] = (list.nodes ?? []).map((n) => ({
    ...n,
    cpu: byName.get(n.name)?.cpuMillicores,
    memory: byName.get(n.name)?.memoryMiB,
  }));
  return { rows, error: list.error };
};

/**
 * CPU/memory for the pods list, on its own `enrichMs` cadence (Task 8) —
 * best-effort: a cluster with no metrics-server makes `podMetrics` fail, and
 * `useResourceList` swallows that so the pods still list, just without a
 * reading. Only rows the metrics mention are touched.
 */
const podEnrich = async (context: string, namespace: string): Promise<Map<string, Partial<PodRow>>> => {
  const out = await podMetrics(context, namespace);
  return new Map((out.metrics ?? []).map((m) => [m.name, { cpu: m.cpuMillicores, memory: m.memoryMiB }]));
};

/**
 * The seven typed entries this task adds (workloads and nodes). Each column
 * set is typed over its own row (`Column<PodRow>[]`, etc.), a proper subtype
 * of `ListRow` on the data side — but `Column`'s `render`/`getSortValue`
 * take the row contravariantly, so TypeScript can't see the assignment into
 * `Column<ListRow>[]` is safe on its own. Same cast `descriptors.ts` already
 * uses for the generic columns (Task 3), confined to this table: every
 * function these typed columns hold only reads fields `ListRow` doesn't
 * guarantee (`phase`, `cpu`, …), never a field a bare `ListRow` caller could
 * supply instead — so a `ListRow`-typed caller cannot actually reach one.
 */
const TYPED: Partial<Record<ResourceKind, KindDescriptor<ListRow>>> = {
  pods: {
    k8sKind: "Pod",
    columns: podColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    // Same variance cast the columns above already need (see the comment
    // above this table): `Partial<PodRow>` only widens `Partial<ListRow>`.
    enrich: podEnrich as (context: string, namespace: string) => Promise<Map<string, Partial<ListRow>>>,
    enrichMs: 10000,
    actions: { logs: true, shell: true, forward: true, evict: true },
    // Same variance cast every function on this table already needs: `flagged`
    // only reads `phase`, a field `ListRow` does not promise.
    flagged: podFlagged as (row: ListRow) => boolean,
    // Task 10 ported `PodContainersBody` off classic's `ContainerCard` — the
    // detail shell only offers the Containers tab where this is set. The
    // pane's ports are also the way into a port forward, which is why `Pod`
    // sets both this and `actions.forward`.
    panes: { containers: true },
  },
  deployments: {
    k8sKind: "Deployment",
    columns: deploymentColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: { logs: true, scale: true, restart: true },
    flagged: deploymentFlagged as (row: ListRow) => boolean,
  },
  statefulsets: {
    k8sKind: "StatefulSet",
    columns: statefulSetColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: { logs: true, scale: true, restart: true },
    flagged: statefulSetFlagged as (row: ListRow) => boolean,
  },
  daemonsets: {
    k8sKind: "DaemonSet",
    columns: daemonSetColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: { logs: true, restart: true },
    flagged: daemonSetFlagged as (row: ListRow) => boolean,
  },
  jobs: {
    k8sKind: "Job",
    columns: jobColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: { logs: true },
    // Same variance cast every function on this table already needs: `flagged`
    // only reads `failed`, a field `ListRow` does not promise.
    flagged: jobFlagged as (row: ListRow) => boolean,
  },
  cronjobs: {
    k8sKind: "CronJob",
    columns: cronJobColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    // The two actions classic offers that no other kind has (Task 4's review
    // ruling): suspend/resume and run-now. No other descriptor sets these.
    actions: { suspend: true, trigger: true },
  },
  nodes: {
    k8sKind: "Node",
    columns: nodeColumns as Column<ListRow>[],
    source: "poll",
    scope: "cluster",
    load: loadNodes,
    actions: {},
    // Same variance cast every function on this table already needs:
    // `flagged` only reads `status`/`unschedulable`, fields `ListRow` does not
    // promise. Present so a NotReady or cordoned node's row asks the same
    // question its detail pane asks — both read core's `nodeStatus`.
    flagged: nodeFlagged as (row: ListRow) => boolean,
  },
  configmaps: {
    k8sKind: "ConfigMap",
    columns: configMapColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: {},
  },
  secrets: {
    k8sKind: "Secret",
    columns: secretColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: {},
  },
  resourcequotas: {
    k8sKind: "ResourceQuota",
    columns: resourceQuotaColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: {},
  },
  limitranges: {
    k8sKind: "LimitRange",
    columns: limitRangeColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: {},
  },
  services: {
    k8sKind: "Service",
    columns: serviceColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: {},
  },
  ingresses: {
    k8sKind: "Ingress",
    columns: ingressColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: {},
  },
  endpointslices: {
    k8sKind: "EndpointSlice",
    columns: endpointSliceColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: {},
  },
  networkpolicies: {
    k8sKind: "NetworkPolicy",
    columns: networkPolicyColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: {},
  },
  persistentvolumeclaims: {
    k8sKind: "PersistentVolumeClaim",
    columns: pvcColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: {},
  },
  persistentvolumes: {
    k8sKind: "PersistentVolume",
    columns: pvColumns as Column<ListRow>[],
    source: "watch",
    scope: "cluster",
    actions: {},
  },
  storageclasses: {
    k8sKind: "StorageClass",
    columns: storageClassColumns as Column<ListRow>[],
    source: "watch",
    scope: "cluster",
    actions: {},
  },
  serviceaccounts: {
    k8sKind: "ServiceAccount",
    columns: serviceAccountColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: {},
  },
  roles: {
    k8sKind: "Role",
    columns: roleColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: {},
  },
  clusterroles: {
    k8sKind: "ClusterRole",
    columns: clusterRoleColumns as Column<ListRow>[],
    source: "watch",
    scope: "cluster",
    actions: {},
  },
  rolebindings: {
    k8sKind: "RoleBinding",
    columns: roleBindingColumns as Column<ListRow>[],
    source: "watch",
    scope: "namespaced",
    actions: {},
  },
  clusterrolebindings: {
    k8sKind: "ClusterRoleBinding",
    columns: clusterRoleBindingColumns as Column<ListRow>[],
    source: "watch",
    scope: "cluster",
    actions: {},
  },
};

/**
 * `overview`, `portforwards`, `helmreleases`, `settings` and friends are in
 * core's `ResourceKind` union for the classic sidebar's sake and have no
 * Kubernetes kind behind them; `K8S_KIND` is `""` for each. They are screens,
 * not lists.
 */
function isListable(slug: string): slug is ResourceKind {
  return Object.prototype.hasOwnProperty.call(K8S_KIND, slug) && K8S_KIND[slug as ResourceKind] !== "";
}

export function descriptorFor(slug: string): KindDescriptor<ListRow> | undefined {
  if (!isListable(slug)) return undefined;
  const typed = Object.prototype.hasOwnProperty.call(TYPED, slug) ? TYPED[slug] : undefined;
  if (typed) return typed;
  const cluster = isClusterScoped(slug);
  return {
    k8sKind: K8S_KIND[slug],
    // `genericColumns` is typed over core's `ResourceRow` (name, namespace,
    // age), which is a proper subtype of `ListRow` — so the reverse cast here
    // is the variance-safe direction: every function on these columns already
    // tolerates a row with fewer fields than `ResourceRow` promises (the only
    // one, `ageSortValue`, reads an optional `age`), TypeScript just can't see
    // that through `Column`'s contravariant row parameter.
    columns: (cluster ? genericClusterColumns : genericColumns) as Column<ListRow>[],
    source: isWatchable(slug) ? "watch" : "poll",
    scope: cluster ? "cluster" : "namespaced",
    load: (context, namespace) =>
      listResource(context, K8S_KIND[slug], namespace).then((o) => ({ rows: o.items, error: o.error })),
    actions: {},
  };
}
