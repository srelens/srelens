import { act, render, renderHook, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { settingsStorage, type ActiveForward, type ClusterContext, type CountKindRef, type CrdRef } from "@srelens/core";

const { countResources, resolveNamespaceCollections, forwards } = vi.hoisted(() => ({
  countResources: vi.fn(),
  resolveNamespaceCollections: vi.fn(),
  forwards: { list: [] as unknown[] },
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  countResources,
  resolveNamespaceCollections,
  getForwards: () => forwards.list,
  subscribeForwards: () => () => {},
}));

import { CACHE_TTL_MS, clearResourceCache } from "./cachedResource";
import { COLLECTIONS_KEY, __resetCollectionsForTests, saveCollection } from "./collections";
import { __resetCollectionScopeForTests, noteNamespaceListing } from "./collectionScope";
import { groupNumber } from "./numbers";
import { useSidebarCounts } from "./sidebarCounts";
import { makeTab } from "./tabs";
import { setState, setTabNamespaceCollections } from "./tabsStore";
import { setNamespaces } from "./workspace";

const ctx = (name: string): ClusterContext => ({
  name,
  stableId: `id:${name}`,
  key: `id:${name}`,
  cluster: name,
  server: "https://example",
  isCurrent: false,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
});
const PROD = ctx("prod-eu");
const OTHER = ctx("staging");

const CERTS: CrdRef = {
  name: "certificates.cert-manager.io",
  group: "cert-manager.io",
  version: "v1",
  kind: "Certificate",
  plural: "certificates",
  namespaced: true,
};

/** The backend answering every kind, `table[kind]` where given and 1 otherwise. */
function answering(table: Record<string, number | null> = {}) {
  countResources.mockImplementation(async (_context: string, kinds: CountKindRef[]) => ({
    counts: kinds.map((k) => ({ kind: k.kind, group: k.group ?? "", count: k.kind in table ? table[k.kind] : 1 })),
  }));
}

const asked = (call = 0) => countResources.mock.calls[call] as [string, CountKindRef[], string[]];
const kindsAsked = (call = 0) => asked(call)[1].map((k) => k.kind);

interface Args {
  ctx: ClusterContext | null;
  paused: boolean;
  expanded: readonly string[];
  crds: CrdRef[];
}
function counts(initial: Partial<Args> = {}) {
  const initialProps: Args = { ctx: PROD, paused: false, expanded: [], crds: [], ...initial };
  return renderHook((args: Args) => useSidebarCounts(args), { initialProps });
}

/** A tab scoped to collection `name`, resolved (when `listing`) to `namespaces`. */
function scopeTo(name: string, resolved: string[] | "waiting", resources?: { kinds: string[]; nameRegex: string }) {
  saveCollection({ name, type: "namespace", patterns: ["*"], ...(resources ? { resources } : {}) });
  resolveNamespaceCollections.mockResolvedValue({ namespaces: resolved === "waiting" ? [] : resolved, collections: [], partial: false });
  setTabNamespaceCollections("t", PROD.stableId, [name]);
  if (resolved !== "waiting") noteNamespaceListing(PROD.stableId, [{ name: "a" }, { name: "b" }], true);
}

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  clearResourceCache();
  settingsStorage.removeItem(COLLECTIONS_KEY);
  __resetCollectionsForTests();
  __resetCollectionScopeForTests();
  countResources.mockReset();
  resolveNamespaceCollections.mockReset();
  forwards.list = [];
  answering();
  setState({
    workspaces: [
      {
        id: "w",
        name: "W",
        clusters: [PROD.stableId, OTHER.stableId],
        activeCluster: PROD.stableId,
        tabs: [{ ...makeTab("/k/pods"), id: "t" }],
        activeId: "t",
        closed: [],
      },
    ],
    currentId: "w",
  });
});

afterEach(() => {
  vi.useRealTimers();
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
});

describe("useSidebarCounts", () => {
  it("asks nothing while every group is closed", async () => {
    const { result } = counts();
    await act(() => vi.advanceTimersByTimeAsync(10));
    expect(countResources).not.toHaveBeenCalled();
    expect(result.current.size).toBe(0);
  });

  it("opening Workloads counts exactly its seven kinds in the tab's namespaces", async () => {
    setNamespaces(PROD.stableId, ["shop"]);
    answering({ Pod: 38 });
    const { result } = counts({ expanded: ["workloads"] });
    await waitFor(() => expect(result.current.get("kind:pods")).toBe("38"));
    expect(countResources).toHaveBeenCalledTimes(1);
    expect(asked()[0]).toBe("prod-eu");
    expect([...kindsAsked()].sort()).toEqual(["CronJob", "DaemonSet", "Deployment", "Job", "Pod", "ReplicaSet", "StatefulSet"]);
    expect(asked()[2]).toEqual(["shop"]);
  });

  it("a paused cluster asks nothing", async () => {
    const { result } = counts({ expanded: ["workloads"], paused: true });
    await act(() => vi.advanceTimersByTimeAsync(10));
    expect(countResources).not.toHaveBeenCalled();
    expect(result.current.size).toBe(0);
  });

  /** A refused list and an empty cluster are the same picture and opposite facts. */
  it("a refused kind has no entry, never 0", async () => {
    answering({ Pod: null, Deployment: 3 });
    const { result } = counts({ expanded: ["workloads"] });
    await waitFor(() => expect(result.current.get("kind:deployments")).toBe("3"));
    expect(result.current.has("kind:pods")).toBe(false);
  });

  it("big numbers use the app's grouping", async () => {
    answering({ Pod: 5416 });
    const { result } = counts({ expanded: ["workloads"] });
    await waitFor(() => expect(result.current.get("kind:pods")).toBe(groupNumber(5416)));
    expect(result.current.get("kind:pods")).not.toBe("5416");
  });

  it("a failed refresh keeps the numbers, dimmed, with when they were counted", async () => {
    answering({ Pod: 38 });
    const { result } = counts({ expanded: ["workloads"] });
    await waitFor(() => expect(result.current.get("kind:pods")).toBe("38"));
    countResources.mockResolvedValue({ error: "the cluster did not answer" });
    await act(() => vi.advanceTimersByTimeAsync(CACHE_TTL_MS));
    await waitFor(() => expect(countResources).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(typeof result.current.get("kind:pods")).toBe("object"));
    render(<>{result.current.get("kind:pods")}</>);
    const shown = screen.getByTitle(/^Counted at .+; could not refresh$/);
    expect(shown.textContent).toMatch(/^38/);
    // Opacity and a hover title reach no screen reader: the row's name has to
    // say it too, or a stale number is read out as current.
    expect(shown.querySelector(".sr-only")?.textContent).toMatch(/not refreshed/);
  });

  it("a selection change recounts, a reordered one does not", async () => {
    setNamespaces(PROD.stableId, ["a", "b"]);
    const { result } = counts({ expanded: ["workloads"] });
    await waitFor(() => expect(result.current.size).toBe(7));
    expect(countResources).toHaveBeenCalledTimes(1);
    act(() => setNamespaces(PROD.stableId, ["b", "a"]));
    await act(() => vi.advanceTimersByTimeAsync(10));
    expect(countResources).toHaveBeenCalledTimes(1);
    act(() => setNamespaces(PROD.stableId, ["a"]));
    await waitFor(() => expect(countResources).toHaveBeenCalledTimes(2));
    expect(asked(1)[2]).toEqual(["a"]);
  });

  it("a ready collection that selects nothing is 0 for namespaced kinds, with no call for them", async () => {
    scopeTo("none", []);
    const { result } = counts({ expanded: ["workloads", "storage"] });
    await waitFor(() => expect(result.current.get("kind:pods")).toBe("0"));
    expect(result.current.get("kind:persistentvolumeclaims")).toBe("0");
    await waitFor(() => expect(countResources).toHaveBeenCalled());
    expect(kindsAsked()).toEqual(["PersistentVolume", "StorageClass"]);
  });

  it("a collection still resolving gives namespaced kinds no number", async () => {
    scopeTo("prod", "waiting");
    const { result } = counts({ expanded: ["workloads", "storage"] });
    await waitFor(() => expect(countResources).toHaveBeenCalled());
    expect(kindsAsked()).toEqual(["PersistentVolume", "StorageClass"]);
    await waitFor(() => expect(result.current.get("kind:storageclasses")).toBe("1"));
    expect(result.current.has("kind:pods")).toBe(false);
  });

  it("a collection that narrows a kind by name leaves that kind out", async () => {
    scopeTo("web", ["a"], { kinds: ["Pod"], nameRegex: "^web-" });
    const { result } = counts({ expanded: ["workloads"] });
    await waitFor(() => expect(countResources).toHaveBeenCalled());
    expect(kindsAsked()).not.toContain("Pod");
    expect(asked()[2]).toEqual(["a"]);
    await waitFor(() => expect(result.current.get("kind:deployments")).toBe("1"));
    expect(result.current.has("kind:pods")).toBe(false);
  });

  it("CRDs are sent with all four hints, only when both folds are open", async () => {
    const { rerender } = counts({ expanded: ["crds"], crds: [CERTS] });
    await act(() => vi.advanceTimersByTimeAsync(10));
    expect(countResources).not.toHaveBeenCalled();
    rerender({ ctx: PROD, paused: false, expanded: ["crds", "crdgroup:cert-manager.io"], crds: [CERTS] });
    await waitFor(() => expect(countResources).toHaveBeenCalledTimes(1));
    expect(asked()[1]).toEqual([{ kind: "Certificate", group: "cert-manager.io", version: "v1", plural: "certificates", namespaced: true }]);
  });

  it("no refresh while hidden", async () => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    const { result } = counts({ expanded: ["workloads"] });
    await waitFor(() => expect(result.current.size).toBe(7));
    await act(() => vi.advanceTimersByTimeAsync(3 * CACHE_TTL_MS));
    expect(countResources).toHaveBeenCalledTimes(1);
  });

  it("switching cluster never shows the old cluster's numbers", async () => {
    answering({ Pod: 38 });
    const { result, rerender } = counts({ expanded: ["workloads"] });
    await waitFor(() => expect(result.current.get("kind:pods")).toBe("38"));
    countResources.mockReturnValue(new Promise(() => {}));
    rerender({ ctx: OTHER, paused: false, expanded: ["workloads"], crds: [] });
    expect(result.current.has("kind:pods")).toBe(false);
  });

  it("Port Forwards counts this cluster's open tunnels", async () => {
    forwards.list = [
      { context: "prod-eu", status: "active" },
      { context: "prod-eu", status: "reconnecting" },
      { context: "prod-eu", status: "failed" },
      { context: "staging", status: "active" },
    ] as ActiveForward[];
    const { result, rerender } = counts({ expanded: ["cluster"] });
    await waitFor(() => expect(result.current.get("route:/forwards")).toBe("2"));
    rerender({ ctx: PROD, paused: false, expanded: [], crds: [] });
    expect(result.current.has("route:/forwards")).toBe(false);
  });
});
