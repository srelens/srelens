import { useCallback, useMemo } from "react";
import {
  K8S_KIND,
  clusterCapacity,
  clusterFacts,
  listDaemonSets,
  listDeployments,
  listNamespaces,
  listNodes,
  listResource,
  listStatefulSets,
  nodeMetrics,
  nodeUsage,
  podOverview,
  type ClusterCapacity,
  type ClusterFacts,
  type DaemonSetSummary,
  type DeploymentSummary,
  type NodeSummary,
  type NodeUsage,
  type PodSummary,
  type ResourceKind,
  type StatefulSetSummary,
} from "@srelens/core";
import { useCachedResource } from "./cachedResource";
import type { ResourceStatus } from "./useResource";

/**
 * The cluster overview's data layer: one loader per independent fact.
 *
 * Classic's `ClusterOverview` fires six list calls through a single
 * `Promise.all` and rethrows the first error it finds, so one refused list —
 * `nodes` on a namespaced service account, say — blanks the whole dashboard,
 * including the five answers that came back fine. That is the property this
 * module exists not to have.
 *
 * A screen made of independent facts fails in independent pieces: a refused
 * `listNodes` empties the nodes table and says so, while the namespace count,
 * the object counts, the control-plane facts and Fleet stay on screen. Every
 * hook below therefore owns its own loader, and nothing here ever awaits two
 * capabilities in a way that lets one fail the other.
 *
 * The second rule, running through all of it: **`null` is not zero.** A
 * percentage of `null` means "no reading" — metrics-server absent, or a node
 * that has not been scraped yet — and `0%` is a measurement that reads as an
 * idle cluster. The same holds one level up: a namespace count of `null` is a
 * refusal, not an empty cluster. Nothing between core's arithmetic and the
 * screen may coalesce one into the other.
 *
 * Percentages leave here exactly as `nodeUsage` computed them: unrounded and
 * uncapped. `Meter` clamps the bar it draws while keeping `aria-valuetext`
 * truthful; clamping in between would make a node at 140% indistinguishable
 * from one exactly at its limit, hiding the case a reader most needs to see.
 *
 * ## Every loader is cached
 *
 * `useCachedResource`, not `useResource`: coming back to this tab paints the
 * last good answer immediately and refreshes behind it, instead of spending a
 * whole cluster's round trips to arrive at the same numbers. The cache keys
 * below all carry the CONTEXT, because that is the one thing a cached cluster
 * answer must never be shown for the wrong one of.
 *
 * The rule the cache does not get to break is the module's first one: a
 * refresh that fails over rows already on screen leaves those rows there and
 * reports `stale`, and every loader carries that flag up to the screen, which
 * says so. Quietly serving figures that stopped refreshing is the same lie as
 * a `0` in place of "no reading".
 */

/** An outcome-shaped core call, turned into the rejection the loader reads. */
function unwrap<T>(value: T | undefined, error: string | undefined, what: string): T {
  if (error) throw new Error(error);
  if (value === undefined) throw new Error(`${what} returned no data`);
  return value;
}

/** The cache identity of one loader's answer for one cluster. */
function key(what: string, context: string): string {
  return `overview:${what}|${context}`;
}

/** One node, paired with its usage against its own allocatable capacity. */
export interface OverviewNode {
  node: NodeSummary;
  usage: NodeUsage;
}

export interface OverviewNodes {
  status: ResourceStatus;
  nodes: OverviewNode[];
  /** The cluster-wide sums for the capacity strip; carries its own `null`s. */
  capacity: ClusterCapacity;
  /** Why the node list is unavailable. The table is empty and says this. */
  error?: string;
  /**
   * Why there are no readings — held apart from `error` on purpose. A cluster
   * with no metrics-server still has a nodes table; it just has no meters, and
   * the rail states the absence once rather than every column announcing it.
   */
  metricsError?: string;
  /** These rows are the last good ones and are no longer refreshing. */
  stale: boolean;
  reload(): void;
}

/**
 * The nodes table's rows and the capacity strip's totals.
 *
 * Two capabilities, two loaders: `listNodes` and `nodeMetrics` fail
 * independently, and neither empties the other. Metrics are the ones that go
 * missing in practice (metrics-server is not installed on every cluster), and
 * losing the node list to that would be the exact all-or-nothing failure this
 * module is written against.
 *
 * @param byNode - How many pods each node is running, or `undefined` while
 *   that is unknown or was refused. Passed in rather than derived here so the
 *   screen makes one pod call for the Pods tile, the unhealthy list and these
 *   per-node counts, instead of three.
 */
export function useOverviewNodes(
  context: string,
  byNode: Map<string, number> | undefined,
): OverviewNodes {
  const nodes = useCachedResource(key("nodes", context), () =>
    listNodes(context).then((o) => unwrap(o.nodes, o.error, "listNodes")),
  );
  const metrics = useCachedResource(key("nodeMetrics", context), () =>
    nodeMetrics(context).then((o) => unwrap(o.metrics, o.error, "nodeMetrics")),
  );

  const list = nodes.data;
  const readings = metrics.data;

  const rows = useMemo<OverviewNode[]>(() => {
    if (!list) return [];
    const byName = new Map((readings ?? []).map((m) => [m.name, m]));
    return list.map((node) => ({
      node,
      // The one place a `0` is legitimate: when `byNode` exists the grouping
      // is KNOWN and complete, so a node missing from it genuinely has no pods
      // on it. When the map is `undefined` the count is unknown, and
      // `undefined` is what `nodeUsage` turns into a `pods` of `null` — never
      // `{ used: 0 }`, which would claim an empty node nobody counted.
      usage: nodeUsage(node, byName.get(node.name), byNode ? (byNode.get(node.name) ?? 0) : undefined),
    }));
  }, [list, readings, byNode]);

  // Sums only over nodes that reported; `clusterCapacity` carries
  // `nodesReporting`/`nodesTotal` so the screen cannot show a partial total as
  // if it were a whole one.
  const capacity = useMemo(() => clusterCapacity(list ?? [], readings ?? []), [list, readings]);

  const reloadNodes = nodes.reload;
  const reloadMetrics = metrics.reload;
  const reload = useCallback(() => {
    reloadNodes();
    reloadMetrics();
  }, [reloadNodes, reloadMetrics]);

  return {
    status: nodes.status,
    nodes: rows,
    capacity,
    error: nodes.error,
    metricsError: metrics.error,
    stale: nodes.stale || metrics.stale,
    reload,
  };
}

export interface OverviewPods {
  status: ResourceStatus;
  /**
   * Every pod in the cluster, or `null` when that is not known — loading, or
   * refused. Deliberately not defaulted to `0`: a zero is the answer "this
   * cluster has no pods", which is what a 113-node cluster's Pods tile used to
   * be one coalesce away from claiming.
   */
  total: number | null;
  /**
   * How many pods each node is running, or `undefined` when unknown.
   *
   * When it is present it is COMPLETE: a node absent from the map runs no
   * pods, and that is a reading rather than a gap. That completeness is what
   * lets a node's Pods column show a truthful `0`.
   */
  byNode?: Map<string, number>;
  /**
   * Every pod that is not simply running — a superset of the unhealthy ones,
   * for core to judge. `undefined` when the cluster has not answered.
   */
  unsettled?: PodSummary[];
  /** Whether {@link unsettled} is shorter than the truth — see `podOverview`. */
  truncated: boolean;
  error?: string;
  stale: boolean;
  reload(): void;
}

/**
 * The cluster's pod facts, from one call that never lists its pods.
 *
 * Three sections need them — the Pods tile, the per-node `31/50` column and
 * the `NOT READY` list — and `k8s.podOverview` is the one call that serves all
 * three: a server-printed table for the counts and the per-node grouping, and
 * pod bodies only for the handful that are not simply running. `listPods` with
 * an empty namespace used to serve them, and on a 113-node cluster that is
 * 5 416 pods and 114 MB, which does not come back inside the request budget —
 * so all three read "No reading" on the one cluster where they matter most.
 * See `crates/kube/src/pod_overview.rs` for what replaced it and why.
 *
 * Fleet counts through `podCount` per cluster for the same reason: neither
 * screen can afford a pod list, and neither takes one.
 */
export function useOverviewPods(context: string): OverviewPods {
  const pods = useCachedResource(
    key("pods", context),
    () => podOverview(context).then((o) => unwrap(o.pods, o.error, "podOverview")),
    // A cluster with no pods at all has answered; it is not an empty section.
    () => false,
  );

  const data = pods.data;
  const byNode = useMemo(() => {
    if (!data) return undefined;
    return new Map(data.byNode.map((n) => [n.node, n.pods]));
  }, [data]);

  return {
    status: pods.status,
    total: data ? data.total : null,
    byNode,
    unsettled: data?.unsettled,
    truncated: data?.truncated ?? false,
    error: pods.error,
    stale: pods.stale,
    reload: pods.reload,
  };
}

export interface NamespaceCount {
  status: ResourceStatus;
  /** `null` when unknown — loading, or refused. Never `0` for either. */
  count: number | null;
  error?: string;
  stale: boolean;
  reload(): void;
}

/** The Namespaces tile's number, and nothing else — the tile has no caption. */
export function useNamespaceCount(context: string): NamespaceCount {
  const namespaces = useCachedResource(key("namespaces", context), () =>
    listNamespaces(context).then((o) => unwrap(o.namespaces, o.error, "listNamespaces")),
  );
  return {
    status: namespaces.status,
    count: namespaces.data === undefined ? null : namespaces.data.length,
    error: namespaces.error,
    stale: namespaces.stale,
    reload: namespaces.reload,
  };
}

/**
 * The kinds the rail's `OBJECTS BY KIND` section counts, in the design's
 * order. Slugs, not Kubernetes kinds, so each row can open `/k/<slug>` and
 * take its label from `RESOURCE_LABELS` without a second table.
 */
export const OVERVIEW_KINDS: ResourceKind[] = [
  "deployments",
  "pods",
  "statefulsets",
  "daemonsets",
  "cronjobs",
  "jobs",
];

/**
 * The kinds this screen has to LIST to count — the two nothing else on it
 * loads.
 *
 * The other four of `OVERVIEW_KINDS` are already on screen: Deployments,
 * StatefulSets and DaemonSets are the rows the `Not ready` list reads, and
 * Pods is the count `podOverview` already made. Counting them through
 * `listResource` as well would be four extra whole-cluster list calls to
 * produce four numbers the screen has already fetched — and on a cluster with
 * 5 416 pods, the pod one is not a small request at all. So the counts are
 * taken off the loaders that own those kinds, and only CronJobs and Jobs are
 * listed here.
 *
 * The rail's ORDER is still `OVERVIEW_KINDS`; this is only about where each
 * number comes from.
 */
const COUNTED_BY_LISTING: ResourceKind[] = ["cronjobs", "jobs"];

export interface KindCount {
  slug: ResourceKind;
  /** `null` when this kind could not be counted. Never `0` for a refusal. */
  count: number | null;
  /** Why this one kind has no count. The other rows are unaffected. */
  error?: string;
}

export interface ObjectCounts {
  status: ResourceStatus;
  counts: KindCount[];
  error?: string;
  stale: boolean;
  reload(): void;
}

/**
 * One row per kind, each carrying its own count or its own failure.
 *
 * Two sources, and the seam between them is invisible to the rail: the four
 * kinds the screen already loaded are counted off those loaders (see
 * {@link COUNTED_BY_LISTING}), and the two it does not are listed here. The
 * `Promise.all` over the second group is safe in the way classic's was not:
 * `listResource` returns its error rather than throwing, so no branch of the
 * fan-out can reject and cancel the others.
 *
 * A kind that could not be counted becomes `count: null` WITH the reason,
 * never `0` — a refused list and an empty cluster are the same picture and
 * opposite facts, and zero is the one a reader believes.
 */
export function useObjectCounts(
  context: string,
  workloads: OverviewWorkloads,
  pods: OverviewPods,
): ObjectCounts {
  const listed = useCachedResource(key("objectCounts", context), () =>
    Promise.all(
      COUNTED_BY_LISTING.map((slug) =>
        listResource(context, K8S_KIND[slug], "").then<KindCount>((o) =>
          o.error ? { slug, count: null, error: o.error } : { slug, count: (o.items ?? []).length },
        ),
      ),
    ),
  );

  const data = listed.data;
  const listedError = listed.error;
  const podTotal = pods.total;
  const podError = pods.error;
  const counts = useMemo<KindCount[]>(() => {
    /** A count off a list the screen already has, or the reason it has none. */
    const shared = (slug: ResourceKind, rows: unknown[] | undefined, error?: string): KindCount => ({
      slug,
      count: rows ? rows.length : null,
      error: rows ? undefined : error,
    });

    const already = new Map<ResourceKind, KindCount>([
      ["deployments", shared("deployments", workloads.deployments, workloads.refusals.deployments ?? workloads.error)],
      ["statefulsets", shared("statefulsets", workloads.statefulSets, workloads.refusals.statefulsets ?? workloads.error)],
      ["daemonsets", shared("daemonsets", workloads.daemonSets, workloads.refusals.daemonsets ?? workloads.error)],
      // Counted, never listed — the whole point of `podOverview`. `null` is
      // the refusal; `0` would be the claim that the cluster runs no pods.
      ["pods", { slug: "pods", count: podTotal, error: podTotal === null ? podError : undefined }],
    ]);
    const byList = new Map((data ?? []).map((c) => [c.slug, c]));

    return OVERVIEW_KINDS.map(
      (slug) => already.get(slug) ?? byList.get(slug) ?? { slug, count: null, error: listedError },
    );
  }, [
    workloads.deployments,
    workloads.statefulSets,
    workloads.daemonSets,
    workloads.refusals,
    workloads.error,
    podTotal,
    podError,
    data,
    listedError,
  ]);

  return {
    status: listed.status,
    counts,
    error: listedError,
    stale: listed.stale,
    reload: listed.reload,
  };
}

export interface OverviewWorkloads {
  status: ResourceStatus;
  /**
   * The kind's rows, or `undefined` when it could not be listed. Deliberately
   * not `[]`: an empty array is the answer "this cluster runs no Deployments",
   * and the `Not ready` list would then read a refusal as a clean bill of
   * health — the one thing that section must never do.
   */
  deployments?: DeploymentSummary[];
  statefulSets?: StatefulSetSummary[];
  daemonSets?: DaemonSetSummary[];
  /**
   * Why a kind has no rows, keyed by the slug the rail counts it under. The
   * kinds that answered still render.
   *
   * Keyed rather than a flat list of reasons because two readers want
   * different things from it: the `Not ready` list wants all of them at once,
   * to say why it may be short, and the rail's count row for one kind wants
   * that kind's own reason and no other. A flat array can serve the first and
   * not the second.
   */
  refusals: Partial<Record<ResourceKind, string>>;
  error?: string;
  stale: boolean;
  reload(): void;
}

/**
 * The three scaling kinds the `Not ready` list draws its workload rows from.
 *
 * Deployments, StatefulSets and DaemonSets — the kinds core's `scaledStatus`
 * gives a ready-out-of-desired verdict for. Jobs and CronJobs are deliberately
 * not here: a CronJob has no unhealthy state of its own (the health lives in
 * the Jobs it spawns), and a failed Job's pods are already in the pod list as
 * Pods, with the phase that says what went wrong.
 *
 * One loader over three calls that cannot reject — every `list*` wrapper
 * returns its error rather than throwing — so the fan-out is safe in the way
 * classic's `Promise.all` was not: no branch can cancel the others.
 */
export function useOverviewWorkloads(context: string): OverviewWorkloads {
  const loaded = useCachedResource(key("workloads", context), () =>
    Promise.all([
      // The empty namespace is every namespace: the overview is a whole
      // cluster's view, and so is the list beneath it.
      listDeployments(context, ""),
      listStatefulSets(context, ""),
      listDaemonSets(context, ""),
    ]).then(([deployments, statefulSets, daemonSets]) => {
      const refusals: Partial<Record<ResourceKind, string>> = {};
      if (deployments.error) refusals.deployments = deployments.error;
      if (statefulSets.error) refusals.statefulsets = statefulSets.error;
      if (daemonSets.error) refusals.daemonsets = daemonSets.error;
      return {
        deployments: deployments.deployments,
        statefulSets: statefulSets.statefulsets,
        daemonSets: daemonSets.daemonsets,
        refusals,
      };
    }),
  );

  return {
    status: loaded.status,
    deployments: loaded.data?.deployments,
    statefulSets: loaded.data?.statefulSets,
    daemonSets: loaded.data?.daemonSets,
    refusals: loaded.data?.refusals ?? EMPTY_REFUSALS,
    error: loaded.error,
    stale: loaded.stale,
    reload: loaded.reload,
  };
}

/**
 * One shared empty object rather than a fresh `{}` per render: it is a
 * `useMemo` dependency in {@link useObjectCounts}, and a new identity every
 * render would rebuild those counts every render.
 */
const EMPTY_REFUSALS: Partial<Record<ResourceKind, string>> = {};

export interface OverviewFacts {
  status: ResourceStatus;
  /**
   * The control-plane facts. `provider` and `region` are empty when the
   * cluster named none, and the rail omits those rows — "unknown" as a value
   * would look like an answer.
   */
  facts?: ClusterFacts;
  error?: string;
  stale: boolean;
  reload(): void;
}

/**
 * The rail's Provider, Region and Metrics server rows.
 *
 * `clusterFacts` never rejects: it normalises a failure into empty facts plus
 * a reason. Empty facts are indistinguishable from a cluster that named none,
 * so a carried `error` is mapped back to an error status here rather than
 * being handed to the rail as six silently omitted rows.
 */
export function useClusterFacts(context: string): OverviewFacts {
  const facts = useCachedResource(key("facts", context), () => clusterFacts(context));
  const carried = facts.data?.error;
  return {
    status: carried ? "error" : facts.status,
    facts: facts.data,
    error: carried ?? facts.error,
    stale: facts.stale,
    reload: facts.reload,
  };
}

export interface Overview {
  nodes: OverviewNodes;
  pods: OverviewPods;
  workloads: OverviewWorkloads;
  namespaces: NamespaceCount;
  objects: ObjectCounts;
  facts: OverviewFacts;
  /**
   * Why some of the figures on screen are the last good ones rather than
   * current ones — empty when everything on screen refreshed.
   *
   * The screen states this ONCE, at the top, rather than every band that could
   * not refresh saying it: five tiles and two columns each announcing the same
   * outage has said it seven times and explained it nowhere. That is the same
   * rule the metrics-server absence follows.
   */
  staleReasons: string[];
  /** Retry every section. Each still succeeds or fails on its own. */
  reload(): void;
}

/**
 * Every loader the screen composes, in one call.
 *
 * A single entry point so the sections cannot be wired up inconsistently, but
 * emphatically not a single request: each field below settles on its own
 * schedule and carries its own error, and there is no combined status because
 * there is no moment when "the overview" is loaded or failed as a whole.
 */
export function useOverview(context: string): Overview {
  const pods = useOverviewPods(context);
  const nodes = useOverviewNodes(context, pods.byNode);
  const workloads = useOverviewWorkloads(context);
  const namespaces = useNamespaceCount(context);
  // Fed the two loaders whose kinds it would otherwise list a second time.
  const objects = useObjectCounts(context, workloads, pods);
  const facts = useClusterFacts(context);

  const staleReasons = useMemo(
    () =>
      [
        nodes.stale ? nodes.error : undefined,
        nodes.stale ? nodes.metricsError : undefined,
        pods.stale ? pods.error : undefined,
        workloads.stale ? workloads.error : undefined,
        namespaces.stale ? namespaces.error : undefined,
        objects.stale ? objects.error : undefined,
        facts.stale ? facts.error : undefined,
      ].filter((reason): reason is string => reason !== undefined && reason !== ""),
    [
      nodes.stale,
      nodes.error,
      nodes.metricsError,
      pods.stale,
      pods.error,
      workloads.stale,
      workloads.error,
      namespaces.stale,
      namespaces.error,
      objects.stale,
      objects.error,
      facts.stale,
      facts.error,
    ],
  );

  const reloadNodes = nodes.reload;
  const reloadPods = pods.reload;
  const reloadWorkloads = workloads.reload;
  const reloadNamespaces = namespaces.reload;
  const reloadObjects = objects.reload;
  const reloadFacts = facts.reload;
  const reload = useCallback(() => {
    reloadNodes();
    reloadPods();
    reloadWorkloads();
    reloadNamespaces();
    reloadObjects();
    reloadFacts();
  }, [reloadNodes, reloadPods, reloadWorkloads, reloadNamespaces, reloadObjects, reloadFacts]);

  return { nodes, pods, workloads, namespaces, objects, facts, staleReasons, reload };
}
