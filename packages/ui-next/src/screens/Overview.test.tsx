import { describe, it, expect, vi, beforeEach } from "vitest";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Only the capability wrappers are replaced. `nodeUsage`, `clusterCapacity`,
// `nodeStatus`, `podStatus` and `resourceStatusLine` stay real, so every
// assertion below is against core's own arithmetic and core's own status
// vocabulary rather than a copy of either.
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
  podCount: vi.fn(),
  cordonNode: vi.fn(),
  drainNode: vi.fn(),
  copyKubectlCommand: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}
const proto = window.HTMLElement.prototype as unknown as Record<string, unknown>;
proto.scrollIntoView ??= () => {};
proto.hasPointerCapture ??= () => false;
proto.setPointerCapture ??= () => {};
proto.releasePointerCapture ??= () => {};

import {
  K8S_KIND,
  RESOURCE_LABELS,
  resourceStatusLine,
  scaledStatus,
  type ClusterContext,
  type ClusterFacts,
  type DaemonSetSummary,
  type DeploymentSummary,
  type K8sObject,
  type NodeSummary,
  type PodOverview,
  type PodSummary,
  type ResourceKind,
  type StatefulSetSummary,
} from "@srelens/core";
import { Overview } from "./Overview";
import { CACHE_TTL_MS, clearResourceCache } from "../lib/cachedResource";
import { ConsoleProvider } from "../console";
import { defaultState } from "../lib/tabs";
import * as store from "../lib/tabsStore";
import { resetContexts, setContexts, setKubeconfigFiles } from "../lib/clusters";
import { probeCluster, resetProbes } from "../lib/probe";
import { resetView, setLink, type LinkState } from "../lib/workspace";

const CTX: ClusterContext = {
  name: "prod-eu",
  stableId: "prod",
  cluster: "prod",
  server: "https://prod",
  isCurrent: true,
};

const ROUTE = "/overview";

/** A 401 as `String(e)` over a `CapabilityError` actually delivers it. */
const API_401 =
  'Error: handler error: ApiError: Unauthorized: Unauthorized (Status { status: Some("Failure"), ' +
  "metadata: Some(ListMeta { continue_: None, remaining_item_count: None, resource_version: None, " +
  'self_link: None }), reason: Some("Unauthorized"), code: Some(401), message: Some("Unauthorized") })';

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
 * A `podOverview` answer built from a list of pods.
 *
 * The screen never sees a pod list any more — the backend counts the cluster's
 * pods and groups them by node server-side, and sends bodies only for the ones
 * that are not simply running. A fixture that starts from pods and derives all
 * three keeps the numbers in a test consistent with each other the way the
 * backend keeps them consistent on a real cluster.
 *
 * `unsettled` mirrors `crates/kube/src/pod_overview.rs`: the union of the
 * `status.phase!=Running` field selector and every pod the printed pod table's
 * READY column shows short of ready. It used to read `phase !== "Running" ||
 * p.waitingReason`, which is a NARROWER list than the backend actually sends —
 * a crash-looping pod between restarts has phase `Running` and no waiting
 * reason, and the real `short_of_ready` still fetches it by name. Modelling
 * the backend as narrower than it is hid the very pod this screen was losing.
 */
function anOverview(pods: PodSummary[], over: Partial<PodOverview> = {}): PodOverview {
  const byNode = new Map<string, number>();
  for (const pod of pods) if (pod.node) byNode.set(pod.node, (byNode.get(pod.node) ?? 0) + 1);
  const shortOfReady = (ready: string) => {
    const [have, want] = ready.split("/").map(Number);
    return !(have >= want);
  };
  return {
    total: pods.length,
    byNode: [...byNode].map(([node, count]) => ({ node, pods: count })),
    unsettled: pods.filter((p) => p.phase !== "Running" || shortOfReady(p.ready)),
    truncated: false,
    ...over,
  };
}

/** The three-node cluster every test starts from: all Ready, all reporting. */
const NODES = [aNode("n1"), aNode("n2"), aNode("n3")];
const METRICS = NODES.map((n) => ({ name: n.name, cpuMillicores: 2800, memoryMiB: 12000 }));
const PODS = [
  aPod("api-1", "n1"),
  aPod("api-2", "n1"),
  aPod("web-1", "n2"),
  // Phase `Running` with a waiting container: core calls this
  // `CrashLoopBackOff` and flags it, which is the one unhealthy pod here.
  aPod("worker-1", "n3", { phase: "Running", ready: "0/1", waitingReason: "CrashLoopBackOff" }),
];

function aDeployment(name: string, ready: string, namespace = "checkout"): DeploymentSummary {
  return { name, namespace, ready, upToDate: 1, available: 1, age: "8d" };
}

function aStatefulSet(name: string, ready: string, namespace = "payments"): StatefulSetSummary {
  return { name, namespace, ready, updated: 1, service: "svc", age: "8d" };
}

function aDaemonSet(name: string, ready: number, desired: number, namespace = "kube-system"): DaemonSetSummary {
  return {
    name,
    namespace,
    desired,
    current: ready,
    ready,
    upToDate: ready,
    available: ready,
    age: "40d",
  };
}

const NO_FACTS: ClusterFacts = {
  context: "prod-eu",
  provider: "",
  region: "",
  metricsServer: { state: "unknown", version: "" },
};

function quiet() {
  core.listNodes.mockResolvedValue({ nodes: NODES });
  core.nodeMetrics.mockResolvedValue({ metrics: METRICS });
  core.podOverview.mockResolvedValue({ pods: anOverview(PODS) });
  core.listNamespaces.mockResolvedValue({ namespaces: ["default", "kube-system", "prod", "obs"] });
  core.listResource.mockResolvedValue({ items: [] });
  core.listDeployments.mockResolvedValue({ deployments: [] });
  core.listStatefulSets.mockResolvedValue({ statefulsets: [] });
  core.listDaemonSets.mockResolvedValue({ daemonsets: [] });
  core.clusterFacts.mockResolvedValue(NO_FACTS);
  core.podCount.mockResolvedValue({ counts: { running: 4, total: 4 } });
  core.cordonNode.mockResolvedValue({ ok: true });
  core.drainNode.mockResolvedValue({ evicted: 3, skipped: 0 });
}

beforeEach(() => {
  vi.clearAllMocks();
  // The loaders remember their last good answer, so a test that did not clear
  // the cache would be handed the previous test's cluster.
  clearResourceCache();
  quiet();
  resetContexts();
  setContexts([CTX]);
  setKubeconfigFiles(["/home/u/.kube/config"]);
  store.setState(defaultState([CTX]));
  // The rail reads both: the probe for the server version, the workspace view
  // for the link. Neither survives a test.
  resetProbes();
  resetView();
});

function open() {
  store.openTab(ROUTE);
  return render(
    <ConsoleProvider>
      <Overview route={ROUTE} />
    </ConsoleProvider>,
  );
}

/** One tile of the capacity strip, found by the label above its figure. */
function tile(label: string): HTMLElement {
  const strip = document.querySelector('[data-slot="capacity"]');
  if (!strip) throw new Error("no capacity strip on screen");
  const found = within(strip as HTMLElement).getByText(label).closest(".stat");
  if (!found) throw new Error(`no ${label} tile`);
  return found as HTMLElement;
}

const value = (label: string) => tile(label).querySelector(".stat-value")?.textContent;
const caption = (label: string) => {
  const parts = Array.from(tile(label).children);
  // The delta is the third child when there is one — label, value, delta.
  return parts.length > 2 ? (parts[2].textContent ?? null) : null;
};
const tone = (label: string) => tile(label).getAttribute("data-tone");

const rowFor = (name: string) => screen.getByText(name).closest("tr") as HTMLElement;
const headers = () =>
  Array.from(document.querySelectorAll("thead th")).map((th) => th.textContent?.trim() ?? "");
const cells = (row: HTMLElement) => Array.from(row.querySelectorAll("td")).map((td) => td.textContent?.trim() ?? "");
const dialog = () => document.querySelector('[data-slot="dialog-content"]') as HTMLElement | null;

describe("Overview — the capacity strip", () => {
  it("counts the cluster, and says in the caption what is wrong with it", async () => {
    open();
    await waitFor(() => expect(value("Nodes")).toBe("3"));

    // Every node is Ready: the caption says so, in the ok tone.
    expect(caption("Nodes")).toBe("all ready");
    expect(tone("Nodes")).toBe("ok");

    // One of the four pods is in CrashLoopBackOff, which core flags.
    expect(value("Pods")).toBe("4");
    expect(caption("Pods")).toBe("1 not ready");
    expect(tone("Pods")).toBe("sev");

    // The one tile the design gives no caption at all.
    expect(value("Namespaces")).toBe("4");
    expect(caption("Namespaces")).toBeNull();
  });

  it("says how many nodes are not ready rather than that they all are", async () => {
    core.listNodes.mockResolvedValue({
      nodes: [aNode("n1"), aNode("n2", { status: "NotReady" }), aNode("n3", { status: "NotReady" })],
    });
    open();

    await waitFor(() => expect(caption("Nodes")).toBe("2 not ready"));
    expect(tone("Nodes")).toBe("sev");
  });

  it("reads a cordoned node as cordoned, not as not ready", async () => {
    core.listNodes.mockResolvedValue({
      nodes: [aNode("n1"), aNode("n2"), aNode("n3", { unschedulable: true })],
    });
    open();

    await waitFor(() => expect(caption("Nodes")).toBe("1 cordoned"));
    expect(tone("Nodes")).toBe("warn");
  });

  it("shows CPU and memory as a share of what the cluster allocated", async () => {
    open();

    // 3 nodes x 2800m of 4000m allocatable.
    await waitFor(() => expect(value("CPU")).toBe("70%"));
    expect(caption("CPU")).toBe("8.4 / 12 cores");
    expect(tone("CPU")).toBe("warn");

    // 3 nodes x 12000MiB of 16000MiB allocatable.
    expect(value("Memory")).toBe("75%");
    expect(caption("Memory")).toBe("35.2Gi / 46.9Gi");
    expect(tone("Memory")).toBe("warn");
  });

  it("qualifies a partial total instead of showing it as the whole cluster", async () => {
    // Two of three nodes reported: the sums describe those two only.
    core.nodeMetrics.mockResolvedValue({ metrics: METRICS.slice(0, 2) });
    open();

    await waitFor(() => expect(value("CPU")).toBe("70%"));
    expect(caption("CPU")).toBe("5.6 / 8 cores · 2 of 3 nodes reporting");
    expect(caption("Memory")).toBe("23.4Gi / 31.3Gi · 2 of 3 nodes reporting");
  });

  it("reads a cluster with no metrics as no reading, never as 0%", async () => {
    core.nodeMetrics.mockResolvedValue({ error: "the server could not find the requested resource" });
    open();

    await waitFor(() => expect(value("CPU")).toBe("No reading"));
    expect(value("Memory")).toBe("No reading");
    // No figure at all, and no caption pretending to a total.
    expect(caption("CPU")).toBeNull();
    expect(caption("Memory")).toBeNull();
    expect(screen.queryByText("0%")).toBeNull();
    // The absence is the rail's to state, once — not five tiles announcing it.
    expect(screen.queryByText(/metrics-server/i)).toBeNull();
  });

  it("does not count a refused list as an empty cluster", async () => {
    core.listNamespaces.mockResolvedValue({ error: "namespaces is forbidden" });
    open();

    await waitFor(() => expect(value("Namespaces")).toBe("No reading"));
    // The other tiles are untouched by that one refusal.
    expect(value("Nodes")).toBe("3");
    expect(value("Pods")).toBe("4");
  });
});

describe("Overview — the nodes table", () => {
  it("draws the design's columns, in its order", async () => {
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());
    expect(headers()).toEqual(["Name", "Pool", "State", "CPU", "Memory", "Pods", ""]);
  });

  it("reads each node's own state, usage and pod count", async () => {
    core.listNodes.mockResolvedValue({ nodes: [aNode("n1")] });
    core.nodeMetrics.mockResolvedValue({
      metrics: [{ name: "n1", cpuMillicores: 3520, memoryMiB: 11840 }],
    });
    open();

    await waitFor(() => expect(rowFor("n1")).toBeTruthy());
    const row = rowFor("n1");
    // 3520/4000 = 88%, 11840/16000 = 74%, two of the four pods are on n1.
    expect(within(row).getByRole("meter", { name: "n1 CPU" }).getAttribute("aria-valuetext")).toBe("88%");
    expect(within(row).getByRole("meter", { name: "n1 memory" }).getAttribute("aria-valuetext")).toBe("74%");
    expect(cells(row)[5]).toBe("2/50");
    expect(within(row).getByText("Ready")).toBeTruthy();
  });

  it("takes the state word and its tone from core, never from a table of its own", async () => {
    core.listNodes.mockResolvedValue({
      nodes: [aNode("n1", { unschedulable: true }), aNode("n2", { status: "NotReady" }), aNode("n3")],
    });
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());

    /** The same node as a fetched object, read by the function a detail pane reads. */
    const asObject = (ready: boolean, unschedulable: boolean): K8sObject =>
      ({
        apiVersion: "v1",
        kind: "Node",
        metadata: { name: "n" },
        spec: { unschedulable },
        status: { conditions: [{ type: "Ready", status: ready ? "True" : "False" }] },
      }) as unknown as K8sObject;

    const cordoned = resourceStatusLine("Node", asObject(true, true));
    const broken = resourceStatusLine("Node", asObject(false, false));
    const healthy = resourceStatusLine("Node", asObject(true, false));
    expect([cordoned, broken, healthy].every(Boolean)).toBe(true);

    // The word AND the tone, for all three states core distinguishes: a
    // cordoned node is warning, a NotReady one is danger, and a hand-paired
    // table that called them both "not Ready, so warn" would disagree here.
    const pill = (node: string, word: string) =>
      within(rowFor(node)).getByText(word).closest(".status");
    expect(pill("n1", cordoned!.status)?.getAttribute("data-kind")).toBe(cordoned!.health);
    expect(pill("n2", broken!.status)?.getAttribute("data-kind")).toBe(broken!.health);
    expect(pill("n3", healthy!.status)?.getAttribute("data-kind")).toBe(healthy!.health);
    expect(cordoned!.health).not.toBe(broken!.health);

    // Coloured and bold only where core called the state bad.
    expect(pill("n1", cordoned!.status)?.getAttribute("data-bad")).toBe("true");
    expect(pill("n2", broken!.status)?.getAttribute("data-bad")).toBe("true");
    expect(pill("n3", healthy!.status)?.getAttribute("data-bad")).toBeNull();
  });

  it("marks only the node that needs attention", async () => {
    core.listNodes.mockResolvedValue({ nodes: [aNode("n1"), aNode("n2", { status: "NotReady" })] });
    open();
    await waitFor(() => expect(rowFor("n2")).toBeTruthy());

    expect(within(rowFor("n2")).getByText("Needs attention")).toBeTruthy();
    expect(within(rowFor("n1")).queryByText("Needs attention")).toBeNull();
  });

  it("reads a node with no metric as no reading, not as an idle node", async () => {
    core.nodeMetrics.mockResolvedValue({ metrics: [METRICS[0]] });
    open();
    await waitFor(() => expect(rowFor("n2")).toBeTruthy());

    const row = rowFor("n2");
    expect(cells(row)[3]).toBe("No reading");
    expect(cells(row)[4]).toBe("No reading");
    // Not an empty meter, which reads as a measured zero.
    expect(within(row).queryByRole("meter")).toBeNull();
    // The node that did report still has both of its meters.
    expect(within(rowFor("n1")).getByRole("meter", { name: "n1 CPU" })).toBeTruthy();
  });

  it("reads a node that reported no allocatable pods as no reading, not as 2/0", async () => {
    core.listNodes.mockResolvedValue({ nodes: [aNode("n1", { allocatablePods: 0 })] });
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());

    expect(cells(rowFor("n1"))[5]).toBe("No reading");
    expect(within(rowFor("n1")).queryByText("2/0")).toBeNull();
  });

  it("passes a node over its limit through at its true percentage", async () => {
    core.listNodes.mockResolvedValue({ nodes: [aNode("n1")] });
    core.nodeMetrics.mockResolvedValue({
      metrics: [{ name: "n1", cpuMillicores: 5600, memoryMiB: 8000 }],
    });
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());

    const meter = within(rowFor("n1")).getByRole("meter", { name: "n1 CPU" });
    // The reading is honest; only the bar the meter draws is clamped.
    expect(meter.getAttribute("aria-valuetext")).toBe("140%");
    expect(meter.getAttribute("aria-valuenow")).toBe("100");
  });

  it("reads Pool from the node's own machine type", async () => {
    core.listNodes.mockResolvedValue({
      nodes: [aNode("eu-w4-c3-standard-a1", { instanceType: "c3-standard-8" })],
    });
    open();
    await waitFor(() => expect(rowFor("eu-w4-c3-standard-a1")).toBeTruthy());

    expect(cells(rowFor("eu-w4-c3-standard-a1"))[1]).toBe("c3-standard-8");
  });

  it("shows nothing in Pool rather than guessing one", async () => {
    // The node carries neither instance-type label — kind's nodes are
    // containers, not cloud machines — so it named no pool at all.
    core.listNodes.mockResolvedValue({
      nodes: [aNode("eu-w4-c3-standard-a1", { roles: "worker", instanceType: "" })],
    });
    open();
    await waitFor(() => expect(rowFor("eu-w4-c3-standard-a1")).toBeTruthy());

    const pool = cells(rowFor("eu-w4-c3-standard-a1"))[1];
    expect(pool).toBe("—");
    // Neither the naming convention in the node's name nor its roles.
    expect(pool).not.toContain("c3-standard");
    expect(pool).not.toContain("worker");
  });

  it("keeps the table when the metrics fail", async () => {
    core.nodeMetrics.mockResolvedValue({ error: "metrics API unavailable" });
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());
    expect(cells(rowFor("n1"))[3]).toBe("No reading");
  });

  it("empties the table and says why when the node list is refused", async () => {
    core.listNodes.mockResolvedValue({ error: "nodes is forbidden" });
    open();
    await waitFor(() => expect(screen.getByText(/nodes is forbidden/)).toBeTruthy());
    // One refused list is one empty section: the namespace count is untouched.
    expect(value("Namespaces")).toBe("4");
  });

  /**
   * The case the cache creates, and the one it must not get wrong: the reader
   * has rows on screen from a minute ago and the refresh behind them just
   * failed. Throwing the rows away would lose a real reading nobody asked to
   * lose; keeping them silently would present a stale cluster as a live one.
   */
  it("keeps the last good rows when a refresh is refused, and says they stopped refreshing", async () => {
    const { unmount } = open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());
    unmount();

    // Past the cache's TTL, so the next mount paints the cached rows AND
    // refreshes behind them — inside it there is no refresh to fail.
    const clock = vi.spyOn(Date, "now");
    clock.mockReturnValue(Date.now() + CACHE_TTL_MS + 1);
    core.listNodes.mockResolvedValue({ error: "nodes is forbidden" });
    open();
    // The rows are still the ones the cluster last gave, not an error state.
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());
    expect(value("Nodes")).toBe("3");
    // And the screen says so, once, in words the reader can act on — the
    // sentence about the rows is this screen's and does not change, the
    // reason under it is the classification, and what the cluster actually
    // said is folded away rather than dropped.
    const notice = await waitFor(() => screen.getByText(/no longer refreshing/i));
    expect(notice).toBeTruthy();
    expect(screen.getByText(/Check your RBAC roles/)).toBeTruthy();
    const folded = document.querySelector('[data-slot="raw"]') as HTMLDetailsElement;
    expect(folded.open).toBe(false);
    expect(folded.textContent).toContain("nodes is forbidden");
    clock.mockRestore();
  });

  /**
   * The rail is 286px wide, and every one of these rows used to print a
   * whole apiserver `Status { … }` into it.
   */
  it("says why a count is missing in a phrase, not in the apiserver's struct", async () => {
    // `listResource` is called with the kind LABEL, which is what the rail
    // prints — `COUNTED_BY_LISTING` is CronJob and Job.
    core.listResource.mockImplementation(async (_c: string, kind: string) =>
      kind === "Job" ? { error: API_401 } : { items: [] },
    );
    open();

    const row = await waitFor(() => screen.getByText(/Could not count Job/));
    // The copy, with the folded-away original taken out: a closed `details`
    // keeps its content in the DOM — that is what makes it a disclosure and
    // not a deletion — so reading straight through `textContent` would pass
    // whether or not the struct was being printed at the reader.
    const copy = row.cloneNode(true) as HTMLElement;
    copy.querySelector('[data-slot="raw"]')?.remove();
    expect(copy.textContent).toBe("Could not count Job: Not authorized");
    // Still reachable, still closed, still nowhere near an attribute.
    const folded = row.querySelector('[data-slot="raw"]') as HTMLDetailsElement;
    expect(folded.open).toBe(false);
    expect(folded.textContent).toContain("ListMeta");
    for (const node of Array.from(row.querySelectorAll("*"))) {
      for (const attribute of Array.from(node.attributes)) {
        expect(attribute.value).not.toContain("ListMeta");
      }
    }
  });

  it("says one outage once when it refused every kind at the same time", async () => {
    // Six kinds behind one expired credential is one problem. Repeating the
    // same sentence six times says it six times and explains it nowhere.
    core.listNodes.mockResolvedValue({ error: API_401 });
    core.podOverview.mockResolvedValue({ error: API_401 });
    core.listDeployments.mockResolvedValue({ error: API_401 });
    core.listStatefulSets.mockResolvedValue({ error: API_401 });
    core.listDaemonSets.mockResolvedValue({ error: API_401 });
    open();

    const card = await waitFor(() => screen.getByText(/Could not check every workload/));
    const detail = card.parentElement?.querySelector('[data-slot="detail"]');
    expect(detail?.textContent).toBe(
      "The cluster rejected your credentials. Your token or client certificate may have expired — refresh your kubeconfig credentials and try again.",
    );
  });
});

describe("Overview — the node actions", () => {
  it("does not drain a node until the drain is confirmed", async () => {
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());

    await userEvent.click(within(rowFor("n1")).getByRole("button", { name: "Drain" }));
    expect(core.drainNode).not.toHaveBeenCalled();

    const box = dialog();
    expect(box).toBeTruthy();
    // The confirm says what will happen, and shows the kubectl it stands for.
    expect(within(box!).getByText(/evicts every pod/i)).toBeTruthy();
    expect(
      within(box!).getByText(
        "kubectl drain n1 --ignore-daemonsets --delete-emptydir-data --force --context prod-eu",
      ),
    ).toBeTruthy();

    await userEvent.click(within(box!).getByRole("button", { name: "Drain" }));
    await waitFor(() => expect(core.drainNode).toHaveBeenCalledWith("prod-eu", "n1"));
    expect(core.drainNode).toHaveBeenCalledTimes(1);
  });

  it("cordons behind a confirm, and offers to uncordon a node already cordoned", async () => {
    core.listNodes.mockResolvedValue({ nodes: [aNode("n1"), aNode("n2", { unschedulable: true })] });
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());

    await userEvent.click(within(rowFor("n1")).getByRole("button", { name: "Cordon" }));
    expect(core.cordonNode).not.toHaveBeenCalled();
    expect(within(dialog()!).getByText("kubectl cordon n1 --context prod-eu")).toBeTruthy();
    await userEvent.click(within(dialog()!).getByRole("button", { name: "Cordon" }));
    await waitFor(() => expect(core.cordonNode).toHaveBeenCalledWith("prod-eu", "n1", true));

    // The cordoned node is offered the other direction, not the same one again.
    expect(within(rowFor("n2")).queryByRole("button", { name: "Cordon" })).toBeNull();
    await userEvent.click(within(rowFor("n2")).getByRole("button", { name: "Uncordon" }));
    await userEvent.click(within(dialog()!).getByRole("button", { name: "Uncordon" }));
    await waitFor(() => expect(core.cordonNode).toHaveBeenCalledWith("prod-eu", "n2", false));
  });

  it("keeps the dialog open with the reason when a drain fails", async () => {
    core.drainNode.mockResolvedValue({ error: "nodes/eviction is forbidden" });
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());

    await userEvent.click(within(rowFor("n1")).getByRole("button", { name: "Drain" }));
    await userEvent.click(within(dialog()!).getByRole("button", { name: "Drain" }));

    await waitFor(() => expect(within(dialog()!).getByText(/nodes\/eviction is forbidden/)).toBeTruthy());
    // Still open, so nothing reads as having succeeded.
    expect(dialog()).toBeTruthy();
  });
});

/* --------------------------------------------------------------- not ready */

/**
 * A cluster in several kinds of trouble at once.
 *
 * Deliberately not one shape of failure repeated: a fixture where every
 * unhealthy row is a Degraded workload cannot tell a right table of status
 * words from a wrong one, because both agree on the only case in it. This one
 * spans all three severities core can flag (danger, warning and the neutral
 * "we do not recognise this state"), four kinds, and — for each kind — one
 * subject core calls healthy or at rest, which must NOT appear.
 */
const SICK_DEPLOYMENTS: DeploymentSummary[] = [
  // 9 of 12 ready: core says Degraded, danger, flagged.
  aDeployment("zz-checkout-api", "9/12"),
  // Scaled to zero: neutral and NOT flagged. Amber-adjacent states like this
  // are why `flagged` is data rather than a reading of the tone.
  aDeployment("idle-batch", "0/0"),
];
const SICK_STATEFULSETS: StatefulSetSummary[] = [
  aStatefulSet("mm-payments-db", "1/3"),
  aStatefulSet("ok-cache", "3/3"),
];
const SICK_DAEMONSETS: DaemonSetSummary[] = [
  aDaemonSet("cc-log-agent", 2, 4),
  // Matches no node: "Not scheduled", and doing exactly what it was asked.
  aDaemonSet("nn-gpu-agent", 0, 0),
];
const SICK_PODS: PodSummary[] = [
  aPod("aa-worker-0", "n1", { namespace: "checkout", ready: "0/1", waitingReason: "CrashLoopBackOff" }),
  aPod("bb-queue-0", "n2", { namespace: "payments", phase: "Pending", ready: "0/1" }),
  // A phase core's table does not know: neutral, and still flagged — not
  // recognising a state is not the same as knowing it is fine.
  aPod("dd-mystery-0", "n3", { namespace: "search", phase: "Terminating", ready: "0/1" }),
  aPod("ok-web-0", "n1", { namespace: "checkout" }),
  aPod("done-backup-0", "n2", { namespace: "ops", phase: "Succeeded", ready: "0/1" }),
];

function sick() {
  core.listDeployments.mockResolvedValue({ deployments: SICK_DEPLOYMENTS });
  core.listStatefulSets.mockResolvedValue({ statefulsets: SICK_STATEFULSETS });
  core.listDaemonSets.mockResolvedValue({ daemonsets: SICK_DAEMONSETS });
  core.podOverview.mockResolvedValue({ pods: anOverview(SICK_PODS) });
}

function notReady(): HTMLElement {
  const heading = screen.getByRole("heading", { name: "Not ready" });
  const card = heading.closest("section");
  if (!card) throw new Error("no Not ready section on screen");
  return card as HTMLElement;
}

const notReadyNames = () =>
  Array.from(notReady().querySelectorAll(".status-row-name")).map((el) => el.textContent ?? "");

const notReadyRow = (name: string) =>
  Array.from(notReady().querySelectorAll<HTMLElement>(".status-row")).find(
    (row) => row.querySelector(".status-row-name")?.textContent === name,
  );

const factsOf = (row: HTMLElement) =>
  Array.from(row.querySelectorAll(".status-row-fact")).map((el) => el.textContent ?? "");

/** The same subject as a fetched object, read by the function a detail pane reads. */
const asObject = (kind: string, spec: unknown, status: unknown): K8sObject =>
  ({ apiVersion: "v1", kind, metadata: { name: "x" }, spec, status }) as unknown as K8sObject;

describe("Overview — the not-ready list", () => {
  it("puts the worst thing first, whatever kind it happens to be", async () => {
    sick();
    open();
    await waitFor(() => expect(notReadyNames()).toHaveLength(6));

    // Danger first (alphabetically within the band, so the order is stable
    // across three lists that settle in whatever order they settle), then the
    // warning, then the state core could not read.
    expect(notReadyNames()).toEqual([
      "aa-worker-0",
      "cc-log-agent",
      "mm-payments-db",
      "zz-checkout-api",
      "bb-queue-0",
      "dd-mystery-0",
    ]);

    // And the point of the section: this is NOT the order a list grouped by
    // kind would produce. A Pod leads three workloads, and two more Pods
    // follow them.
    expect(notReadyNames()).not.toEqual([
      "zz-checkout-api",
      "mm-payments-db",
      "cc-log-agent",
      "aa-worker-0",
      "bb-queue-0",
      "dd-mystery-0",
    ]);
  });

  it("lists what core calls unhealthy, and nothing else", async () => {
    sick();
    open();
    await waitFor(() => expect(notReadyNames()).toHaveLength(6));

    // Healthy, finished, scaled to zero, and matching no node: four subjects
    // core does not flag, in four different tones. A section that read
    // badness off the tone would pick up at least one of them.
    for (const quiet of ["idle-batch", "ok-cache", "nn-gpu-agent", "ok-web-0", "done-backup-0"]) {
      expect(notReadyRow(quiet)).toBeUndefined();
    }

    // The sharpest form of it: two subjects core gives the SAME tone, one
    // flagged and one not. `idle-batch` (scaled to zero) and `dd-mystery-0`
    // (a phase core does not recognise) are both neutral; only the second is
    // in the list, which no reading of the colour could have produced.
    expect(notReadyRow("dd-mystery-0")?.querySelector(".status")?.getAttribute("data-kind")).toBe(
      "neutral",
    );
  });

  it("takes every word and every tone from core, across all three severities", async () => {
    sick();
    open();
    await waitFor(() => expect(notReadyNames()).toHaveLength(6));

    const expected: [name: string, line: ReturnType<typeof resourceStatusLine>][] = [
      ["zz-checkout-api", resourceStatusLine("Deployment", asObject("Deployment", { replicas: 12 }, { readyReplicas: 9 }))],
      ["mm-payments-db", resourceStatusLine("StatefulSet", asObject("StatefulSet", { replicas: 3 }, { readyReplicas: 1 }))],
      ["cc-log-agent", resourceStatusLine("DaemonSet", asObject("DaemonSet", {}, { numberReady: 2, desiredNumberScheduled: 4 }))],
      [
        "aa-worker-0",
        resourceStatusLine(
          "Pod",
          asObject("Pod", {}, {
            phase: "Running",
            containerStatuses: [{ ready: false, state: { waiting: { reason: "CrashLoopBackOff" } } }],
          }),
        ),
      ],
      ["bb-queue-0", resourceStatusLine("Pod", asObject("Pod", {}, { phase: "Pending" }))],
      ["dd-mystery-0", resourceStatusLine("Pod", asObject("Pod", {}, { phase: "Terminating" }))],
    ];

    // Three distinct tones among them, so a single wrong tone cannot pass by
    // agreeing with the one case the fixture happens to contain.
    expect(new Set(expected.map(([, line]) => line!.health))).toEqual(
      new Set(["danger", "warning", "neutral"]),
    );

    for (const [name, line] of expected) {
      const row = notReadyRow(name);
      expect(row, `${name} should be in the not-ready list`).toBeTruthy();
      const pill = row!.querySelector(".status");
      expect(pill?.textContent, name).toBe(line!.status);
      expect(pill?.getAttribute("data-kind"), name).toBe(line!.health);
      // `flagged` is passed as data, not derived from the tone: every row
      // here is one core flagged, including the amber and the grey ones.
      expect(line!.flagged, name).toBe(true);
      // It reaches the pill as `tinted`. The kit colours the two tones the
      // design colours and leaves neutral plain — its own asymmetry, tested
      // in `StatusPill`; what matters here is that a flagged warning row is
      // coloured, which a caller passing `kind === "danger"` would have lost.
      if (line!.health !== "neutral") {
        expect(pill?.getAttribute("data-bad"), name).toBe("true");
      }
    }
  });

  it("holds a crash-looping pod in the list through the instant it is not backing off", async () => {
    // The flicker, at the screen it was observed on. `aa-worker-0` is the same
    // pod as in the fixture above with one field changed — the waiting reason
    // the kubelet stops reporting while the container is briefly up. Nothing
    // else about the pod moves: still 0/1 ready, still 41 restarts, still
    // phase "Running". Two of four consecutive screenshots of this list caught
    // it and two did not.
    const between = SICK_PODS.map((p) =>
      p.name === "aa-worker-0" ? { ...p, waitingReason: "", restarts: 41 } : p,
    );
    core.listDeployments.mockResolvedValue({ deployments: SICK_DEPLOYMENTS });
    core.listStatefulSets.mockResolvedValue({ statefulsets: SICK_STATEFULSETS });
    core.listDaemonSets.mockResolvedValue({ daemonsets: SICK_DAEMONSETS });
    core.podOverview.mockResolvedValue({ pods: anOverview(between) });
    open();

    // Same six rows, same order: the pod holds its place at the top of the
    // danger band rather than vanishing and re-appearing under the reader.
    await waitFor(() => expect(notReadyNames()).toHaveLength(6));
    expect(notReadyNames()).toEqual([
      "aa-worker-0",
      "cc-log-agent",
      "mm-payments-db",
      "zz-checkout-api",
      "bb-queue-0",
      "dd-mystery-0",
    ]);
    const row = notReadyRow("aa-worker-0")!;
    expect(row.querySelector(".status")?.getAttribute("data-kind")).toBe("danger");
    expect(row.querySelector(".status")?.textContent).toBe("NotReady");

    // And the two pods that must NOT be dragged in with it, both of which are
    // also short of ready: one finished, one never started.
    expect(notReadyRow("done-backup-0")).toBeUndefined();
    expect(notReadyRow("ok-web-0")).toBeUndefined();
  });

  it("counts that pod in the pods tile too, at the same severity", async () => {
    // The tile reads `podFlagged` over the same list, so a pod that fell out
    // of the rows fell out of the count and its colour with it — "4 not ready"
    // one second and "3 not ready" the next, on an unchanged cluster.
    const between = SICK_PODS.map((p) =>
      p.name === "aa-worker-0" ? { ...p, waitingReason: "", restarts: 41 } : p,
    );
    core.podOverview.mockResolvedValue({ pods: anOverview(between) });
    core.listDeployments.mockResolvedValue({ deployments: SICK_DEPLOYMENTS });
    core.listStatefulSets.mockResolvedValue({ statefulsets: SICK_STATEFULSETS });
    core.listDaemonSets.mockResolvedValue({ daemonsets: SICK_DAEMONSETS });
    open();

    await waitFor(() => expect(notReadyNames()).toHaveLength(6));
    // Three pods flagged — the crash-looper, the Pending one and the one in a
    // phase core cannot read — and NOT the Succeeded or the healthy one.
    expect(screen.getByText("3 not ready")).toBeTruthy();
  });

  it("names every trailing fact, so a reader hears more than 'checkout 9/12'", async () => {
    sick();
    open();
    await waitFor(() => expect(notReadyNames()).toHaveLength(6));

    // `StatusRow` takes `ReactNode`s and cannot know what a fact means; the
    // noun is the caller's job, and this is the assertion that keeps it done.
    for (const row of Array.from(notReady().querySelectorAll<HTMLElement>(".status-row"))) {
      const facts = factsOf(row);
      expect(facts).toHaveLength(2);
      for (const fact of facts) {
        expect(fact, `"${fact}" says nothing about what it is`).toMatch(/\b(namespace|ready)\b/);
      }
    }

    // And in the accessible name of the row itself, not merely somewhere in
    // the markup: the whole row is one button, and its name is its text.
    const row = within(notReady()).getByRole("button", { name: /zz-checkout-api/ });
    expect(row.textContent).toContain("Degraded");
    expect(row.textContent).toContain("namespace checkout");
    expect(row.textContent).toContain("9/12 ready");
    expect(within(notReady()).getByRole("button", { name: /bb-queue-0 namespace payments/ })).toBeTruthy();

    // The namespace comes before the ratio — the design's own column order.
    expect(factsOf(notReadyRow("cc-log-agent")!)).toEqual(["namespace kube-system", "2/4 ready"]);
  });

  it("opens the object's own detail when a row is activated", async () => {
    sick();
    open();
    await waitFor(() => expect(notReadyNames()).toHaveLength(6));

    await userEvent.click(within(notReady()).getByRole("button", { name: /zz-checkout-api/ }));
    expect(store.activeRoute()).toBe("/k/Deployment/checkout/zz-checkout-api");

    await userEvent.click(within(notReady()).getByRole("button", { name: /aa-worker-0/ }));
    expect(store.activeRoute()).toBe("/k/Pod/checkout/aa-worker-0");
  });

  it("says that nothing is unhealthy rather than leaving a blank", async () => {
    // The default fixture's one crash-looping pod, taken away.
    core.podOverview.mockResolvedValue({ pods: anOverview(PODS.filter((p) => !p.waitingReason)) });
    open();
    await waitFor(() => expect(within(notReady()).getByText("Nothing is unhealthy")).toBeTruthy());

    expect(notReadyNames()).toEqual([]);
    expect(within(notReady()).getByText(/prod-eu/)).toBeTruthy();
  });

  it("does not read a refused list as a healthy cluster", async () => {
    core.podOverview.mockResolvedValue({ pods: anOverview([]) });
    core.listDeployments.mockResolvedValue({ error: 'deployments is forbidden: User "dev" cannot list' });
    open();

    await waitFor(() => expect(within(notReady()).getByText(/forbidden/)).toBeTruthy());
    // "Nothing is unhealthy" would be a claim nobody checked.
    expect(within(notReady()).queryByText("Nothing is unhealthy")).toBeNull();
  });

  it("keeps the rows it does have when one kind is refused, and says which", async () => {
    core.podOverview.mockResolvedValue({ pods: anOverview(SICK_PODS) });
    core.listStatefulSets.mockResolvedValue({ error: 'statefulsets is forbidden: User "dev" cannot list' });
    open();

    await waitFor(() => expect(notReadyRow("aa-worker-0")).toBeTruthy());
    expect(notReadyNames()).toEqual(["aa-worker-0", "bb-queue-0", "dd-mystery-0"]);
    // The refusal is stated, not swallowed: the list is short for a reason.
    expect(within(notReady()).getByText(/statefulsets is forbidden/)).toBeTruthy();
    // And it stays one section's failure — the nodes table is untouched.
    expect(rowFor("n1")).toBeTruthy();
  });
});

/* --------------------------------------------------------------- the rail */

/** The `At a glance` rail, as the landmark its own head names. */
const rail = () => screen.getByRole("complementary", { name: "At a glance" });

/** One rail section, by the heading over it. */
function section(title: string): HTMLElement {
  const heading = within(rail()).getByRole("heading", { name: title });
  const found = heading.closest("section");
  if (!found) throw new Error(`no ${title} section in the rail`);
  return found as HTMLElement;
}

/** The control-plane facts as `label -> value`, in the order the rail draws them. */
function controlPlane(): Array<[string, string]> {
  return Array.from(section("Control plane").querySelectorAll(".kv")).map((kv) => [
    kv.querySelector(".kv-k")?.textContent ?? "",
    kv.querySelector(".kv-v")?.textContent ?? "",
  ]);
}

const factLabels = () => controlPlane().map(([k]) => k);
const factValue = (label: string) => controlPlane().find(([k]) => k === label)?.[1];

/** Seed the probe store the way the shell does, without a real connect call. */
async function probed(version: string | null) {
  await probeCluster(CTX, async () => ({ context: CTX.name, reachable: true, version }));
}

const SOME_FACTS: ClusterFacts = {
  context: "prod-eu",
  provider: "GKE",
  region: "europe-west4",
  metricsServer: { state: "present", version: "v1beta1" },
};

describe("Overview — the rail's control plane", () => {
  it("omits a fact the cluster did not report rather than calling it unknown", async () => {
    // The live case, not an edge one: no node on a kind cluster carries a
    // region label, and `clusterFacts` reports that as an empty string.
    core.clusterFacts.mockResolvedValue({ ...SOME_FACTS, provider: "", region: "" });
    open();
    await waitFor(() => expect(factValue("Context")).toBe("prod-eu"));

    expect(factLabels()).not.toContain("Provider");
    expect(factLabels()).not.toContain("Region");
    // And nothing standing in for them: "unknown" and an em dash both read as
    // answers, and the cluster gave none.
    expect(section("Control plane").textContent).not.toMatch(/unknown/i);
    expect(section("Control plane").textContent).not.toContain("—");
  });

  it("draws the facts the cluster did report, in the design's order", async () => {
    core.clusterFacts.mockResolvedValue(SOME_FACTS);
    await probed("v1.31.4");
    setLink(CTX.stableId, "connected");
    open();

    await waitFor(() => expect(factValue("Provider")).toBe("GKE"));
    expect(factLabels()).toEqual([
      "Version",
      "Provider",
      "Region",
      "Context",
      "Connection",
      "Metrics server",
    ]);
    expect(factValue("Version")).toBe("v1.31.4");
    expect(factValue("Region")).toBe("europe-west4");
  });

  it("takes the connection's word from the one table the status bar reads", async () => {
    // Four states, not one: a fixture with a single link state cannot tell a
    // right table from a wrong one. The words are `LINK_WORD`'s, shared with
    // the status bar, so the rail and the strip cannot disagree about the
    // same cluster.
    const cases: Array<[LinkState, string]> = [
      ["connected", "Connected"],
      ["connecting", "Connecting"],
      ["disconnected", "Disconnected"],
      ["error", "Unreachable"],
    ];
    for (const [state, word] of cases) {
      setLink(CTX.stableId, state);
      const { unmount } = open();
      await waitFor(() => expect(factValue("Connection")).toBe(word));
      unmount();
    }
  });

  it("says nothing about the connection until something has probed it", async () => {
    open();
    await waitFor(() => expect(factValue("Context")).toBe("prod-eu"));
    // No link state is not "Disconnected" — it is nobody having asked yet.
    expect(factLabels()).not.toContain("Connection");
    // Same for the version, which arrives with the probe.
    expect(factLabels()).not.toContain("Version");
  });

  it("reads the metrics server's API group version, not a component version", async () => {
    core.clusterFacts.mockResolvedValue(SOME_FACTS);
    open();

    await waitFor(() => expect(factValue("Metrics server")).toBe("v1beta1"));
    // The mock's `v0.7.2 · reporting` is two claims this screen cannot make:
    // the component version needs an RBAC-sensitive read of the deployment's
    // image, and an aggregated APIService stays in discovery while its backing
    // deployment is down — so "present" is not "answering".
    expect(section("Control plane").textContent).not.toContain("reporting");
    expect(section("Control plane").textContent).not.toContain("v0.7.2");
  });

  it("states a missing metrics server once, in the rail, and nowhere else", async () => {
    core.clusterFacts.mockResolvedValue({
      ...SOME_FACTS,
      metricsServer: { state: "absent", version: "" },
    });
    // The same cluster's metrics call fails too, which is what a missing
    // metrics-server does — every tile and column reads "No reading".
    core.nodeMetrics.mockResolvedValue({ error: "the server could not find the requested resource" });
    open();

    await waitFor(() => expect(factValue("Metrics server")).toContain("Not installed"));
    expect(value("CPU")).toBe("No reading");

    // Once. Five tiles and two columns each announcing it would have said it
    // seven times and explained it nowhere.
    expect(screen.getAllByText(/not installed/i)).toHaveLength(1);
  });

  it("does not call the metrics server absent when only the request failed", async () => {
    // Discovery says the group is there; the reading failed anyway — a
    // throttled apiserver, a transient refusal. Two different questions with
    // the same visible answer, and the rail must read the discovery one.
    core.clusterFacts.mockResolvedValue(SOME_FACTS);
    core.nodeMetrics.mockResolvedValue({ error: "the server is currently unable to handle the request" });
    open();

    await waitFor(() => expect(factValue("Metrics server")).toBe("v1beta1"));
    expect(screen.queryByText(/not installed/i)).toBeNull();
    // The tiles still say there is no reading — they just do not say why.
    expect(value("CPU")).toBe("No reading");
  });

  it("omits the metrics-server row when nobody could ask", async () => {
    // `unknown` is not `absent`: an unreachable cluster has not told us
    // metrics-server is missing, and drawing that as an absence would be the
    // rail inventing the one fact it exists to report.
    core.clusterFacts.mockResolvedValue({ ...SOME_FACTS, metricsServer: { state: "unknown", version: "" } });
    open();

    await waitFor(() => expect(factValue("Provider")).toBe("GKE"));
    expect(factLabels()).not.toContain("Metrics server");
  });
});

describe("Overview — the rail's object counts", () => {
  const countRow = (label: string) =>
    within(section("Objects by kind")).getByRole("button", { name: new RegExp(`^${label}`) });

  it("names the kind, singular, and not the sidebar's list label", async () => {
    open();
    await waitFor(() => expect(countRow("Deployment")).toBeTruthy());

    // §7 writes `Deployment 25`: the row names the KIND its number counts,
    // where the sidebar's plural names a list you are about to open. Core
    // holds both tables (`K8S_KIND` and `RESOURCE_LABELS`) side by side, so
    // this is a choice between them and not a table of the screen's own.
    const labels = Array.from(
      section("Objects by kind").querySelectorAll<HTMLElement>(".ns-row"),
    ).map((row) => row.firstElementChild?.textContent ?? "");
    expect(labels).toEqual(["Deployment", "Pod", "StatefulSet", "DaemonSet", "CronJob", "Job"]);
    for (const [slug, label] of Object.entries(K8S_KIND)) {
      if (labels.includes(label)) expect(RESOURCE_LABELS[slug as ResourceKind]).not.toBe(label);
    }
  });

  it("counts a kind off the list the screen already loaded, without asking twice", async () => {
    core.listDeployments.mockResolvedValue({
      deployments: [aDeployment("a", "1/1"), aDeployment("b", "1/1")],
    });
    core.listStatefulSets.mockResolvedValue({ statefulsets: [aStatefulSet("c", "1/1")] });
    core.listResource.mockResolvedValue({ items: [{ name: "x" }] });
    open();

    await waitFor(() => expect(countRow("Deployment").textContent).toContain("2"));
    expect(countRow("StatefulSet").textContent).toContain("1");
    // Four pods in the fixture — a count the backend made, not the length of
    // a list this screen fetched.
    expect(countRow("Pod").textContent).toContain("4");

    // The collapse: Deployments, StatefulSets, DaemonSets and Pods are all
    // already on screen, so the generic list is called only for the two kinds
    // nothing else loads.
    expect(core.listDeployments).toHaveBeenCalledTimes(1);
    expect(core.podOverview).toHaveBeenCalledTimes(1);
    expect(core.listResource).toHaveBeenCalledTimes(2);
    expect(core.listResource.mock.calls.map((c: unknown[]) => c[1])).toEqual(["CronJob", "Job"]);
  });

  it("opens that kind's list when a row is activated", async () => {
    open();
    await waitFor(() => expect(countRow("Deployment")).toBeTruthy());

    await userEvent.click(countRow("Deployment"));
    expect(store.activeRoute()).toBe("/k/deployments");

    await userEvent.click(countRow("CronJob"));
    expect(store.activeRoute()).toBe("/k/cronjobs");
  });

  it("shows no number for a kind it could not count, and says why on that row", async () => {
    core.listResource.mockImplementation((_context: string, kind: string) =>
      Promise.resolve(
        kind === "Job"
          ? { error: 'jobs is forbidden: User "dev" cannot list resource "jobs"' }
          : { items: [{ name: "x" }, { name: "y" }] },
      ),
    );
    open();

    await waitFor(() => expect(countRow("CronJob").textContent).toContain("2"));
    // Zero is a number a reader will believe. A refusal is not a count.
    expect(countRow("Job").textContent).not.toContain("0");
    expect(countRow("Job").textContent).toContain("—");

    // The reason belongs to the row that could not answer — the treatment
    // `Fleet` gives an unreachable cluster — not to a paragraph under the
    // section naming the kinds a second time. It sits beside its own row and
    // carries what the cluster actually said.
    const reason = within(section("Objects by kind")).getByText(/could not count job/i);
    expect(reason.textContent).toContain("jobs is forbidden");
    expect(reason.previousElementSibling).toBe(countRow("Job"));
    // And it is the only one: the kinds that answered say nothing.
    expect(within(section("Objects by kind")).getAllByText(/could not count/i)).toHaveLength(1);
  });

  it("does not count a refused workload list as a cluster with none of that kind", async () => {
    core.listDeployments.mockResolvedValue({ error: "deployments is forbidden" });
    open();

    await waitFor(() => expect(countRow("Pod").textContent).toContain("4"));
    expect(countRow("Deployment").textContent).not.toContain("0");
    expect(countRow("Deployment").textContent).toContain("—");
  });
});

describe("Overview — the rail's incidents and fleet", () => {
  it("names the incidents section rather than leaving a hole", async () => {
    open();
    const incidents = await waitFor(() => section("Open incidents"));

    // A silent absence reads as a bug. This one says what it is: no
    // Kubernetes API returns an incident's title, severity or trend, and
    // Incidents is scheduled as its own feature.
    expect(incidents.textContent).toMatch(/incident/i);
    expect(incidents.textContent).toMatch(/srelens/);
    expect(incidents.textContent).not.toMatch(/SEV-\d/);
  });

  it("counts this cluster's pods in the fleet, whatever else is in the workspace", async () => {
    core.podCount.mockResolvedValue({ counts: { running: 30, total: 33 } });
    open();

    const fleet = await waitFor(() => section("Fleet"));
    await waitFor(() => expect(fleet.textContent).toContain("30/33 running"));
    expect(within(fleet).getByText("prod-eu")).toBeTruthy();
    expect(core.podCount).toHaveBeenCalledWith("prod-eu");
  });

  it("keeps the rest of the screen when the fleet cannot answer", async () => {
    core.podCount.mockResolvedValue({ error: "pod count timed out" });
    open();

    await waitFor(() => expect(section("Fleet").textContent).toContain("Unreachable"));
    // Fleet is a courtesy; the overview is about this cluster.
    expect(value("Nodes")).toBe("3");
    expect(rowFor("n1")).toBeTruthy();
  });
});

/* ------------------------------------------------------- the screen's shape */

/** The left-hand column of bands — the sibling run `.section + .section` rules. */
const leftColumn = () =>
  (document.querySelector('[data-slot="rail-main"] .scroll') ?? undefined) as HTMLElement | undefined;

const railBody = () =>
  (document.querySelector('[data-slot="rail-body"]') ?? undefined) as HTMLElement | undefined;

const bandTitles = (host: HTMLElement) =>
  Array.from(host.querySelectorAll(":scope > section.section > h3")).map((h) => h.textContent ?? "");

describe("Overview — a flat surface, not a stack of cards", () => {
  it("draws no card anywhere on the screen", async () => {
    sick();
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());

    // The user settled this for the resource detail pane and it is the same
    // decision: "flat sections win — the mock is a flat run of sections
    // separated by hairline rules… Drop the cards." A card has a border, a
    // lifted surface and a ruled head, and three of them read as three boxes
    // on a page rather than one continuous surface.
    expect(document.querySelectorAll(".card")).toHaveLength(0);
    expect(document.querySelectorAll(".card-head")).toHaveLength(0);
  });

  it("makes the bands direct siblings of one box, in both columns", async () => {
    sick();
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());

    // `.section + .section` is what draws the hairlines. A wrapper per band —
    // one div each to hang a class on — silently removes every rule while
    // leaving the screen looking almost right, so this is pinned rather than
    // trusted. The kit's own suite pins the CSS; this pins the adjacency.
    expect(bandTitles(leftColumn()!)).toEqual(["Capacity", "Nodes", "Not ready"]);
    expect(bandTitles(railBody()!)).toEqual([
      "Control plane",
      "Objects by kind",
      "Open incidents",
      "Fleet",
    ]);
    for (const host of [leftColumn()!, railBody()!]) {
      expect([...host.children].every((el) => el.matches("section.section"))).toBe(true);
    }
  });

  it("puts no gap and no padding between the bands", async () => {
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());

    // The design's bands sit directly against each other, divided by one
    // hairline. A `gap` puts daylight either side of that rule and a `p-3`
    // insets every band inside a second margin the rail beside it has not
    // got — which is what made the build read as three boxes.
    const column = leftColumn()!.className;
    expect(column).not.toMatch(/\bgap-\d/);
    expect(column).not.toMatch(/\bp-\d/);
  });

  it("heads every band in the design's small caps", async () => {
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());

    // §C.3's structural signpost, the same recipe the pane heads wear. The
    // uppercase is CSS, so the outline and a screen reader still hear the
    // words themselves.
    for (const host of [leftColumn()!, railBody()!]) {
      for (const head of Array.from(host.querySelectorAll(":scope > section.section > h3"))) {
        expect(head.className, head.textContent ?? "").toContain("subhead-caps");
      }
    }
  });

  it("runs the tables and the row lists to both edges, and keeps the fact list inset", async () => {
    sick();
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());

    const unpadded = (title: string) =>
      screen
        .getAllByRole("heading", { name: title })[0]
        .closest("section")
        ?.getAttribute("data-padded");

    // §D names it: `padded: false` for tables and list rows. A table inside an
    // inset draws its hairlines short of the rules dividing the bands around
    // it.
    expect(unpadded("Capacity")).toBe("false");
    expect(unpadded("Nodes")).toBe("false");
    expect(unpadded("Not ready")).toBe("false");
    expect(unpadded("Objects by kind")).toBe("false");
    // The key/value bands keep theirs: a `.kv` row is not a full-width row.
    expect(unpadded("Control plane")).toBeNull();
    expect(unpadded("Fleet")).toBeNull();
  });

  it("heads the left column with the cluster and its server version", async () => {
    await probed("v1.31.4");
    open();

    // §7's `prod-eu · v1.31.4`, level with the rail's own head. The version is
    // the probe's — the same reading the rail's `Version` row takes, not a
    // second call.
    await waitFor(() =>
      expect(document.querySelector('[data-slot="main-head"]')?.textContent).toBe(
        "prod-eu · v1.31.4",
      ),
    );
  });

  it("heads it with the name alone until something has probed the cluster", async () => {
    open();
    // Not "prod-eu · " — a separator with nothing after it reads as a fact
    // that failed to load rather than as one nobody has asked for yet.
    await waitFor(() =>
      expect(document.querySelector('[data-slot="main-head"]')?.textContent).toBe("prod-eu"),
    );
  });
});

describe("Overview — a context name too long for the toolbar", () => {
  const LONG = "m01-1786968575165/kubernetes-admin@cluster.local";
  const crumb = () => document.querySelector(".crumb")?.textContent ?? "";

  beforeEach(() => {
    const ctx = { ...CTX, name: LONG };
    resetContexts();
    setContexts([ctx]);
    store.setState(defaultState([ctx]));
  });

  it("keeps both ends of the name, and cuts the middle", async () => {
    open();
    await waitFor(() => expect(crumb()).toBeTruthy());

    // Cutting the TAIL is the one form that must not be used: every kubeadm
    // context in a fleet ends `@cluster.local`, so two clusters truncated that
    // way are indistinguishable. Cutting the head loses the other half. The
    // middle goes, and both ends that tell one cluster from another survive.
    expect(crumb().length).toBeLessThan(LONG.length);
    expect(crumb()).toContain("…");
    expect(crumb().startsWith("m01-")).toBe(true);
    expect(crumb().endsWith("cluster.local")).toBe(true);
    expect(crumb()).not.toContain("kubernetes-admin");
  });

  it("tells two clusters apart that differ only at one end", async () => {
    open();
    const first = crumb();
    cleanup();

    const other = { ...CTX, name: "m02-1786968575165/kubernetes-admin@cluster.local" };
    resetContexts();
    setContexts([other]);
    store.setState(defaultState([other]));
    open();
    await waitFor(() => expect(crumb()).toBeTruthy());
    expect(crumb()).not.toBe(first);
  });

  it("leaves a name that already fits exactly as it is", async () => {
    resetContexts();
    setContexts([CTX]);
    store.setState(defaultState([CTX]));
    open();
    await waitFor(() => expect(crumb()).toBeTruthy());
    expect(crumb()).toBe("prod-eu");
    expect(crumb()).not.toContain("…");
  });

  it("still carries the whole name in the rail, where there is room for it", async () => {
    open();
    // Nothing is hidden by the cut: the header is a label and the rail is the
    // record.
    await waitFor(() => expect(factValue("Context")).toBe(LONG));
  });
});

/* -------------------------------------------------- the nodes band's shape */

/** The node names the table drew, in the order it drew them — the name alone,
 *  without the flagged node's `sr-only` "Needs attention". */
const nodeNames = () =>
  Array.from(document.querySelectorAll("tbody tr.tbl-row td:first-child span.truncate")).map(
    (el) => el.textContent?.trim() ?? "",
  );

describe("Overview — a nodes band that stays a summary", () => {
  /** A fleet big enough that drawing all of it is the bug. */
  const many = (count: number) =>
    Array.from({ length: count }, (_, i) => aNode(`node-${String(i).padStart(3, "0")}`));

  it("caps the rows, and says what it is not showing", async () => {
    core.listNodes.mockResolvedValue({ nodes: many(113) });
    core.nodeMetrics.mockResolvedValue({ metrics: [] });
    open();
    await waitFor(() => expect(nodeNames().length).toBeGreaterThan(0));

    // 113 rows pushed `Not ready` — the band that says what is actually wrong
    // — entirely off the screen on a real cluster. `/k/nodes` is the screen
    // for the whole inventory; this one is a summary of it.
    expect(nodeNames()).toHaveLength(10);
    // Silently truncating is the part that would be dishonest.
    expect(screen.getByText(/Showing 10 of 113 nodes/)).toBeTruthy();
    // And the whole list is one click away.
    await userEvent.click(screen.getByRole("button", { name: /Showing 10 of 113 nodes/ }));
    expect(store.activeRoute()).toBe("/k/nodes");
  });

  it("says nothing about a cap when there is nothing to hide", async () => {
    open();
    await waitFor(() => expect(nodeNames()).toHaveLength(3));
    expect(screen.queryByText(/Showing \d+ of/)).toBeNull();
  });

  it("picks the rows worth looking at, not the first ten alphabetically", async () => {
    const rest = many(20);
    core.listNodes.mockResolvedValue({
      nodes: [
        ...rest,
        // Two nodes core has an opinion about, named so that alphabetical
        // order buries both of them at the very end of 22.
        aNode("zz-broken", { status: "NotReady" }),
        aNode("zy-cordoned", { unschedulable: true }),
      ],
    });
    // And one core is content with, running hot.
    core.nodeMetrics.mockResolvedValue({
      metrics: [{ name: "node-019", cpuMillicores: 3760, memoryMiB: 2000 }],
    });
    open();
    await waitFor(() => expect(nodeNames()).toHaveLength(10));

    // core's verdict first — a NotReady node above a cordoned one, which is
    // `nodeVerdict`'s own ordering and not a reading of this file's — then
    // the hottest of the ones core is content with.
    expect(nodeNames().slice(0, 3)).toEqual(["zz-broken", "zy-cordoned", "node-019"]);
    // The point of it: this is NOT what the list arrived in.
    expect(nodeNames()[0]).not.toBe("node-000");
  });

  it("counts the whole cluster in the capacity tile, not the ten it drew", async () => {
    core.listNodes.mockResolvedValue({
      nodes: [...many(112), aNode("zz-broken", { status: "NotReady" })],
    });
    core.nodeMetrics.mockResolvedValue({ metrics: [] });
    open();

    // The cap is the TABLE's. A tile that counted the rows on screen would
    // report a 113-node cluster as a ten-node one.
    await waitFor(() => expect(value("Nodes")).toBe("113"));
    expect(caption("Nodes")).toBe("1 not ready");
  });

  it("gives the meters room to show their tone", async () => {
    core.listNodes.mockResolvedValue({ nodes: [aNode("n1")] });
    core.nodeMetrics.mockResolvedValue({
      metrics: [{ name: "n1", cpuMillicores: 3520, memoryMiB: 6560 }],
    });
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());

    // `Meter`'s bar is `w-full`, which has no intrinsic width, so a table cell
    // sized by its content collapsed the column onto the percentage beside it
    // and drew a 40px stub. At that size the tone — 88% red against 41% green
    // — is the one thing the column exists to show and the one thing nobody
    // can see. The width goes on the CONTENT, where auto layout can act on it.
    for (const label of ["n1 CPU", "n1 memory"]) {
      const meter = within(rowFor("n1")).getByRole("meter", { name: label });
      const cell = meter.closest("td");
      expect(cell?.firstElementChild?.className, label).toContain("min-w-[10rem]");
    }
  });

  it("asks for that room before the metrics have landed", async () => {
    // The node list answers well before `nodeMetrics` does, and `Table`
    // measures the natural column widths on the FIRST render that has rows,
    // then pins them. A width that arrived with the meters arrived after the
    // columns had been fixed at the width of the words "No reading", and the
    // meters drew straight across the columns beside them.
    core.listNodes.mockResolvedValue({ nodes: [aNode("n1")] });
    core.nodeMetrics.mockResolvedValue({ error: "metrics API unavailable" });
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());

    const row = rowFor("n1");
    expect(cells(row)[3]).toBe("No reading");
    expect(within(row).queryByRole("meter")).toBeNull();
    for (const index of [3, 4]) {
      const cell = row.querySelectorAll("td")[index];
      expect(cell.firstElementChild?.className, String(index)).toContain("min-w-[10rem]");
    }
  });

  it("marks the two node actions with the design's glyphs", async () => {
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());

    // A crossed circle on Cordon, a wave on Drain. From `lib/icons` — the
    // app's vocabulary — because the kit takes no icon-set dependency.
    for (const label of ["Cordon", "Drain"]) {
      const button = within(rowFor("n1")).getByRole("button", { name: label });
      expect(button.querySelector("svg"), label).not.toBeNull();
    }
    // The words still say what the buttons do; the glyph is a second channel.
    expect(within(rowFor("n1")).getByRole("button", { name: "Drain" }).textContent).toBe("Drain");
  });
});

/**
 * The failure this whole change exists to end.
 *
 * On the user's 113-node cluster the screen asked for every pod in the
 * cluster — 5 416 of them, 114 MB — for three figures, and the request did not
 * come back inside its budget. The Pods tile, every node's Pods column and the
 * rail's Pods count all read "No reading", which was HONEST and useless.
 *
 * Nothing downstream of the backend changed its mind about honesty: every
 * assertion below that says "no reading" still has to hold. What changed is
 * that on a cluster this size there is now a reading to show.
 */
describe("Overview — a cluster too big to list", () => {
  /** 113 nodes and 5 416 pods, counted the way the backend counts them. */
  const BIG_NODES = Array.from({ length: 113 }, (_, i) => aNode(`worker-${String(i).padStart(3, "0")}`));
  const BIG_COUNTS: PodOverview = {
    total: 5416,
    byNode: BIG_NODES.map((n, i) => ({ node: n.name, pods: 40 + (i % 11) })),
    unsettled: [
      aPod("crash-0", "worker-000", { namespace: "checkout", ready: "0/1", waitingReason: "CrashLoopBackOff" }),
      aPod("done-0", "worker-001", { namespace: "ops", phase: "Succeeded", ready: "0/1" }),
    ],
    truncated: false,
  };

  function big(over: Partial<PodOverview> = {}) {
    core.listNodes.mockResolvedValue({ nodes: BIG_NODES });
    core.nodeMetrics.mockResolvedValue({ metrics: [] });
    core.podOverview.mockResolvedValue({ pods: { ...BIG_COUNTS, ...over } });
  }

  it("answers the Pods tile from a count, with no pod list anywhere", async () => {
    big();
    open();

    await waitFor(() => expect(value("Pods")).toBe("5416"));
    // One pod is crash-looping; the finished Job pod is not a problem, and
    // core is what says which is which.
    expect(caption("Pods")).toBe("1 not ready");
    expect(tone("Pods")).toBe("sev");
    expect(core.podOverview).toHaveBeenCalledWith("prod-eu");
  });

  it("fills every node's Pods column from the backend's grouping", async () => {
    big();
    open();

    await waitFor(() => expect(rowFor("worker-000")).toBeTruthy());
    // 40 of the node's own allocatable 50 — the ratio the design draws, on a
    // cluster whose pod list could never have been fetched to build it.
    expect(cells(rowFor("worker-000"))[5]).toBe("40/50");
    expect(screen.queryAllByText("No reading").length).toBeGreaterThan(0);
    expect(cells(rowFor("worker-000"))[5]).not.toBe("No reading");
  });

  it("counts the Pods row in the rail from the same count", async () => {
    big();
    open();

    const row = await waitFor(() => {
      const found = within(section("Objects by kind"))
        .getAllByRole("button")
        .find((b) => b.textContent?.startsWith(K8S_KIND.pods));
      if (!found) throw new Error("no Pod row");
      return found;
    });
    expect(row.textContent).toContain("5416");
    expect(within(section("Objects by kind")).queryByText(/could not count Pod/i)).toBeNull();
  });

  it("still reads a cluster that did not answer as no reading, never as no pods", async () => {
    core.listNodes.mockResolvedValue({ nodes: BIG_NODES });
    core.podOverview.mockResolvedValue({ error: "pod overview timed out" });
    open();

    await waitFor(() => expect(value("Pods")).toBe("No reading"));
    // The lie the whole null-is-not-zero chain exists to prevent.
    expect(value("Pods")).not.toBe("0");
    await waitFor(() => expect(rowFor("worker-000")).toBeTruthy());
    expect(cells(rowFor("worker-000"))[5]).toBe("No reading");
  });

  it("says the not-ready list is short rather than presenting a cap as the whole truth", async () => {
    big({ truncated: true });
    open();

    await waitFor(() => expect(within(notReady()).getByText(/more pods need a look/i)).toBeTruthy());
    // And the tile stops claiming an exact figure it cannot have.
    expect(caption("Pods")).toBe("at least 1 not ready");
  });

  it("never says every pod is ready off a list that was cut short", async () => {
    big({ unsettled: [], truncated: true });
    open();

    await waitFor(() => expect(value("Pods")).toBe("5416"));
    // "all ready" is a claim about every pod, and a capped list has not seen
    // every pod. Silence is what it actually knows.
    expect(caption("Pods")).toBeNull();
    expect(within(notReady()).queryByText("Nothing is unhealthy")).toBeNull();
  });
});

describe("Overview — coming back to the tab", () => {
  it("paints the cluster from the last reading instead of loading it again", async () => {
    const { unmount } = open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());
    const calls = {
      nodes: core.listNodes.mock.calls.length,
      pods: core.podOverview.mock.calls.length,
      facts: core.clusterFacts.mock.calls.length,
    };
    unmount();

    open();
    // Already there, on the first render, with no spinner in between.
    expect(rowFor("n1")).toBeTruthy();
    expect(value("Pods")).toBe("4");
    expect(screen.queryByText("Loading nodes")).toBeNull();
    expect(core.listNodes).toHaveBeenCalledTimes(calls.nodes);
    expect(core.podOverview).toHaveBeenCalledTimes(calls.pods);
    expect(core.clusterFacts).toHaveBeenCalledTimes(calls.facts);
  });

  it("never shows one cluster's cached figures for another", async () => {
    core.listNodes.mockImplementation((context: string) =>
      Promise.resolve({ nodes: [aNode(`${context}-1`)] }),
    );
    const other: ClusterContext = { ...CTX, name: "staging-eu", stableId: "staging" };
    setContexts([CTX, other]);
    store.setState(defaultState([CTX, other]));

    const { unmount } = open();
    await waitFor(() => expect(rowFor("prod-eu-1")).toBeTruthy());
    unmount();

    store.setState({ ...defaultState([other]), });
    store.openTab(ROUTE);
    render(
      <ConsoleProvider>
        <Overview route={ROUTE} />
      </ConsoleProvider>,
    );
    await waitFor(() => expect(rowFor("staging-eu-1")).toBeTruthy());
    expect(screen.queryByText("prod-eu-1")).toBeNull();
  });
});

/**
 * **A node action runs on the cluster it was picked on.**
 *
 * Until #357 a dialog was window-modal: its overlay covered the cluster rail,
 * so the reader could not switch clusters while one was open, and reading the
 * live `context` prop inside `confirm` was accidentally right. It is not any
 * more. `setActiveCluster` switches the active cluster in place, globally;
 * `Nodes` is mounted unconditionally and `{pending && <NodeConfirm/>}` sits
 * outside the loading and error branches, so nothing here remounts and nothing
 * resets `pending`.
 *
 * Reproduced by execution against this screen before this suite existed: Drain
 * opened on `prod-eu`'s `n1`, `setActiveCluster("stage")`, the dialog still on
 * screen, Drain confirmed — and `drainNode` was called with `[ 'stage-eu',
 * 'n1' ]`. Every pod on staging's node of that name, evicted from a dialog
 * that named production's.
 *
 * The rule is `lib/clusterMoved`'s: pin at pick, run against the pinned
 * cluster, state the divergence, and re-arm the confirmation — this confirm's
 * whole input is one click, so asking again costs the reader a tick.
 */
describe("Overview — the cluster a node action was picked on", () => {
  const MOVED = "stage-eu";
  const STAGE: ClusterContext = { ...CTX, name: MOVED, stableId: "stage", isCurrent: false };

  const box = () => dialog() as HTMLElement;
  const tick = (verb: string) =>
    screen.getByRole("checkbox", { name: `Yes, still ${verb} on prod-eu.` }) as HTMLInputElement;
  const REFUSAL = `This runs on prod-eu, not ${MOVED}. Confirm the cluster above, or cancel.`;

  beforeEach(() => {
    setContexts([CTX, STAGE]);
    store.setState(defaultState([CTX, STAGE]));
    store.setActiveCluster(CTX.stableId);
  });

  /** Pick a node action on `prod-eu`, then move the rail to `stage-eu`. */
  async function pickThenMove(label: string, node = "n1") {
    open();
    await waitFor(() => expect(rowFor(node)).toBeTruthy());
    await userEvent.click(within(rowFor(node)).getByRole("button", { name: label }));
    expect(box()).toBeTruthy();

    store.setActiveCluster(STAGE.stableId);
    await waitFor(() => expect(store.currentWorkspace().activeCluster).toBe(STAGE.stableId));
    // The rail is reachable behind a scoped dialog now, so the question is
    // still on screen — which is the whole premise this fix answers.
    expect(box()).toBeTruthy();
  }

  it("drains the node on the cluster it was picked on, not the one the rail moved to", async () => {
    await pickThenMove("Drain");
    await userEvent.click(tick("drain"));
    await userEvent.click(within(box()).getByRole("button", { name: "Drain" }));

    await waitFor(() => expect(core.drainNode).toHaveBeenCalledTimes(1));
    expect(core.drainNode).toHaveBeenCalledWith("prod-eu", "n1");
  });

  it("refuses the drain until the reader confirms which cluster, and says why", async () => {
    await pickThenMove("Drain");
    await userEvent.click(within(box()).getByRole("button", { name: "Drain" }));

    expect(core.drainNode).not.toHaveBeenCalled();
    expect(within(box()).getByText(REFUSAL)).toBeTruthy();
    expect(box()).toBeTruthy();

    await userEvent.click(tick("drain"));
    await userEvent.click(within(box()).getByRole("button", { name: "Drain" }));
    await waitFor(() => expect(core.drainNode).toHaveBeenCalledWith("prod-eu", "n1"));
  });

  it("cordons the node on the cluster it was picked on", async () => {
    // The other half of the same `confirm`: a fix applied to the drain branch
    // alone would still retarget this one.
    await pickThenMove("Cordon");
    await userEvent.click(tick("cordon"));
    await userEvent.click(within(box()).getByRole("button", { name: "Cordon" }));
    await waitFor(() => expect(core.cordonNode).toHaveBeenCalledTimes(1));
    expect(core.cordonNode).toHaveBeenCalledWith("prod-eu", "n1", true);
  });

  it("says the rail moved, first, above the node's own name, and keeps the kubectl honest", async () => {
    await pickThenMove("Drain");
    const alert = screen.getByText(`This still runs against prod-eu, not ${MOVED}`).closest("[data-tone]");
    expect(alert).toBeTruthy();
    expect(alert?.getAttribute("data-tone")).toBe("warn");
    expect(alert?.getAttribute("role")).toBe("status");
    expect(alert?.textContent?.replace(/\s+/g, " ")).toContain(
      `the same names on ${MOVED} are different objects`,
    );

    const message = alert?.parentElement;
    expect(message?.firstElementChild).toBe(alert);
    expect(message?.textContent).toContain("evicts every pod");

    // The command names the cluster the write will actually reach, so the
    // dialog and the operation cannot agree on the wrong one.
    expect(
      within(box()).getByText(
        "kubectl drain n1 --ignore-daemonsets --delete-emptydir-data --force --context prod-eu",
      ),
    ).toBeTruthy();
    expect(within(box()).queryByText(new RegExp(`--context ${MOVED}`))).toBeNull();
  });

  it("says nothing, and asks nothing, while the rail has not moved", async () => {
    open();
    await waitFor(() => expect(rowFor("n1")).toBeTruthy());
    await userEvent.click(within(rowFor("n1")).getByRole("button", { name: "Drain" }));

    expect(within(box()).queryByText(/This still runs against/)).toBeNull();
    expect(screen.queryByRole("checkbox")).toBeNull();
    // One click, exactly as before this fix existed.
    await userEvent.click(within(box()).getByRole("button", { name: "Drain" }));
    await waitFor(() => expect(core.drainNode).toHaveBeenCalledWith("prod-eu", "n1"));
  });
});
