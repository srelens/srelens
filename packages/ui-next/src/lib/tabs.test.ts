// jsdom, not node: `./tabs` imports `./routes`, whose screen table reaches
// `@xterm/addon-fit` — a UMD bundle that reads `self` as it evaluates.
import { describe, it, expect, vi, afterEach } from "vitest";
import type { ClusterContext } from "@srelens/core";
import { CLOSED_CAP, defaultState, makeTab, newId, reconcile, type TabsState, type Workspace } from "./tabs";

const ctx = (stableId: string, name = stableId): ClusterContext => ({
  name, stableId, cluster: name, server: `https://${name}`, isCurrent: false,
});

const ws = (over: Partial<Workspace> = {}): Workspace => ({
  id: "w1", name: "Default", clusters: ["a", "b"],
  tabs: [makeTab("/"), makeTab("/k/pods")], activeId: "", closed: [], ...over,
});

describe("newId", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("does not repeat", () => {
    expect(new Set(Array.from({ length: 200 }, newId)).size).toBe(200);
  });

  it("still hands back unique ids where randomUUID does not exist", () => {
    // `crypto.randomUUID` is [SecureContext]-only: over plain http it is
    // undefined, and an unguarded call threw while `@srelens/ui-next` was
    // still evaluating — the whole design failed to boot, with no way back.
    vi.stubGlobal("crypto", {});
    const ids = Array.from({ length: 200 }, newId);
    expect(ids.every((id) => typeof id === "string" && id.length > 0)).toBe(true);
    expect(new Set(ids).size).toBe(200);
  });
});

describe("makeTab", () => {
  it("describes the route and carries the cluster name into the sub", () => {
    const tab = makeTab("/k/pods", { clusterName: "staging" });
    expect(tab).toMatchObject({ route: "/k/pods", title: "Pods", sub: "staging", kind: "workloads" });
    expect(tab.id).toBeTruthy();
  });

  it("marks a preview when asked and leaves it off otherwise", () => {
    expect(makeTab("/logs", { preview: true }).preview).toBe(true);
    expect(makeTab("/logs").preview).toBeUndefined();
  });

  it("pins the home route", () => {
    expect(makeTab("/").pinned).toBe(true);
  });

  it("has no view until the user sorts or filters it", () => {
    expect(makeTab("/k/pods").view).toBeUndefined();
  });
});

describe("defaultState", () => {
  it("puts every known cluster into one workspace called Default", () => {
    const state = defaultState([ctx("a"), ctx("b")]);
    expect(state.workspaces).toHaveLength(1);
    expect(state.workspaces[0].name).toBe("Default");
    expect(state.workspaces[0].clusters).toEqual(["a", "b"]);
    expect(state.currentId).toBe(state.workspaces[0].id);
  });

  it("keys clusters by stableId, not display name", () => {
    // #265: the display name gains a `file/` prefix the moment another
    // kubeconfig declares the same context name; the id does not.
    const state = defaultState([ctx("id-1", "file/prod")]);
    expect(state.workspaces[0].clusters).toEqual(["id-1"]);
  });

  it("opens one pinned home tab so the strip is never empty", () => {
    const [w] = defaultState([ctx("a")]).workspaces;
    expect(w.tabs).toHaveLength(1);
    expect(w.tabs[0]).toMatchObject({ route: "/", pinned: true });
    expect(w.activeId).toBe(w.tabs[0].id);
  });

  it("points at the first cluster", () => {
    const s = defaultState([ctx("b"), ctx("a")]);
    expect(s.workspaces[0].activeCluster).toBe("b");
  });

  it("still makes a workspace when there are no clusters at all", () => {
    const state = defaultState([]);
    expect(state.workspaces).toHaveLength(1);
    expect(state.workspaces[0].clusters).toEqual([]);
  });
});

describe("reconcile", () => {
  it("drops cluster ids that no longer exist", () => {
    const state: TabsState = { workspaces: [ws({ clusters: ["a", "gone", "b"] })], currentId: "w1" };
    expect(reconcile(state, [ctx("a"), ctx("b")]).workspaces[0].clusters).toEqual(["a", "b"]);
  });

  it("keeps a workspace whose clusters all vanished, but empty", () => {
    // The user named it; losing the name because a kubeconfig moved would be
    // worse than an empty rail.
    const state: TabsState = { workspaces: [ws({ clusters: ["gone"] })], currentId: "w1" };
    const out = reconcile(state, []);
    expect(out.workspaces).toHaveLength(1);
    expect(out.workspaces[0].clusters).toEqual([]);
  });

  it("points activeId at a real tab when it names a closed one", () => {
    const w = ws({ activeId: "nope" });
    const out = reconcile({ workspaces: [w], currentId: "w1" }, [ctx("a")]);
    expect(out.workspaces[0].activeId).toBe(w.tabs[0].id);
  });

  it("gives a workspace with no tabs a fresh pinned home tab", () => {
    const out = reconcile({ workspaces: [ws({ tabs: [], activeId: "" })], currentId: "w1" }, []);
    const [w] = out.workspaces;
    expect(w.tabs).toHaveLength(1);
    expect(w.tabs[0].route).toBe("/");
    expect(w.activeId).toBe(w.tabs[0].id);
  });

  it("points currentId at a real workspace when it names none", () => {
    const out = reconcile({ workspaces: [ws()], currentId: "deleted" }, []);
    expect(out.currentId).toBe("w1");
  });

  it("guarantees at least one workspace", () => {
    const out = reconcile({ workspaces: [], currentId: "" }, [ctx("a")]);
    expect(out.workspaces).toHaveLength(1);
    expect(out.workspaces[0].clusters).toEqual(["a"]);
  });

  it("caps the closed list", () => {
    const closed = Array.from({ length: CLOSED_CAP + 5 }, (_, i) => makeTab(`/resources/r${i}`));
    const out = reconcile({ workspaces: [ws({ closed })], currentId: "w1" }, []);
    expect(out.workspaces[0].closed).toHaveLength(CLOSED_CAP);
    // Most recent first: the front survives.
    expect(out.workspaces[0].closed[0].route).toBe("/resources/r0");
  });

  it("clears an active cluster that vanished and picks the first that remains", () => {
    const s = defaultState([ctx("a"), ctx("b")]);
    s.workspaces[0].activeCluster = "a";
    expect(reconcile(s, [ctx("b")]).workspaces[0].activeCluster).toBe("b");
    expect(reconcile(s, []).workspaces[0].activeCluster).toBeUndefined();
  });

  it("adopts the first cluster for a workspace stored before there was an active one", () => {
    // Storage version 1 predates the field, so the documents on disk have no
    // `activeCluster`: boot has to fill it in or the rail comes up with
    // clusters and nothing selected.
    const state: TabsState = { workspaces: [ws({ clusters: ["a", "b"] })], currentId: "w1" };
    expect(reconcile(state, [ctx("a"), ctx("b")]).workspaces[0].activeCluster).toBe("a");
  });

  it("returns the same object when nothing needed changing", () => {
    // So a store can compare by identity and skip a save.
    const w = ws({ clusters: ["a"], activeCluster: "a" });
    w.activeId = w.tabs[0].id;
    const state: TabsState = { workspaces: [w], currentId: "w1" };
    expect(reconcile(state, [ctx("a")])).toBe(state);
  });
});
