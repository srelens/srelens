import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClusterContext, DaemonSetSummary, DeploymentSummary, EventSummary, PodSummary, StatefulSetSummary } from "@srelens/core";

const core = vi.hoisted(() => ({
  podOverview: vi.fn(),
  listDeployments: vi.fn(),
  listStatefulSets: vi.fn(),
  listDaemonSets: vi.fn(),
  listEvents: vi.fn(),
}));
vi.mock("@srelens/core", async (original) => ({ ...(await original<typeof import("@srelens/core")>()), ...core }));

import {
  ATTENTION_CONCURRENCY,
  ATTENTION_REFRESH_MS,
  attentionQuestion,
  readClusterAttention,
  useAttention,
  type AttentionItem,
} from "./attention";
import { clearResourceCache } from "./cachedResource";
import { useOverviewPods, useOverviewWorkloads } from "./overview";

const NOW = Date.parse("2026-10-09T12:00:00Z");
const ctx = (stableId: string, name = stableId): ClusterContext => ({
  stableId, key: stableId, name, cluster: name, server: `https://${stableId}.example`, isCurrent: false,
  sourceFile: "/mock/config", authKind: "client certificate",
});
const pod = (name: string, status: string, phase = "Running"): PodSummary =>
  ({ name, namespace: "checkout", phase, status, ready: "0/1", restarts: 4, node: "n1", age: "1h", image: "web:1" }) as PodSummary;
const deployment = (name: string, ready: string): DeploymentSummary =>
  ({ name, namespace: "checkout", ready, upToDate: 1, available: 0, age: "1d" }) as DeploymentSummary;
const warning = (object: string, reason: string, minutesAgo: number, type = "Warning"): EventSummary => ({
  name: `checkout/${object}.${reason}`, namespace: "checkout", type, reason, object, message: `${reason} happened`,
  created: new Date(NOW - minutesAgo * 60_000).toISOString(), age: `${minutesAgo}m`, count: 1,
});

/** Every read answers, with nothing in it, unless a test says otherwise. */
function answerEmpty() {
  core.podOverview.mockResolvedValue({ pods: { total: 0, byNode: [], unsettled: [], truncated: false } });
  core.listDeployments.mockResolvedValue({ deployments: [] });
  core.listStatefulSets.mockResolvedValue({ statefulsets: [] });
  core.listDaemonSets.mockResolvedValue({ daemonsets: [] });
  core.listEvents.mockResolvedValue({ events: [] });
}

beforeEach(() => {
  for (const fn of Object.values(core)) fn.mockReset();
  answerEmpty();
  clearResourceCache();
});

const brief = (items: AttentionItem[]) => items.map((i) => `${i.kind} ${i.namespace}/${i.name}: ${i.problem}`);

describe("readClusterAttention", () => {
  it("names crash-looping and image-pull-failing pods, and leaves a pending one out", async () => {
    core.podOverview.mockResolvedValue({ pods: { total: 3, byNode: [], truncated: false, unsettled: [
      pod("web-7d4b", "CrashLoopBackOff"),
      pod("api-1", "ImagePullBackOff", "Pending"),
      pod("api-2", "ErrImagePull", "Pending"),
      pod("batch-1", "ContainerCreating", "Pending"),
    ] } });
    const scan = await readClusterAttention(ctx("prod-id", "prod"), NOW);
    expect(brief(scan.items)).toEqual([
      "Pod checkout/web-7d4b: CrashLoopBackOff",
      "Pod checkout/api-1: ImagePullBackOff",
      "Pod checkout/api-2: ErrImagePull",
    ]);
    expect(scan.items[0]).toMatchObject({ clusterId: "prod-id", cluster: "prod", cause: "crash" });
    expect(scan.items[1].cause).toBe("image");
    expect(scan.failures).toEqual([]);
  });

  it("names workloads with unavailable replicas, and leaves healthy and scaled-to-zero ones out", async () => {
    core.listDeployments.mockResolvedValue({ deployments: [deployment("web", "1/3"), deployment("ok", "2/2"), deployment("off", "0/0")] });
    core.listStatefulSets.mockResolvedValue({ statefulsets: [{ name: "db", namespace: "data", ready: "0/1" } as StatefulSetSummary] });
    core.listDaemonSets.mockResolvedValue({ daemonsets: [{ name: "agent", namespace: "kube-system", ready: 2, desired: 3 } as DaemonSetSummary] });
    const scan = await readClusterAttention(ctx("prod-id", "prod"), NOW);
    expect(brief(scan.items)).toEqual([
      "Deployment checkout/web: 1/3 ready",
      "StatefulSet data/db: 0/1 ready",
      "DaemonSet kube-system/agent: 2/3 ready",
    ]);
    expect(scan.items.every((i) => i.cause === "replicas")).toBe(true);
  });

  it("names warning events from the last hour, one per object, and nothing older or normal", async () => {
    core.podOverview.mockResolvedValue({ pods: { total: 1, byNode: [], truncated: false, unsettled: [pod("web-7d4b", "CrashLoopBackOff")] } });
    core.listEvents.mockResolvedValue({ events: [
      warning("Pod/web-7d4b", "BackOff", 2),
      warning("Service/front", "FailedToUpdateEndpoint", 10),
      warning("Service/front", "FailedToUpdateEndpoint", 12),
      warning("Node/n1", "Rebooted", 90),
      warning("Pod/web-7d4b", "Pulled", 1, "Normal"),
    ] });
    const scan = await readClusterAttention(ctx("prod-id", "prod"), NOW);
    expect(core.listEvents).toHaveBeenCalledWith("prod", null);
    expect(brief(scan.items)).toEqual([
      "Pod checkout/web-7d4b: CrashLoopBackOff",
      "Service checkout/front: FailedToUpdateEndpoint",
    ]);
    expect(scan.items[1].cause).toBe("warning");
  });

  it("keeps a failed read as a failure beside what did answer, never as nothing to report", async () => {
    core.listDeployments.mockResolvedValue({ deployments: [deployment("web", "1/3")] });
    core.listEvents.mockResolvedValue({ error: "events is forbidden: User \"dana\" cannot list resource \"events\"" });
    core.podOverview.mockResolvedValue({ error: "connection refused" });
    const scan = await readClusterAttention(ctx("prod-id", "prod"), NOW);
    expect(brief(scan.items)).toEqual(["Deployment checkout/web: 1/3 ready"]);
    expect(scan.failures).toEqual(["connection refused", "events is forbidden: User \"dana\" cannot list resource \"events\""]);
  });

  it("says when the pod read stopped short of the whole list", async () => {
    core.podOverview.mockResolvedValue({ pods: { total: 900, byNode: [], unsettled: [], truncated: true } });
    expect((await readClusterAttention(ctx("prod-id"), NOW)).truncated).toBe(true);
  });

  it("says when the event read stopped short, since recent warnings may be past the cap", async () => {
    core.listEvents.mockResolvedValue({ events: [], truncated: true });
    const scan = await readClusterAttention(ctx("prod-id"), NOW);
    expect(scan.eventsTruncated).toBe(true);
    expect(scan.truncated).toBe(false);
  });
});

describe("readClusterAttention beside an open overview of the same cluster", () => {
  const crashing = { pods: { total: 1, byNode: [], truncated: false, unsettled: [pod("web-7d4b", "CrashLoopBackOff")] } };
  const shared = [core.podOverview, core.listDeployments, core.listStatefulSets, core.listDaemonSets];
  const openOverview = () => renderHook(() => ({ pods: useOverviewPods("prod"), workloads: useOverviewWorkloads("prod") }));

  it("takes the pods and workloads the overview read moments ago, rather than reading them again", async () => {
    core.podOverview.mockResolvedValue(crashing);
    core.listDeployments.mockResolvedValue({ deployments: [deployment("api", "1/3")] });
    const overview = openOverview();
    await waitFor(() => expect([overview.result.current.pods.status, overview.result.current.workloads.status]).toEqual(["ready", "ready"]));
    const scan = await readClusterAttention(ctx("prod-id", "prod"), NOW);
    expect(brief(scan.items)).toEqual(["Pod checkout/web-7d4b: CrashLoopBackOff", "Deployment checkout/api: 1/3 ready"]);
    for (const read of shared) expect(read).toHaveBeenCalledTimes(1);
    // The overview reads no events, so there is nothing to share there.
    expect(core.listEvents).toHaveBeenCalledTimes(1);
  });

  it("waits for the overview's pod and workload reads still out, rather than racing them", async () => {
    let answer!: () => void;
    core.podOverview.mockImplementation(() => new Promise((done) => { answer = () => done(crashing); }));
    openOverview();
    const scanning = readClusterAttention(ctx("prod-id", "prod"), NOW);
    const scan = await act(async () => { answer(); return scanning; });
    expect(brief(scan.items)).toEqual(["Pod checkout/web-7d4b: CrashLoopBackOff"]);
    for (const read of shared) expect(read).toHaveBeenCalledTimes(1);
  });

  it("keeps a refused pod read as the failure it was, beside the workloads that answered", async () => {
    core.podOverview.mockResolvedValue({ error: "connection refused" });
    core.listDeployments.mockResolvedValue({ deployments: [deployment("web", "1/3")] });
    core.listStatefulSets.mockResolvedValue({ error: "statefulsets is forbidden" });
    const scan = await readClusterAttention(ctx("prod-id", "prod"), NOW);
    expect(brief(scan.items)).toEqual(["Deployment checkout/web: 1/3 ready"]);
    expect(scan.failures).toEqual(["connection refused", "statefulsets is forbidden"]);
  });
});

describe("attentionQuestion", () => {
  const item = (over: Partial<AttentionItem>): AttentionItem => ({
    clusterId: "prod-id", cluster: "prod", kind: "Pod", namespace: "checkout", name: "web-7d4b", problem: "CrashLoopBackOff", cause: "crash", ...over,
  });
  it.each([
    [item({}), "Why is checkout/web-7d4b crash-looping on prod?"],
    [item({ problem: "ImagePullBackOff", cause: "image" }), "Why can't checkout/web-7d4b pull its image on prod?"],
    [item({ kind: "Deployment", name: "web", problem: "1/3 ready", cause: "replicas" }), "Why does Deployment checkout/web have unavailable replicas on prod?"],
    [item({ kind: "Service", name: "front", problem: "FailedToUpdateEndpoint", cause: "warning" }), "What is behind the FailedToUpdateEndpoint warning on Service checkout/front on prod?"],
    [item({ kind: "Node", namespace: "", name: "n1", problem: "Rebooted", cause: "warning" }), "What is behind the Rebooted warning on Node n1 on prod?"],
  ])("asks about %#", (subject, question) => {
    expect(attentionQuestion(subject)).toBe(question);
  });
});

describe("useAttention", () => {
  let hidden = false;
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "setTimeout", "clearTimeout", "Date"] });
    vi.setSystemTime(NOW);
    hidden = false;
    vi.spyOn(document, "visibilityState", "get").mockImplementation(() => (hidden ? "hidden" : "visible"));
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  /** Clusters whose pod read has been asked for, in order. */
  const asked = () => core.podOverview.mock.calls.map(([context]) => context as string);

  it("reads at most a few clusters at once, and starts the next as one finishes", async () => {
    const release = new Map<string, () => void>();
    core.podOverview.mockImplementation((context: string) => new Promise((done) => {
      release.set(context, () => done({ pods: { total: 0, byNode: [], unsettled: [], truncated: false } }));
    }));
    const targets = ["a", "b", "c", "d", "e"].map((id) => ctx(id));
    const { result } = renderHook(() => useAttention(targets, false));
    await act(async () => { await Promise.resolve(); });
    expect(asked()).toEqual(["a", "b", "c"].slice(0, ATTENTION_CONCURRENCY));
    await act(async () => { release.get("a")!(); await vi.advanceTimersByTimeAsync(0); });
    expect(asked()).toEqual(["a", "b", "c", "d"]);
    expect(Object.keys(result.current)).toEqual(["a"]);
  });

  /** Pod reads that answer only when the test says so, by cluster. */
  function heldReads() {
    const release = new Map<string, () => void>();
    core.podOverview.mockImplementation((context: string) => new Promise((done) => {
      release.set(context, () => done({ pods: { total: 0, byNode: [], unsettled: [], truncated: false } }));
    }));
    return async (...contexts: string[]) => {
      await act(async () => { for (const c of contexts) release.get(c)!(); await vi.advanceTimersByTimeAsync(0); });
    };
  }
  const five = ["a", "b", "c", "d", "e"].map((id) => ctx(id));

  it("starts no more reads once it is paused in the middle of a batch", async () => {
    const answer = heldReads();
    const view = renderHook(({ paused }) => useAttention(five, paused), { initialProps: { paused: false } });
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    expect(asked()).toEqual(["a", "b", "c"]);
    view.rerender({ paused: true });
    await answer("a", "b", "c");
    expect(asked()).toEqual(["a", "b", "c"]);
  });

  it("starts no more reads once the window is hidden in the middle of a batch, and finishes when shown", async () => {
    const answer = heldReads();
    renderHook(() => useAttention(five, false));
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    hidden = true;
    await answer("a", "b", "c");
    expect(asked()).toEqual(["a", "b", "c"]);
    hidden = false;
    await act(async () => { document.dispatchEvent(new Event("visibilitychange")); await vi.advanceTimersByTimeAsync(0); });
    expect(asked().length).toBeGreaterThan(3);
  });

  it("reads the clusters a hidden window skipped as soon as the batch ends, when shown again meanwhile", async () => {
    // Shown again while reads from the cut batch were still out: the window's
    // own catch-up found the batch running and stood down, so without a
    // restart when it ends the skipped clusters waited for the next tick.
    const answer = heldReads();
    renderHook(() => useAttention(five, false));
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    hidden = true;
    await answer("a");
    expect(asked()).toEqual(["a", "b", "c"]);
    hidden = false;
    await act(async () => { document.dispatchEvent(new Event("visibilitychange")); await vi.advanceTimersByTimeAsync(0); });
    await answer("b", "c");
    // The restart is a whole new pass. The first three were read moments ago,
    // so the result cache answers them, and the two the hidden window skipped
    // are asked for straight away, in order.
    expect(asked()).toEqual(["a", "b", "c", "d", "e"]);
  });

  it("does not start a second batch beside one still reading when the clusters change", async () => {
    const answer = heldReads();
    const view = renderHook(({ targets }) => useAttention(targets, false), { initialProps: { targets: five.slice(0, 3) } });
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    expect(asked()).toEqual(["a", "b", "c"]);
    view.rerender({ targets: five });
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    expect(asked()).toEqual(["a", "b", "c"]);
    // A second batch started too soon would share the first one's pod reads
    // through the cache, so the pod count alone cannot see it. Events are not
    // cached: only the first batch's three may be out.
    expect(core.listEvents).toHaveBeenCalledTimes(3);
    // The new batch starts once those land. The three just read come from the
    // result cache, so only the two new clusters are asked for.
    await answer("a", "b", "c");
    expect(asked()).toEqual(["a", "b", "c", "d", "e"]);
  });

  it("refreshes on its interval, not before", async () => {
    renderHook(() => useAttention([ctx("a")], false));
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    expect(asked()).toEqual(["a"]);
    await act(async () => { await vi.advanceTimersByTimeAsync(ATTENTION_REFRESH_MS - 1_000); });
    expect(asked()).toEqual(["a"]);
    await act(async () => { await vi.advanceTimersByTimeAsync(1_000); });
    expect(asked()).toEqual(["a", "a"]);
  });

  it("reads nothing while the window is hidden, and catches up when it is shown again", async () => {
    renderHook(() => useAttention([ctx("a")], false));
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    hidden = true;
    await act(async () => { await vi.advanceTimersByTimeAsync(ATTENTION_REFRESH_MS * 3); });
    expect(asked()).toEqual(["a"]);
    hidden = false;
    await act(async () => { document.dispatchEvent(new Event("visibilitychange")); await vi.advanceTimersByTimeAsync(0); });
    expect(asked()).toEqual(["a", "a"]);
  });

  it("reads nothing at all while paused, and forgets clusters it no longer reads", async () => {
    const view = renderHook(({ paused, targets }) => useAttention(targets, paused), {
      initialProps: { paused: true, targets: [ctx("a")] },
    });
    await act(async () => { await vi.advanceTimersByTimeAsync(ATTENTION_REFRESH_MS * 2); });
    expect(asked()).toEqual([]);
    view.rerender({ paused: false, targets: [ctx("a")] });
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    expect(Object.keys(view.result.current)).toEqual(["a"]);
    view.rerender({ paused: false, targets: [] });
    expect(view.result.current).toEqual({});
  });
});
