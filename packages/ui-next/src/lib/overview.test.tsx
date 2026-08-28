import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import type {
  ClusterFacts,
  PodOverview,
  DaemonSetSummary,
  DeploymentSummary,
  NodeSummary,
  PodSummary,
  StatefulSetSummary,
} from "@srelens/core";

// `vi.hoisted` because `vi.mock` is hoisted above every declaration in this
// file — the same pattern resourceList.test.tsx uses. Only the six capability
// wrappers are replaced; `nodeUsage`, `clusterCapacity` and `K8S_KIND` stay
// real, so these tests assert against core's arithmetic rather than a copy.
const core = vi.hoisted(() => ({
  listNodes: vi.fn(),
  nodeMetrics: vi.fn(),
  podOverview: vi.fn(),
  listNamespaces: vi.fn(),
  listResource: vi.fn(),
  listDeployments: vi.fn(),
  listStatefulSets: vi.fn(),
  listDaemonSets: vi.fn(),
  clusterFacts: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

import { clearResourceCache } from "./cachedResource";
import {
  OVERVIEW_KINDS,
  useClusterFacts,
  useNamespaceCount,
  useOverview,
  useOverviewNodes,
  useOverviewPods,
  useOverviewWorkloads,
} from "./overview";

function aNode(name: string, over: Partial<NodeSummary> = {}): NodeSummary {
  return {
    name,
    status: "Ready",
    unschedulable: false,
    taints: 0,
    version: "v1.31.4",
    roles: "worker",
    age: "10d",
    allocatableCpuMillicores: 4000,
    allocatableMemoryMiB: 16000,
    allocatablePods: 50,
    instanceType: "",
    ...over,
  };
}

function aPod(name: string, node: string, over: Partial<PodSummary> = {}): PodSummary {
  return {
    name,
    namespace: "default",
    phase: "Running",
    ready: "1/1",
    restarts: 0,
    node,
    age: "3d",
    image: "acme/api:1",
    ...over,
  };
}

/**
 * A `podOverview` answer built from a list of pods — the counts grouped the
 * way the backend groups them, so a fixture reads as a cluster rather than as
 * three hand-written numbers that can drift from each other.
 *
 * `unsettled` is every pod that is not simply running, which is what the
 * backend sends: a superset core then judges.
 */
function anOverview(pods: PodSummary[], over: Partial<PodOverview> = {}): PodOverview {
  const byNode = new Map<string, number>();
  for (const pod of pods) if (pod.node) byNode.set(pod.node, (byNode.get(pod.node) ?? 0) + 1);
  return {
    total: pods.length,
    byNode: [...byNode].map(([node, count]) => ({ node, pods: count })),
    unsettled: pods.filter((p) => p.phase !== "Running" || p.waitingReason),
    truncated: false,
    ...over,
  };
}

/** The per-node map `useOverviewNodes` takes, from a list of pods. */
function podsPerNode(pods: PodSummary[]): Map<string, number> {
  return new Map(anOverview(pods).byNode.map((n) => [n.node, n.pods]));
}

function aDeployment(name: string, ready: string): DeploymentSummary {
  return { name, namespace: "checkout", ready, upToDate: 1, available: 1, age: "8d" };
}

function aStatefulSet(name: string, ready: string): StatefulSetSummary {
  return { name, namespace: "payments", ready, updated: 1, service: "svc", age: "8d" };
}

function aDaemonSet(name: string, ready: number, desired: number): DaemonSetSummary {
  return {
    name,
    namespace: "kube-system",
    desired,
    current: ready,
    ready,
    upToDate: ready,
    available: ready,
    age: "40d",
  };
}

const NO_FACTS: ClusterFacts = {
  context: "prod",
  provider: "",
  region: "",
  metricsServer: { state: "unknown", version: "" },
};

/**
 * The loaders keep their last good answer in a module-level cache, so a test
 * that did not clear it would be handed the previous test's cluster.
 */
beforeEach(() => {
  clearResourceCache();
});

/** Every loader answers something harmless; each test overrides what it cares about. */
function allQuiet() {
  core.listNodes.mockResolvedValue({ nodes: [] });
  core.nodeMetrics.mockResolvedValue({ metrics: [] });
  core.podOverview.mockResolvedValue({ pods: anOverview([]) });
  core.listNamespaces.mockResolvedValue({ namespaces: [] });
  core.listResource.mockResolvedValue({ items: [] });
  core.listDeployments.mockResolvedValue({ deployments: [] });
  core.listStatefulSets.mockResolvedValue({ statefulsets: [] });
  core.listDaemonSets.mockResolvedValue({ daemonsets: [] });
  core.clusterFacts.mockResolvedValue(NO_FACTS);
}

describe("useOverviewNodes", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    allQuiet();
  });

  it("pairs each node with its usage against its own allocatable", async () => {
    core.listNodes.mockResolvedValue({ nodes: [aNode("a1"), aNode("b2", { allocatableCpuMillicores: 8000 })] });
    core.nodeMetrics.mockResolvedValue({
      metrics: [
        { name: "a1", cpuMillicores: 2000, memoryMiB: 8000 },
        { name: "b2", cpuMillicores: 2000, memoryMiB: 4000 },
      ],
    });

    const { result } = renderHook(() => useOverviewNodes("prod", undefined));
    await waitFor(() => expect(result.current.status).toBe("ready"));

    expect(result.current.nodes.map((n) => n.node.name)).toEqual(["a1", "b2"]);
    expect(result.current.nodes[0].usage.cpuPercent).toBe(50);
    expect(result.current.nodes[1].usage.cpuPercent).toBe(25);
    expect(result.current.nodes[1].usage.memoryPercent).toBe(25);
    expect(result.current.capacity).toEqual({
      cpu: { usedMillicores: 4000, allocatableMillicores: 12000 },
      memory: { usedMiB: 12000, allocatableMiB: 32000 },
      nodesReporting: 2,
      nodesTotal: 2,
    });
  });

  it("keeps the rows when metrics-server is absent, and reads null rather than zero", async () => {
    core.listNodes.mockResolvedValue({ nodes: [aNode("a1")] });
    core.nodeMetrics.mockResolvedValue({ error: "the server could not find the requested resource" });

    const { result } = renderHook(() => useOverviewNodes("prod", undefined));
    await waitFor(() => expect(result.current.status).toBe("ready"));

    expect(result.current.nodes).toHaveLength(1);
    expect(result.current.nodes[0].usage.cpuPercent).toBeNull();
    expect(result.current.nodes[0].usage.memoryPercent).toBeNull();
    expect(result.current.metricsError).toContain("could not find");
    // The node list itself did not fail, so the table is not in an error state.
    expect(result.current.error).toBeUndefined();
    expect(result.current.capacity.nodesTotal).toBe(1);
    expect(result.current.capacity.cpu).toBeNull();
  });

  it("reports a node above its allocatable unrounded and uncapped", async () => {
    core.listNodes.mockResolvedValue({ nodes: [aNode("hot", { allocatableCpuMillicores: 1000 })] });
    core.nodeMetrics.mockResolvedValue({ metrics: [{ name: "hot", cpuMillicores: 1400, memoryMiB: 1234 }] });

    const { result } = renderHook(() => useOverviewNodes("prod", undefined));
    await waitFor(() => expect(result.current.status).toBe("ready"));

    expect(result.current.nodes[0].usage.cpuPercent).toBe(140);
    expect(result.current.nodes[0].usage.memoryPercent).toBeCloseTo(7.7125, 6);
  });

  it("counts the pods on each node, and a node with none reads zero", async () => {
    core.listNodes.mockResolvedValue({ nodes: [aNode("a1"), aNode("b2")] });
    const pods = [aPod("p1", "a1"), aPod("p2", "a1"), aPod("p3", "b2")];

    const { result } = renderHook(() => useOverviewNodes("prod", podsPerNode(pods)));
    await waitFor(() => expect(result.current.status).toBe("ready"));

    expect(result.current.nodes[0].usage.pods).toEqual({ used: 2, allocatable: 50 });
    expect(result.current.nodes[1].usage.pods).toEqual({ used: 1, allocatable: 50 });

    // A node the grouping does not mention genuinely runs none: the map is
    // complete, so its absence is a reading rather than a gap.
    const empty = renderHook(() => useOverviewNodes("prod", podsPerNode([aPod("p1", "a1")])));
    await waitFor(() => expect(empty.result.current.status).toBe("ready"));
    expect(empty.result.current.nodes[1].usage.pods).toEqual({ used: 0, allocatable: 50 });
  });

  it("reads no pod count at all while the grouping is unknown", async () => {
    core.listNodes.mockResolvedValue({ nodes: [aNode("a1")] });

    const { result } = renderHook(() => useOverviewNodes("prod", undefined));
    await waitFor(() => expect(result.current.status).toBe("ready"));

    // Not `{ used: 0 }`: nobody has told us there are no pods on this node.
    expect(result.current.nodes[0].usage.pods).toBeNull();
  });

  it("passes through a node that reports no allocatable pods rather than papering over it", async () => {
    core.listNodes.mockResolvedValue({ nodes: [aNode("a1", { allocatablePods: 0 })] });

    const { result } = renderHook(() =>
      useOverviewNodes("prod", podsPerNode([aPod("p1", "a1"), aPod("p2", "a1")])),
    );
    await waitFor(() => expect(result.current.status).toBe("ready"));

    expect(result.current.nodes[0].usage.pods).toEqual({ used: 2, allocatable: 0 });
  });

  it("empties the table and says why when the node list is refused", async () => {
    core.listNodes.mockResolvedValue({ error: 'nodes is forbidden: User "dev" cannot list resource "nodes"' });
    core.nodeMetrics.mockResolvedValue({ metrics: [{ name: "a1", cpuMillicores: 1, memoryMiB: 1 }] });

    const { result } = renderHook(() => useOverviewNodes("prod", undefined));
    await waitFor(() => expect(result.current.status).toBe("error"));

    expect(result.current.nodes).toEqual([]);
    expect(result.current.error).toContain("forbidden");
  });

  it("discards a node list that arrives after the reader moved to another cluster", async () => {
    let settleFirst: (value: { nodes: NodeSummary[] }) => void = () => {};
    core.listNodes.mockImplementationOnce(
      () => new Promise<{ nodes: NodeSummary[] }>((resolve) => { settleFirst = resolve; }),
    );
    core.listNodes.mockResolvedValue({ nodes: [aNode("staging-1")] });

    const { result, rerender } = renderHook(({ context }: { context: string }) => useOverviewNodes(context, undefined), {
      initialProps: { context: "prod" },
    });
    rerender({ context: "staging" });
    await waitFor(() => expect(result.current.nodes).toHaveLength(1));

    await act(async () => {
      settleFirst({ nodes: [aNode("prod-1"), aNode("prod-2")] });
    });

    expect(result.current.nodes.map((n) => n.node.name)).toEqual(["staging-1"]);
  });
});

describe("useOverviewPods", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    allQuiet();
  });

  it("counts the cluster's pods rather than listing them", async () => {
    core.podOverview.mockResolvedValue({
      pods: anOverview([aPod("p1", "a1"), aPod("p2", "b2"), aPod("p3", "b2")]),
    });

    const { result } = renderHook(() => useOverviewPods("prod"));
    expect(result.current.status).toBe("loading");
    // Not `0` while loading: a zero would count as "this cluster has no pods"
    // to every consumer, which is the null-is-not-zero mistake one level up.
    expect(result.current.total).toBeNull();
    expect(result.current.byNode).toBeUndefined();

    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(core.podOverview).toHaveBeenCalledWith("prod");
    expect(result.current.total).toBe(3);
    expect([...(result.current.byNode ?? [])]).toEqual([
      ["a1", 1],
      ["b2", 2],
    ]);
  });

  it("carries only the pods that are not simply running", async () => {
    const sick = aPod("crash", "a1", { waitingReason: "CrashLoopBackOff", ready: "0/1" });
    core.podOverview.mockResolvedValue({ pods: anOverview([aPod("ok", "a1"), sick]) });

    const { result } = renderHook(() => useOverviewPods("prod"));
    await waitFor(() => expect(result.current.status).toBe("ready"));

    // The total counts every pod; `unsettled` is the handful worth reading.
    expect(result.current.total).toBe(2);
    expect(result.current.unsettled?.map((p) => p.name)).toEqual(["crash"]);
  });

  it("reports a refusal without inventing an empty cluster", async () => {
    core.podOverview.mockResolvedValue({ error: "pods is forbidden" });

    const { result } = renderHook(() => useOverviewPods("prod"));
    await waitFor(() => expect(result.current.status).toBe("error"));
    // A cluster that did not answer has not told us it has no pods.
    expect(result.current.total).toBeNull();
    expect(result.current.byNode).toBeUndefined();
    expect(result.current.error).toContain("forbidden");
  });

  it("reads a cluster with no pods as an answer rather than an empty section", async () => {
    core.podOverview.mockResolvedValue({ pods: anOverview([]) });

    const { result } = renderHook(() => useOverviewPods("prod"));
    await waitFor(() => expect(result.current.total).toBe(0));
    // "empty" is a state the screen draws an EmptyState for; a cluster that
    // answered zero has answered, and its tile shows the zero.
    expect(result.current.status).toBe("ready");
  });

  it("carries the backend's cap rather than presenting a short list as whole", async () => {
    core.podOverview.mockResolvedValue({
      pods: anOverview([aPod("p1", "a1", { phase: "Failed" })], { truncated: true }),
    });

    const { result } = renderHook(() => useOverviewPods("prod"));
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(result.current.truncated).toBe(true);
  });
});

describe("useNamespaceCount", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    allQuiet();
  });

  it("counts them", async () => {
    core.listNamespaces.mockResolvedValue({ namespaces: ["default", "kube-system", "checkout"] });

    const { result } = renderHook(() => useNamespaceCount("prod"));
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(result.current.count).toBe(3);
  });

  it("has no count at all when the list is refused", async () => {
    core.listNamespaces.mockResolvedValue({ error: "namespaces is forbidden" });

    const { result } = renderHook(() => useNamespaceCount("prod"));
    await waitFor(() => expect(result.current.status).toBe("error"));
    expect(result.current.count).toBeNull();
  });
});

/**
 * The counts now have two sources, so they are exercised through `useOverview`
 * rather than alone: four of the six come off the loaders that already hold
 * those kinds, and wiring them by hand in a test would be a second answer to
 * the question the hook exists to settle.
 */
describe("useObjectCounts", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    allQuiet();
  });

  it("counts one kind per row, in the rail's order", async () => {
    core.listDeployments.mockResolvedValue({ deployments: [aDeployment("a", "1/1"), aDeployment("b", "1/1")] });
    core.listResource.mockResolvedValue({ items: [] });

    const { result } = renderHook(() => useOverview("prod"));
    await waitFor(() => expect(result.current.objects.status).toBe("ready"));

    expect(result.current.objects.counts.map((c) => c.slug)).toEqual(OVERVIEW_KINDS);
    expect(result.current.objects.counts[0]).toEqual({ slug: "deployments", count: 2, error: undefined });
  });

  it("lists only the kinds nothing else on the screen loads", async () => {
    core.listDeployments.mockResolvedValue({ deployments: [aDeployment("a", "1/1")] });
    core.listStatefulSets.mockResolvedValue({ statefulsets: [aStatefulSet("b", "1/1")] });
    core.listDaemonSets.mockResolvedValue({ daemonsets: [aDaemonSet("c", 1, 1)] });
    core.podOverview.mockResolvedValue({ pods: anOverview([aPod("p1", "n1"), aPod("p2", "n1")]) });
    core.listResource.mockResolvedValue({ items: [{ name: "x" }] });

    const { result } = renderHook(() => useOverview("prod"));
    await waitFor(() => expect(result.current.objects.status).toBe("ready"));

    const count = (slug: string) => result.current.objects.counts.find((c) => c.slug === slug)?.count;
    expect(count("deployments")).toBe(1);
    expect(count("statefulsets")).toBe(1);
    expect(count("daemonsets")).toBe(1);
    expect(count("pods")).toBe(2);
    expect(count("cronjobs")).toBe(1);
    expect(count("jobs")).toBe(1);

    // The collapse: four of the six kinds were already fetched, so the generic
    // list goes out twice rather than six times — and the pod count, the one
    // that used to cost a whole cluster's pod bodies, is a count the backend
    // made and this screen asked for once.
    expect(core.listResource).toHaveBeenCalledTimes(2);
    expect(core.listResource.mock.calls.map((c: unknown[]) => c[1])).toEqual(["CronJob", "Job"]);
    expect(core.podOverview).toHaveBeenCalledTimes(1);
    expect(core.listDeployments).toHaveBeenCalledTimes(1);
  });

  it("keeps every other kind's count when one kind is refused", async () => {
    core.listResource.mockImplementation((_context: string, kind: string) =>
      kind === "Job"
        ? Promise.resolve({ error: 'jobs is forbidden: User "dev" cannot list resource "jobs"' })
        : Promise.resolve({ items: [{ name: "a" }] }),
    );
    core.listDeployments.mockResolvedValue({ deployments: [aDeployment("a", "1/1")] });

    const { result } = renderHook(() => useOverview("prod"));
    await waitFor(() => expect(result.current.objects.status).toBe("ready"));

    const jobs = result.current.objects.counts.find((c) => c.slug === "jobs");
    expect(jobs?.count).toBeNull();
    expect(jobs?.error).toContain("forbidden");
    expect(result.current.objects.counts.find((c) => c.slug === "cronjobs")?.count).toBe(1);
    expect(result.current.objects.counts.find((c) => c.slug === "deployments")?.count).toBe(1);
  });

  it("carries a refused kind's own reason onto that kind's row alone", async () => {
    core.listDeployments.mockResolvedValue({ error: 'deployments is forbidden: User "dev" cannot list' });
    core.podOverview.mockResolvedValue({ error: "pods is forbidden" });

    const { result } = renderHook(() => useOverview("prod"));
    await waitFor(() => expect(result.current.objects.status).toBe("ready"));

    const row = (slug: string) => result.current.objects.counts.find((c) => c.slug === slug);
    // Never `0`, which reads as a cluster with no Deployments at all.
    expect(row("deployments")?.count).toBeNull();
    expect(row("deployments")?.error).toContain("deployments is forbidden");
    expect(row("pods")?.count).toBeNull();
    expect(row("pods")?.error).toContain("pods is forbidden");
    // The kinds that answered are untouched by either refusal.
    expect(row("statefulsets")?.count).toBe(0);
    expect(row("statefulsets")?.error).toBeUndefined();
  });
});

describe("useClusterFacts", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    allQuiet();
  });

  it("returns the control-plane facts", async () => {
    core.clusterFacts.mockResolvedValue({
      context: "prod",
      provider: "GKE",
      region: "europe-west4",
      metricsServer: { state: "present", version: "v1beta1" },
    });

    const { result } = renderHook(() => useClusterFacts("prod"));
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(result.current.facts?.provider).toBe("GKE");
    expect(result.current.facts?.metricsServer.version).toBe("v1beta1");
  });

  it("surfaces the wrapper's own error rather than presenting empty facts as an answer", async () => {
    core.clusterFacts.mockResolvedValue({ ...NO_FACTS, error: "connection refused" });

    const { result } = renderHook(() => useClusterFacts("prod"));
    await waitFor(() => expect(result.current.status).toBe("error"));
    expect(result.current.error).toContain("connection refused");
  });
});

describe("useOverviewWorkloads", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    allQuiet();
  });

  it("lists the three scaling kinds across every namespace", async () => {
    core.listDeployments.mockResolvedValue({ deployments: [aDeployment("checkout-api", "9/12")] });
    core.listStatefulSets.mockResolvedValue({ statefulsets: [aStatefulSet("payments-db", "1/3")] });
    core.listDaemonSets.mockResolvedValue({ daemonsets: [aDaemonSet("log-agent", 2, 4)] });

    const { result } = renderHook(() => useOverviewWorkloads("prod"));
    await waitFor(() => expect(result.current.status).toBe("ready"));

    expect(result.current.deployments?.map((d) => d.name)).toEqual(["checkout-api"]);
    expect(result.current.statefulSets?.map((s) => s.name)).toEqual(["payments-db"]);
    expect(result.current.daemonSets?.map((d) => d.name)).toEqual(["log-agent"]);
    // The empty namespace is "every namespace"; the overview is cluster-wide.
    expect(core.listDeployments).toHaveBeenCalledWith("prod", "");
    expect(core.listStatefulSets).toHaveBeenCalledWith("prod", "");
    expect(core.listDaemonSets).toHaveBeenCalledWith("prod", "");
  });

  it("keeps the kinds that answered when one of them is refused", async () => {
    core.listDeployments.mockResolvedValue({ deployments: [aDeployment("checkout-api", "9/12")] });
    core.listStatefulSets.mockResolvedValue({ error: 'statefulsets is forbidden: User "dev" cannot list' });
    core.listDaemonSets.mockResolvedValue({ daemonsets: [aDaemonSet("log-agent", 2, 4)] });

    const { result } = renderHook(() => useOverviewWorkloads("prod"));
    await waitFor(() => expect(result.current.status).toBe("ready"));

    expect(result.current.deployments).toHaveLength(1);
    expect(result.current.daemonSets).toHaveLength(1);
    // Not an empty list, which would read as a cluster with no StatefulSets.
    expect(result.current.statefulSets).toBeUndefined();
    expect(result.current.refusals).toEqual({
      statefulsets: 'statefulsets is forbidden: User "dev" cannot list',
    });
  });

  it("carries no reasons at all when every kind answered", async () => {
    const { result } = renderHook(() => useOverviewWorkloads("prod"));
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(result.current.refusals).toEqual({});
  });
});

describe("useOverview", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    allQuiet();
  });

  // The property this module exists for. Classic's ClusterOverview fires six
  // calls in parallel and throws on the first error, so one refused list blanks
  // the entire dashboard.
  it("leaves every other section's data on screen when one loader fails", async () => {
    core.listNodes.mockResolvedValue({ error: 'nodes is forbidden: User "dev" cannot list resource "nodes"' });
    core.podOverview.mockResolvedValue({ pods: anOverview([aPod("p1", "a1"), aPod("p2", "b2")]) });
    core.listNamespaces.mockResolvedValue({ namespaces: ["default", "checkout"] });
    core.listResource.mockResolvedValue({ items: [{ name: "a" }, { name: "b" }, { name: "c" }] });
    core.clusterFacts.mockResolvedValue({
      context: "prod",
      provider: "kind",
      region: "",
      metricsServer: { state: "present", version: "v1beta1" },
    });

    const { result } = renderHook(() => useOverview("prod"));
    await waitFor(() => expect(result.current.nodes.status).toBe("error"));
    await waitFor(() => expect(result.current.namespaces.status).toBe("ready"));

    expect(result.current.nodes.nodes).toEqual([]);
    expect(result.current.nodes.error).toContain("forbidden");

    expect(result.current.pods.total).toBe(2);
    expect(result.current.namespaces.count).toBe(2);
    // The two kinds the rail lists for itself answered 3; the pod count came
    // off the pod facts this same render already has.
    const count = (slug: string) => result.current.objects.counts.find((c) => c.slug === slug)?.count;
    expect(count("cronjobs")).toBe(3);
    expect(count("jobs")).toBe(3);
    expect(count("pods")).toBe(2);
    expect(result.current.facts.facts?.provider).toBe("kind");
    expect(result.current.workloads.status).toBe("ready");
  });

  it("feeds the backend's grouping into the nodes' pod counts, with one call", async () => {
    core.listNodes.mockResolvedValue({ nodes: [aNode("a1"), aNode("b2")] });
    core.podOverview.mockResolvedValue({
      pods: anOverview([aPod("p1", "a1"), aPod("p2", "a1"), aPod("p3", "b2")]),
    });

    const { result } = renderHook(() => useOverview("prod"));
    await waitFor(() => expect(result.current.nodes.nodes[0]?.usage.pods).toEqual({ used: 2, allocatable: 50 }));

    expect(result.current.nodes.nodes[1].usage.pods).toEqual({ used: 1, allocatable: 50 });
    // The Pods tile, the per-node column and the `Not ready` list, from one
    // call that never listed a pod.
    expect(core.podOverview).toHaveBeenCalledTimes(1);
  });

  it("paints the whole screen from cache when the reader comes back to it", async () => {
    core.listNodes.mockResolvedValue({ nodes: [aNode("a1")] });
    core.podOverview.mockResolvedValue({ pods: anOverview([aPod("p1", "a1")]) });

    const first = renderHook(() => useOverview("prod"));
    await waitFor(() => expect(first.result.current.nodes.status).toBe("ready"));
    first.unmount();

    const again = renderHook(() => useOverview("prod"));
    // No loading state anywhere and not one extra request: this is what
    // makes returning to the tab instant.
    expect(again.result.current.nodes.status).toBe("ready");
    expect(again.result.current.pods.total).toBe(1);
    expect(core.listNodes).toHaveBeenCalledTimes(1);
    expect(core.podOverview).toHaveBeenCalledTimes(1);
    expect(core.clusterFacts).toHaveBeenCalledTimes(1);
  });

  it("never serves one cluster's cached answer for another", async () => {
    core.listNodes.mockImplementation((context: string) =>
      Promise.resolve({ nodes: [aNode(`${context}-1`)] }),
    );

    const prod = renderHook(() => useOverview("prod"));
    await waitFor(() => expect(prod.result.current.nodes.nodes[0]?.node.name).toBe("prod-1"));

    const staging = renderHook(() => useOverview("staging"));
    await waitFor(() => expect(staging.result.current.nodes.nodes[0]?.node.name).toBe("staging-1"));
    expect(prod.result.current.nodes.nodes[0].node.name).toBe("prod-1");
  });

  it("keeps the figures on screen when a refresh fails, and says they are stale", async () => {
    core.listNodes.mockResolvedValueOnce({ nodes: [aNode("a1")] });
    core.listNodes.mockResolvedValue({ error: 'nodes is forbidden: User "dev" cannot list' });

    const { result } = renderHook(() => useOverview("prod"));
    await waitFor(() => expect(result.current.nodes.nodes).toHaveLength(1));

    act(() => result.current.reload());
    await waitFor(() => expect(result.current.nodes.stale).toBe(true));

    // The rows the reader can see are still real, and the screen is told
    // they stopped refreshing rather than being left to present them as
    // current.
    expect(result.current.nodes.nodes.map((n) => n.node.name)).toEqual(["a1"]);
    expect(result.current.staleReasons.join(" ")).toContain("forbidden");
  });

  it("reloads every section at once", async () => {
    const { result } = renderHook(() => useOverview("prod"));
    await waitFor(() => expect(result.current.facts.status).toBe("ready"));

    act(() => result.current.reload());
    await waitFor(() => expect(core.listNodes).toHaveBeenCalledTimes(2));
    expect(core.podOverview).toHaveBeenCalledTimes(2);
    expect(core.listNamespaces).toHaveBeenCalledTimes(2);
    expect(core.listDeployments).toHaveBeenCalledTimes(2);
    expect(core.clusterFacts).toHaveBeenCalledTimes(2);
  });
});
