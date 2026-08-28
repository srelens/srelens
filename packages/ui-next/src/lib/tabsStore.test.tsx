import { describe, it, expect, beforeEach, vi } from "vitest";
import { renderHook, act } from "@testing-library/react";
import type { ClusterContext } from "@srelens/core";
import { defaultState, type TabsState } from "./tabs";
import * as store from "./tabsStore";

const ctx = (id: string): ClusterContext => ({
  name: id, stableId: id, cluster: id, server: `https://${id}`, isCurrent: false,
  sourceFile: "/home/dana/.kube/config", authKind: "client certificate",
});

function seed(over: Partial<TabsState> = {}) {
  const base = defaultState([]);
  store.setState({ ...base, ...over });
  return store.getState();
}

/**
 * A context whose stableId is NOT its name. Workspaces hold stableIds (#265)
 * and tabs carry names, so a fixture where the two are the same string cannot
 * tell a relabel from a stableId written straight onto the strip.
 */
const named = (stableId: string, name: string): ClusterContext => ({ ...ctx(stableId), name });

const routes = () => store.currentWorkspace().tabs.map((t) => t.route);
const subFor = (route: string) => store.currentWorkspace().tabs.find((t) => t.route === route)?.sub;
const active = () => store.currentWorkspace().tabs.find((t) => t.id === store.currentWorkspace().activeId)!;

beforeEach(() => {
  seed();
});

describe("useTabs", () => {
  it("reflects the current workspace and updates when it changes", () => {
    const { result } = renderHook(() => store.useTabs());
    expect(result.current.tabs.map((t) => t.route)).toEqual(["/"]);
    act(() => store.openTab("/k/pods"));
    expect(result.current.tabs.map((t) => t.route)).toEqual(["/", "/k/pods"]);
    expect(result.current.activeId).toBe(result.current.tabs[1].id);
  });
});

describe("openTab", () => {
  it("activates a route that is already open instead of opening it twice", () => {
    store.openTab("/k/pods");
    const first = active().id;
    store.openTab("/");
    store.openTab("/k/pods");
    expect(routes()).toEqual(["/", "/k/pods"]);
    expect(active().id).toBe(first);
  });

  it("replaces the current preview with a new preview", () => {
    // An editor's behaviour: single-click previews replace each other.
    store.openTab("/resources/a", { preview: true });
    store.openTab("/resources/b", { preview: true });
    expect(routes()).toEqual(["/", "/resources/b"]);
  });

  it("promotes a preview when the same route is opened for real", () => {
    store.openTab("/resources/a", { preview: true });
    store.openTab("/resources/a");
    expect(active().preview).toBeFalsy();
  });

  it("leaves a preview alone when re-previewing it", () => {
    store.openTab("/resources/a", { preview: true });
    store.openTab("/resources/a", { preview: true });
    expect(active().preview).toBe(true);
  });

  it("carries the cluster name into the tab", () => {
    store.openTab("/k/pods", { clusterName: "staging" });
    expect(active().sub).toBe("staging");
  });

  /**
   * **The bug this pins: a second cluster opened onto a tab still labelled with
   * the first.**
   *
   * `openTab` dedupes by ROUTE, which is right for most of its callers — a
   * detail route, a list route and `/overview` are each meant to be one tab.
   * But it returned with `activeId: existing.id` and never looked at
   * `clusterName`, and `makeTab` spends `clusterName` on the `sub`, so the
   * label was baked in at creation. With an `/overview` tab open for one
   * cluster, opening a second changed the workspace's active cluster and reused
   * the first tab: the screen rendered the second cluster while the strip read
   * the first. Every caller that names a cluster — `Nav`, `Status`,
   * `openCluster`, and the six screens that open a detail route — had the same
   * hole.
   */
  it("relabels a reused tab for the cluster the caller named", () => {
    store.openTab("/overview", { clusterName: "prod-eu" });
    const first = active().id;
    store.openTab("/overview", { clusterName: "staging-eu" });
    // One tab, still — the route dedupe is not what was wrong.
    expect(routes()).toEqual(["/", "/overview"]);
    expect(active().id).toBe(first);
    expect(active().sub).toBe("staging-eu");
  });

  it("leaves a tab's cluster alone when the caller names none", () => {
    // Cmd-clicking a route from a shortcut says nothing about a cluster, and it
    // must not blank the label of the tab it lands on.
    store.openTab("/overview", { clusterName: "prod-eu" });
    store.openTab("/overview");
    expect(active().sub).toBe("prod-eu");
  });

  /**
   * **Relabelling goes through `describe`, not through `sub = clusterName`.**
   *
   * `describe` drops `sub` for an app-scoped route (`{ route, ...app }`, with no
   * `sub` spread in), because `/connections` and `/connect` are not about a
   * cluster. Assigning the caller's `clusterName` straight onto the tab would
   * put a cluster name under the strip's `Connections` label — a tab claiming
   * to be scoped to a cluster it has nothing to do with. The route table stays
   * the one place that decides whether a route carries a cluster.
   */
  it("puts no cluster on a tab whose route is not about one", () => {
    store.openTab("/connections");
    store.openTab("/connections", { clusterName: "staging-eu" });
    expect(active().sub).toBeUndefined();
  });

  it("promotes a preview and relabels it in one go", () => {
    store.openTab("/overview", { preview: true, clusterName: "prod-eu" });
    store.openTab("/overview", { clusterName: "staging-eu" });
    expect(active().preview).toBeFalsy();
    expect(active().sub).toBe("staging-eu");
  });

  it("newTab always appends, even when the route is open", () => {
    store.newTab("/");
    expect(routes()).toEqual(["/", "/"]);
  });

  it("newTab makes a closeable tab, even on a route that is pinned by default", () => {
    // `pinned` belongs to the seed home tab, not to "/" — a Cmd+T tab the user
    // cannot then close is a trap, and every close path refuses a pinned tab.
    store.newTab("/");
    expect(active().pinned).toBeFalsy();
    expect(active().preview).toBeFalsy();
    store.closeTab(active().id);
    expect(routes()).toEqual(["/"]);
  });
});

describe("closeTab", () => {
  it("activates the right neighbour, then the left at the end", () => {
    store.openTab("/a"); store.openTab("/b"); store.openTab("/c");
    const b = store.currentWorkspace().tabs[2].id;
    store.activateTab(b);
    store.closeTab(b);
    expect(active().route).toBe("/c");
    store.closeTab(active().id);
    expect(active().route).toBe("/a");
  });

  it("refuses a pinned tab", () => {
    const home = active().id;
    store.closeTab(home);
    expect(routes()).toEqual(["/"]);
  });

  it("refuses the last tab even when unpinned", () => {
    seed();
    store.togglePin(active().id);
    store.closeTab(active().id);
    expect(routes()).toHaveLength(1);
  });

  it("remembers what it closed, most recent first, per workspace", () => {
    store.openTab("/a"); store.openTab("/b");
    store.closeTab(store.currentWorkspace().tabs[1].id);
    store.closeTab(store.currentWorkspace().tabs[1].id);
    expect(store.currentWorkspace().closed.map((t) => t.route)).toEqual(["/b", "/a"]);
  });

  it("does not close a tab that is not in the current workspace", () => {
    store.openTab("/a");
    const other = store.createWorkspace("Other", []);
    store.switchWorkspace(other);
    store.closeTab(store.getState().workspaces[0].tabs[1].id);
    expect(store.getState().workspaces[0].tabs.map((t) => t.route)).toEqual(["/", "/a"]);
  });
});

describe("closeOthers / closeToRight / closeAll", () => {
  it("closeOthers keeps the named tab and every pinned one", () => {
    store.openTab("/a"); store.openTab("/b");
    const b = active().id;
    store.closeOthers(b);
    expect(routes()).toEqual(["/", "/b"]);
    expect(active().id).toBe(b);
  });

  it("closeToRight drops unpinned tabs after the named one", () => {
    store.openTab("/a"); store.openTab("/b"); store.openTab("/c");
    const a = store.currentWorkspace().tabs[1].id;
    store.closeToRight(a);
    expect(routes()).toEqual(["/", "/a"]);
  });

  it("closeAll keeps pinned tabs, or makes a home tab if none were", () => {
    store.openTab("/a");
    store.closeAll();
    expect(routes()).toEqual(["/"]);
    store.togglePin(active().id);
    store.closeAll();
    expect(routes()).toEqual(["/"]);
    expect(active().pinned).toBe(true);
  });
});

describe("reopenClosed", () => {
  it("brings back the most recently closed tab with a fresh id", () => {
    store.openTab("/a");
    const closedId = active().id;
    store.closeTab(closedId);
    store.reopenClosed();
    expect(active().route).toBe("/a");
    expect(active().id).not.toBe(closedId);
    expect(store.currentWorkspace().closed).toHaveLength(0);
  });

  it("does nothing when nothing was closed", () => {
    store.reopenClosed();
    expect(routes()).toEqual(["/"]);
  });
});

describe("duplicateTab / togglePin", () => {
  it("duplicates beside the original, unpinned and not a preview", () => {
    store.openTab("/a", { preview: true });
    const src = active().id;
    store.duplicateTab(src);
    expect(routes()).toEqual(["/", "/a", "/a"]);
    expect(active().id).not.toBe(src);
    expect(active().preview).toBeFalsy();
  });

  it("toggles pin", () => {
    store.openTab("/a");
    store.togglePin(active().id);
    expect(active().pinned).toBe(true);
    store.togglePin(active().id);
    expect(active().pinned).toBe(false);
  });
});

describe("cycleTab / selectIndex", () => {
  it("cycles with wrap", () => {
    store.openTab("/a"); store.openTab("/b");
    store.cycleTab(1);
    expect(active().route).toBe("/");
    store.cycleTab(-1);
    expect(active().route).toBe("/b");
  });

  it("selects by index and ignores one out of range", () => {
    store.openTab("/a");
    store.selectIndex(0);
    expect(active().route).toBe("/");
    store.selectIndex(9);
    expect(active().route).toBe("/");
  });
});

describe("workspaces", () => {
  it("creates, switches, and starts the new one with a home tab", () => {
    const id = store.createWorkspace("Prod", ["c1"]);
    expect(store.getState().currentId).toBe(id);
    expect(store.currentWorkspace()).toMatchObject({ name: "Prod", clusters: ["c1"] });
    expect(routes()).toEqual(["/"]);
  });

  it("keeps each workspace's tabs apart", () => {
    store.openTab("/a");
    const id = store.createWorkspace("Other", []);
    store.openTab("/b");
    expect(routes()).toEqual(["/", "/b"]);
    store.switchWorkspace(store.getState().workspaces[0].id);
    expect(routes()).toEqual(["/", "/a"]);
    store.switchWorkspace(id);
    expect(routes()).toEqual(["/", "/b"]);
  });

  it("ignores a switch to a workspace that does not exist", () => {
    const before = store.getState().currentId;
    store.switchWorkspace("nope");
    expect(store.getState().currentId).toBe(before);
  });

  it("renames", () => {
    const id = store.getState().currentId;
    store.renameWorkspace(id, "Renamed");
    expect(store.currentWorkspace().name).toBe("Renamed");
  });

  it("removes, moving to a neighbour, and refuses the last one", () => {
    const first = store.getState().currentId;
    const second = store.createWorkspace("Two", []);
    store.removeWorkspace(second);
    expect(store.getState().workspaces).toHaveLength(1);
    expect(store.getState().currentId).toBe(first);
    store.removeWorkspace(first);
    expect(store.getState().workspaces).toHaveLength(1);
  });

  it("sets a workspace's clusters", () => {
    const id = store.getState().currentId;
    store.setWorkspaceClusters(id, ["x", "y"]);
    expect(store.currentWorkspace().clusters).toEqual(["x", "y"]);
  });
});

describe("activeCluster", () => {
  it("setActiveCluster accepts a cluster of the workspace and nothing else", () => {
    store.setState(defaultState([ctx("a"), ctx("b")]));
    store.setActiveCluster("b");
    expect(store.activeCluster()).toBe("b");
    store.setActiveCluster("zzz");
    expect(store.activeCluster()).toBe("b");
    store.setActiveCluster(null);
    expect(store.activeCluster()).toBeNull();
  });

  /**
   * **The rail switches cluster; the strip has to follow.** Every cluster-scoped
   * tab is about whichever cluster is active — nothing pins a tab to one, and
   * `useActiveContext()` is the single answer every screen reads — so a tab
   * whose `sub` still names the cluster before the switch labels the screen
   * with a cluster it is not showing, and an action run from that tab runs
   * against one cluster under a tab reading another.
   *
   * Through `relabel`, so the route table stays the one place that decides
   * whether a route carries a cluster at all: `/settings` is app-scoped and
   * keeps no `sub`.
   *
   * The fixtures' stableIds are deliberately not their names, so these
   * assertions can tell a relabel from a stableId written onto the strip.
   */
  it("relabels every cluster-scoped tab for the cluster it switches to", () => {
    store.setState(defaultState([named("id-prod", "prod-eu"), named("id-stage", "staging-eu")]));
    store.openTab("/overview", { clusterName: "prod-eu" });
    store.openTab("/k/pods", { clusterName: "prod-eu" });
    store.openTab("/settings");

    store.setActiveCluster("id-stage", "staging-eu");

    const tabs = store.currentWorkspace().tabs;
    expect(tabs.filter((t) => t.sub === "prod-eu")).toEqual([]);
    expect(subFor("/overview")).toBe("staging-eu");
    expect(subFor("/k/pods")).toBe("staging-eu");
    expect(subFor("/")).toBe("staging-eu");
    // A stableId on the strip would satisfy "no longer prod-eu" and be the
    // same bug wearing the other name.
    expect(tabs.filter((t) => t.sub === "id-stage")).toEqual([]);
    // App-scoped: `/settings` is not about a cluster and gains no sub.
    expect(subFor("/settings")).toBeUndefined();
  });

  it("relabels a stale tab even when the cluster is already the active one", () => {
    // How a persisted session comes back: the active cluster is restored from
    // the file, the tabs come back with whatever `sub` was written last.
    store.setState(defaultState([named("id-prod", "prod-eu")]));
    store.openTab("/overview", { clusterName: "staging-eu" });

    store.setActiveCluster("id-prod", "prod-eu");

    expect(subFor("/overview")).toBe("prod-eu");
  });

  it("relabels nothing when it refuses the cluster", () => {
    store.setState(defaultState([named("id-prod", "prod-eu")]));
    store.openTab("/overview", { clusterName: "prod-eu" });

    store.setActiveCluster("not-in-this-workspace", "somewhere-else");

    expect(store.activeCluster()).toBe("id-prod");
    expect(subFor("/overview")).toBe("prod-eu");
  });

  it("setActiveCluster does not notify for a no-op", () => {
    store.setState(defaultState([ctx("a")]));
    const spy = vi.fn();
    const off = store.subscribe(spy);
    store.setActiveCluster("a"); // already "a" from defaultState
    expect(spy).not.toHaveBeenCalled();
    off();
  });

  it("tells the hook", () => {
    store.setState(defaultState([ctx("a"), ctx("b")]));
    const { result } = renderHook(() => store.useActiveCluster());
    expect(result.current).toBe("a");
    act(() => store.setActiveCluster("b"));
    expect(result.current).toBe("b");
  });

  it("is the first cluster of a workspace just created", () => {
    // Otherwise a new workspace comes up with a full rail and nothing
    // selected until the next `reconcile`, which only runs when the machine's
    // contexts change.
    store.createWorkspace("Prod", ["c1", "c2"]);
    expect(store.activeCluster()).toBe("c1");
    store.createWorkspace("Empty", []);
    expect(store.activeCluster()).toBeNull();
  });

  it("stays valid when the workspace's clusters are replaced", () => {
    store.setState(defaultState([ctx("a"), ctx("b")]));
    const id = store.getState().currentId;
    store.setWorkspaceClusters(id, ["b", "c"]);
    expect(store.activeCluster()).toBe("b");
    store.setWorkspaceClusters(id, ["c", "d"]);
    expect(store.activeCluster()).toBe("c");
    store.setWorkspaceClusters(id, []);
    expect(store.activeCluster()).toBeNull();
  });
});

describe("setTabView / useTabView", () => {
  it("defaults to an empty view for a tab with none set", () => {
    const { result } = renderHook(() => store.useTabView(active().id));
    expect(result.current).toEqual({});
  });

  it("merges a patch into what the tab already holds", () => {
    const id = active().id;
    store.setTabView(id, { filter: "abc" });
    store.setTabView(id, { sort: { key: "name", direction: "asc" } });
    expect(store.currentWorkspace().tabs.find((t) => t.id === id)?.view).toEqual({
      filter: "abc",
      sort: { key: "name", direction: "asc" },
    });
  });

  it("tells the hook when the view changes", () => {
    const id = active().id;
    const { result } = renderHook(() => store.useTabView(id));
    act(() => store.setTabView(id, { filter: "x" }));
    expect(result.current.filter).toBe("x");
  });

  it("ignores an id that is not there", () => {
    const before = store.getState();
    store.setTabView("nope", { filter: "x" });
    expect(store.getState()).toBe(before);
  });
});

describe("activeRoute", () => {
  it("is the active tab's route", () => {
    store.openTab("/k/pods");
    expect(store.activeRoute()).toBe("/k/pods");
  });
});

describe("subscribe", () => {
  it("notifies on change and stops after unsubscribe", () => {
    let n = 0;
    const off = store.subscribe(() => n++);
    store.openTab("/a");
    expect(n).toBe(1);
    off();
    store.openTab("/b");
    expect(n).toBe(1);
  });

  it("does not notify for a no-op action", () => {
    let n = 0;
    store.subscribe(() => n++);
    store.activateTab(active().id);
    store.switchWorkspace("nope");
    expect(n).toBe(0);
  });
});

describe("no-op actions do not notify", () => {
  /**
   * The store's subscribers include the one that writes the settings file, so an
   * action that changes nothing must not emit. Every guard below is a line of
   * `tabsStore.ts` that looks redundant until you delete it; these are the tests
   * that make deleting it fail.
   */
  function silent(name: string, arrange: () => void, action: () => void) {
    it(name, () => {
      arrange();
      let n = 0;
      const off = store.subscribe(() => n++);
      const before = store.getState();
      action();
      off();
      expect(n, "notifications").toBe(0);
      expect(store.getState(), "state identity").toBe(before);
    });
  }

  const activeId = () => store.currentWorkspace().activeId;
  /** Two tabs, both pinned, the first of them active. */
  function twoPinnedFirstActive() {
    store.openTab("/a");
    store.togglePin(activeId());
    store.activateTab(store.currentWorkspace().tabs[0].id);
  }

  silent("openTab on the active, non-preview route", () => {}, () => store.openTab("/"));

  // The relabel must be a no-op when there is nothing to relabel: one
  // subscriber writes a file, and re-opening the cluster you are already
  // looking at must not schedule a save.
  silent(
    "openTab naming the cluster the tab already carries",
    () => store.openTab("/overview", { clusterName: "prod-eu" }),
    () => store.openTab("/overview", { clusterName: "prod-eu" }),
  );

  silent(
    "closeOthers when every other tab is pinned and this one is active",
    () => store.openTab("/a"),
    () => store.closeOthers(activeId()),
  );

  silent("closeToRight when everything to the right is pinned", twoPinnedFirstActive, () =>
    store.closeToRight(activeId()),
  );

  silent("closeAll when every tab is pinned and the first is active", twoPinnedFirstActive, () => store.closeAll());

  silent("togglePin on an id that is not there", () => {}, () => store.togglePin("nope"));

  // The cluster switch relabels the strip, so it has to be as quiet as
  // `openTab` is when there is nothing to relabel: picking the cluster you are
  // already on must not schedule a save.
  silent(
    "setActiveCluster naming the cluster every tab already carries",
    () => {
      store.setState(defaultState([named("id-prod", "prod-eu")]));
      store.setActiveCluster("id-prod", "prod-eu");
    },
    () => store.setActiveCluster("id-prod", "prod-eu"),
  );

  silent(
    "setWorkspaceClusters with a list equal to the one it has",
    () => store.setWorkspaceClusters(store.getState().currentId, ["x", "y"]),
    () => store.setWorkspaceClusters(store.getState().currentId, ["x", "y"]),
  );

  silent("renameWorkspace to the name it already has", () => {}, () =>
    store.renameWorkspace(store.getState().currentId, "Default"),
  );

  silent(
    "setTabView with values it already holds",
    () => store.setTabView(activeId(), { filter: "abc", sort: { key: "name", direction: "asc" } }),
    () => store.setTabView(activeId(), { filter: "abc", sort: { key: "name", direction: "asc" } }),
  );

  it("but a real change still notifies exactly once", () => {
    let n = 0;
    const off = store.subscribe(() => n++);
    const before = store.getState();
    store.openTab("/a");
    off();
    expect(n).toBe(1);
    expect(store.getState()).not.toBe(before);
  });
});
