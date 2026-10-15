import { describe, it, expect, beforeEach, vi } from "vitest";
import { renderHook, act } from "@testing-library/react";
import type { ClusterContext } from "@srelens/core";
import { CLOSED_CAP, defaultState, type TabsState } from "./tabs";
import { detailRoute } from "./detailRoute";
import * as store from "./tabsStore";

const ctx = (id: string): ClusterContext => ({
  name: id, stableId: id, key: id, cluster: id, server: `https://${id}`, isCurrent: false,
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

  /**
   * **#834: one resource, two clusters, two tabs.** A node name, a system pod,
   * anything deployed from one manifest to staging and prod collides on kind,
   * namespace and name. With no cluster in the detail route the second open
   * focused the first tab and relabelled it, and the first cluster's object
   * was not open anywhere any more.
   */
  it("opens a second detail tab for the same resource on another cluster, each labelled with its own", () => {
    const prod = detailRoute("Node", null, "worker-1", { cluster: "prod" });
    const staging = detailRoute("Node", null, "worker-1", { cluster: "staging" });
    store.openTab(prod, { clusterName: "prod" });
    store.openTab(staging, { clusterName: "staging" });
    expect(routes()).toEqual(["/", prod, staging]);
    expect(subFor(prod)).toBe("prod");
    expect(subFor(staging)).toBe("staging");
  });

  it("still focuses the open detail tab for the same resource on the same cluster", () => {
    const prod = detailRoute("Node", null, "worker-1", { cluster: "prod" });
    store.openTab(prod, { clusterName: "prod" });
    const first = active().id;
    store.openTab("/");
    store.openTab(prod, { clusterName: "prod" });
    expect(routes()).toEqual(["/", prod]);
    expect(active().id).toBe(first);
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

/**
 * #839: a tab opened from another started with no namespace selection, so
 * Pods narrowed to two namespaces and then Deployments from the sidebar was
 * Deployments on "all namespaces" — a refusal, for a reader whose credentials
 * are namespace-scoped — on every kind they visited.
 *
 * A tab now STARTS from the selection of the tab it was opened from, and is
 * its own from then on.
 */
describe("openTab carries the namespace selection of the tab it was opened from (#839)", () => {
  const PROD = "prod-id";
  const STAGE = "stage-id";
  const tabFor = (route: string) => store.currentWorkspace().tabs.find((t) => t.route === route)!;
  const selection = (route: string, cluster = PROD) => store.tabNamespaces(tabFor(route).id, cluster);

  /** Open Pods and narrow it, leaving it the active tab. */
  function narrowedPods(namespaces: string[] = ["team-a", "team-b"], cluster = PROD) {
    store.openTab("/k/pods");
    store.setTabNamespaces(active().id, cluster, namespaces);
  }

  /** #881: a collection scope travels as a namespace selection does. */
  it("gives a new tab the namespace collections the tab it was opened from is scoped to", () => {
    store.openTab("/k/pods");
    store.setTabNamespaceCollections(active().id, PROD, ["prod"]);
    store.openTab("/k/deployments");
    expect(store.tabNamespaceCollections(tabFor("/k/deployments").id, PROD)).toEqual(["prod"]);
    // A copy, not the source's array.
    store.setTabNamespaceCollections(tabFor("/k/pods").id, PROD, ["staging"]);
    expect(store.tabNamespaceCollections(tabFor("/k/deployments").id, PROD)).toEqual(["prod"]);
  });

  it("leaves a tab's own choice for a cluster alone, either way it was made", () => {
    store.openTab("/k/deployments");
    store.setTabNamespaces(active().id, PROD, ["billing"]);
    store.openTab("/k/pods");
    store.setTabNamespaceCollections(active().id, PROD, ["prod"]);
    store.openTab("/k/deployments");
    expect(store.tabNamespaceCollections(tabFor("/k/deployments").id, PROD)).toBeUndefined();
    expect(selection("/k/deployments")).toEqual(["billing"]);
  });

  it("gives a new tab the namespaces selected on the tab it was opened from", () => {
    narrowedPods();
    store.openTab("/k/deployments");

    expect(active().route).toBe("/k/deployments");
    expect(selection("/k/deployments")).toEqual(["team-a", "team-b"]);
  });

  it("carries an explicit \"all namespaces\" as that, not as nothing chosen", () => {
    // An empty array is a choice — it overrides the default-namespace
    // preference — and must arrive as one.
    narrowedPods([]);
    store.openTab("/k/deployments");
    expect(selection("/k/deployments")).toEqual([]);
  });

  it("leaves the new tab unset when nothing was selected on the source, so it follows the default", () => {
    store.openTab("/k/pods");
    store.openTab("/k/deployments");
    expect(selection("/k/deployments")).toBeUndefined();
    expect(tabFor("/k/deployments").namespaces).toBeUndefined();
  });

  it("carries every cluster's selection, each under its own stable id", () => {
    narrowedPods(["team-a"], PROD);
    store.setTabNamespaces(active().id, STAGE, ["sandbox"]);

    store.openTab("/k/deployments");

    expect(selection("/k/deployments", PROD)).toEqual(["team-a"]);
    expect(selection("/k/deployments", STAGE)).toEqual(["sandbox"]);
  });

  it("is a copy: changing one tab afterwards does not change the other", () => {
    narrowedPods();
    store.openTab("/k/deployments");

    // Not even the array is shared between them — checked here, straight
    // after the open: every `setTabNamespaces` below replaces a tab's array,
    // so by the end the two would differ even if the copy had been a share.
    expect(selection("/k/deployments")).toEqual(["team-a", "team-b"]);
    expect(tabFor("/k/pods").namespaces![PROD]).not.toBe(tabFor("/k/deployments").namespaces![PROD]);

    store.setTabNamespaces(tabFor("/k/deployments").id, PROD, ["team-c"]);
    expect(selection("/k/pods")).toEqual(["team-a", "team-b"]);

    store.setTabNamespaces(tabFor("/k/pods").id, PROD, ["team-d"]);
    expect(selection("/k/deployments")).toEqual(["team-c"]);
  });

  it("chains: a tab opened from a tab that inherited carries the same selection on", () => {
    // Pods → a pod's detail tab → Deployments from the sidebar.
    narrowedPods();
    store.openTab("/k/Pod/team-a/web-0");
    store.openTab("/k/deployments");
    expect(selection("/k/deployments")).toEqual(["team-a", "team-b"]);
  });

  it("fills an existing tab that was never narrowed, since it is in the state a new tab is", () => {
    // Tabs persist: the Deployments tab reached from the sidebar is as often
    // yesterday's as a fresh one.
    store.openTab("/k/deployments");
    narrowedPods();

    store.openTab("/k/deployments");

    expect(routes().filter((r) => r === "/k/deployments")).toHaveLength(1);
    expect(selection("/k/deployments")).toEqual(["team-a", "team-b"]);
  });

  it("never overrules a selection the existing tab already has for that cluster", () => {
    store.openTab("/k/deployments");
    store.setTabNamespaces(active().id, PROD, ["billing"]);
    narrowedPods();

    store.openTab("/k/deployments");

    expect(selection("/k/deployments")).toEqual(["billing"]);
  });

  it("keeps an existing tab's own explicit \"all namespaces\" too", () => {
    store.openTab("/k/deployments");
    store.setTabNamespaces(active().id, PROD, []);
    narrowedPods();
    store.openTab("/k/deployments");
    expect(selection("/k/deployments")).toEqual([]);
  });

  it("fills only the clusters the existing tab has no entry for", () => {
    store.openTab("/k/deployments");
    store.setTabNamespaces(active().id, PROD, ["billing"]);
    narrowedPods(["team-a"], PROD);
    store.setTabNamespaces(active().id, STAGE, ["sandbox"]);

    store.openTab("/k/deployments");

    expect(selection("/k/deployments", PROD)).toEqual(["billing"]);
    expect(selection("/k/deployments", STAGE)).toEqual(["sandbox"]);
  });

  it("gives a replaced preview the selection as well", () => {
    narrowedPods();
    store.openTab("/k/Pod/team-a/web-0", { preview: true });
    store.openTab("/k/pods");
    store.openTab("/k/Pod/team-a/web-1", { preview: true });
    expect(selection("/k/Pod/team-a/web-1")).toEqual(["team-a", "team-b"]);
  });

  it("does not carry a selection into a blank new tab", () => {
    // Cmd+T is a fresh start, not a move from where the reader was.
    narrowedPods();
    store.newTab("/");
    expect(active().namespaces).toBeUndefined();
  });

  it("does not notify when re-opening the active tab carries nothing new", () => {
    narrowedPods();
    const listener = vi.fn();
    const off = store.subscribe(listener);
    store.openTab("/k/pods");
    off();
    expect(listener).not.toHaveBeenCalled();
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

  it("brings back the closed tab it is named, leaving the others closed", () => {
    store.openTab("/a");
    const a = active().id;
    store.openTab("/b");
    const b = active().id;
    store.closeTab(a);
    store.closeTab(b);
    store.reopenClosed(a);
    expect(active().route).toBe("/a");
    expect(store.currentWorkspace().closed.map((t) => t.route)).toEqual(["/b"]);
    store.reopenClosed("no-such-tab");
    expect(routes()).toEqual(["/", "/a"]);
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

  it("trims a new name", () => {
    const id = store.getState().currentId;
    store.renameWorkspace(id, "  Production  ");
    expect(store.currentWorkspace().name).toBe("Production");
  });

  it("sets, changes and clears a workspace's colour", () => {
    const id = store.getState().currentId;
    store.setWorkspaceColor(id, "var(--mark-teal)");
    expect(store.currentWorkspace().color).toBe("var(--mark-teal)");
    store.setWorkspaceColor(id, "var(--mark-red)");
    expect(store.currentWorkspace().color).toBe("var(--mark-red)");
    store.setWorkspaceColor(id, undefined);
    expect(store.currentWorkspace()).not.toHaveProperty("color");
  });

  it("refuses a colour that is not in the mark palette", () => {
    const id = store.getState().currentId;
    store.setWorkspaceColor(id, "var(--mark-teal)");
    store.setWorkspaceColor(id, "#ff0000");
    expect(store.currentWorkspace().color).toBe("var(--mark-teal)");
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

  it("keeps a pause in its workspace and clears it when that cluster is removed", () => {
    const id = store.getState().currentId;
    store.setWorkspaceClusters(id, ["x", "y"]);
    store.setClusterPaused(id, "x", true);
    expect(store.isClusterPaused("x")).toBe(true);
    store.setWorkspaceClusters(id, ["y"]);
    expect(store.currentWorkspace().pausedClusters).toEqual([]);
  });
});

describe("setState", () => {
  /**
   * The store had two answers to "which workspace is current" and they
   * disagreed whenever `currentId` named none of them. The readers fell back
   * to `workspaces[0]` — `currentWorkspace()` and `useTabs()` both end
   * `?? s.workspaces[0]` — so the window drew a full strip. The writers did
   * not: `patchCurrent` hands `cur().currentId` to `patchWorkspace`, which
   * bails on `at < 0`. So the window was drawn and inert, and because no
   * `emit` fired nothing was persisted either. No error appeared anywhere.
   *
   * The repair lives in `setState` rather than in the one boot branch that
   * could install it, so no future caller can install it either.
   */
  it("repairs a currentId that names no workspace, so the strip the reader sees is the one the actions write to", () => {
    const base = defaultState([ctx("a"), ctx("b")]);
    store.setState({ ...base, currentId: "a-workspace-that-did-not-parse" });

    expect(store.getState().currentId).toBe(base.workspaces[0].id);
    // The rail could not switch cluster.
    store.setActiveCluster("b");
    expect(store.activeCluster()).toBe("b");
    // Cmd+T did nothing.
    store.openTab("/k/pods");
    expect(routes()).toEqual(["/", "/overview", "/k/pods"]);
  });

  it("hands a sound document straight through, so an identical write wakes no subscriber", () => {
    // The identity discipline the rest of this store keeps: nothing needed
    // repairing, so nothing is copied, and `emit` can still recognise a write
    // of the state already held — one of the subscribers writes a file.
    const state = defaultState([ctx("a")]);
    store.setState(state);
    expect(store.getState()).toBe(state);

    let n = 0;
    const off = store.subscribe(() => n++);
    store.setState(state);
    off();
    expect(n).toBe(0);
  });

  it("leaves a dangling currentId alone when there is no workspace to point it at", () => {
    // An empty document is the boot branch's other case and `Window` guards it
    // by installing a default instead; the repair must not invent an id, since
    // `currentWorkspace()` has nothing to return either way.
    store.setState({ workspaces: [], currentId: "gone" });
    expect(store.getState().currentId).toBe("gone");
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
   * **Each cluster keeps its own tabs (#908).** The strip used to follow the
   * rail: every cluster-scoped tab was relabelled for the cluster switched to,
   * so Pods, ConfigMaps and Services opened while working on one cluster were
   * still open on the next, showing its objects, where nobody had opened them.
   *
   * The fixtures' stableIds are deliberately not their names, so these
   * assertions can tell a label from a stableId written onto the strip.
   */
  describe("each cluster keeps its own tabs", () => {
    const PROD = named("id-prod", "prod-eu");
    const STAGE = named("id-stage", "staging-eu");
    const toProd = () => store.setActiveCluster("id-prod", "prod-eu");
    const toStage = () => store.setActiveCluster("id-stage", "staging-eu");
    const subs = () => store.currentWorkspace().tabs.map((t) => t.sub);

    beforeEach(() => {
      store.setState(defaultState([PROD, STAGE]));
      store.openTab("/k/pods", { clusterName: "prod-eu" });
      store.openTab("/k/configmaps", { clusterName: "prod-eu" });
      store.openTab("/k/services", { clusterName: "prod-eu" });
    });

    it("does not carry one cluster's tabs to the next: a cluster seen for the first time opens on its overview", () => {
      toStage();
      expect(routes()).toEqual(["/", "/overview"]);
      expect(active().route).toBe("/overview");
      expect(subs()).toEqual([undefined, "staging-eu"]);
    });

    it("brings a cluster's tabs back as they were left, with the tab that was active there", () => {
      store.activateTab(store.currentWorkspace().tabs[2].id);
      toStage();
      store.openTab("/k/nodes", { clusterName: "staging-eu" });

      toProd();
      expect(routes()).toEqual(["/", "/k/pods", "/k/configmaps", "/k/services"]);
      expect(active().route).toBe("/k/configmaps");
      expect(subs()).toEqual([undefined, "prod-eu", "prod-eu", "prod-eu"]);

      toStage();
      expect(routes()).toEqual(["/", "/overview", "/k/nodes"]);
      expect(active().route).toBe("/k/nodes");
    });

    it("keeps each tab's namespaces, sort and filter with it", () => {
      const pods = store.currentWorkspace().tabs[1].id;
      store.setTabNamespaces(pods, "id-prod", ["payments", "checkout"]);
      store.setTabView(pods, { filter: "api" });
      toStage();
      toProd();
      expect(store.tabNamespaces(pods, "id-prod")).toEqual(["payments", "checkout"]);
      expect(store.currentWorkspace().tabs[1].view).toEqual({ filter: "api" });
    });

    it("does not let what is opened, closed or moved on one cluster touch another's", () => {
      toStage();
      store.openTab("/k/pods", { clusterName: "staging-eu" });
      store.closeTab(store.currentWorkspace().tabs.find((t) => t.route === "/overview")!.id);
      toProd();
      expect(routes()).toEqual(["/", "/k/pods", "/k/configmaps", "/k/services"]);
      store.closeTab(store.currentWorkspace().tabs[1].id);
      store.moveTab(store.currentWorkspace().tabs[2].id, 1);
      toStage();
      expect(routes()).toEqual(["/", "/k/pods"]);
      expect(subs()).toEqual([undefined, "staging-eu"]);
      toProd();
      expect(routes()).toEqual(["/", "/k/services", "/k/configmaps"]);
    });

    it("reopens a tab closed on the cluster in focus, not one closed on another", () => {
      store.closeTab(store.currentWorkspace().tabs.find((t) => t.route === "/k/services")!.id);
      toStage();
      expect(store.currentWorkspace().closed).toEqual([]);
      store.reopenClosed();
      expect(routes()).toEqual(["/", "/overview"]);
      toProd();
      store.reopenClosed();
      expect(routes()).toEqual(["/", "/k/pods", "/k/configmaps", "/k/services"]);
    });

    /**
     * Home, Settings and Connections are the same screen whichever cluster is
     * in focus, so they stay — where they were in the strip, not swept to one
     * end each time the cluster changes.
     */
    it("lets an app-wide tab closed on one cluster be reopened on another", () => {
      store.openTab("/settings");
      store.closeTab(store.currentWorkspace().tabs.find((t) => t.route === "/settings")!.id);
      store.closeTab(store.currentWorkspace().tabs.find((t) => t.route === "/k/services")!.id);
      toStage();
      expect(store.currentWorkspace().closed.map((t) => t.route)).toEqual(["/settings"]);
      store.reopenClosed();
      expect(routes()).toEqual(["/", "/overview", "/settings"]);
      toProd();
      expect(store.currentWorkspace().closed.map((t) => t.route)).toEqual(["/k/services"]);
    });

    /**
     * A cluster's own closed tabs come back beside the app-wide ones that
     * stayed, and the two are capped together. With the cluster's own put
     * first, a cluster that had a full list of its own pushed every app-wide
     * closed tab off the end, including one closed a moment ago.
     */
    it("keeps a tab just closed when a cluster with a full closed list of its own comes back", () => {
      vi.useFakeTimers();
      try {
        vi.setSystemTime(1_000);
        for (let i = 0; i < CLOSED_CAP; i++) {
          store.newTab("/k/pods", "prod-eu");
          vi.advanceTimersByTime(10);
          store.closeTab(active().id);
        }
        toStage();
        vi.advanceTimersByTime(10);
        store.openTab("/settings");
        store.closeTab(active().id);

        toProd();
        const closed = store.currentWorkspace().closed;
        expect(closed[0].route).toBe("/settings");
        store.reopenClosed();
        expect(active().route).toBe("/settings");
        expect(active().closedAt).toBeUndefined();
      } finally {
        vi.useRealTimers();
      }
    });

    /**
     * Home offers a closed tab from any cluster and reopens it by focusing
     * its cluster, then asking for it by id. Capping the list as the cluster
     * came back dropped that very tab whenever the shared list was the newer
     * and full, and the click did nothing.
     */
    it("can still reopen a cluster's closed tab after a full list of shared ones was closed elsewhere", () => {
      vi.useFakeTimers();
      try {
        vi.setSystemTime(1_000);
        const services = store.currentWorkspace().tabs.find((t) => t.route === "/k/services")!;
        store.setTabView(services.id, { filter: "api" });
        store.closeTab(services.id);
        toStage();
        for (let i = 0; i < CLOSED_CAP; i++) {
          vi.advanceTimersByTime(10);
          store.newTab("/settings");
          store.closeTab(active().id);
        }
        const offered = store.currentWorkspace().shelved!["id-prod"].closed[0];

        toProd();
        store.reopenClosed(offered.id);
        expect(active().route).toBe("/k/services");
        expect(active().view).toEqual({ filter: "api" });
      } finally {
        vi.useRealTimers();
      }
    });

    /** Home offers closed tabs across clusters, and focuses a tab's cluster to bring it back. */
    it("lists every cluster's recently closed tabs for a surface that offers them all", () => {
      store.closeTab(store.currentWorkspace().tabs.find((t) => t.route === "/k/services")!.id);
      toStage();
      store.openTab("/k/nodes", { clusterName: "staging-eu" });
      store.closeTab(active().id);
      const { result } = renderHook(() => store.useClosedEverywhere());
      expect(result.current.map((t) => [t.route, t.sub])).toEqual([
        ["/k/nodes", "staging-eu"],
        ["/k/services", "prod-eu"],
      ]);
      const before = result.current;
      act(() => toStage());
      expect(result.current).toBe(before);
    });

    it("keeps the app-wide tabs on screen, in their places, whichever cluster is in focus", () => {
      store.openTab("/settings");
      store.moveTab(store.currentWorkspace().tabs.find((t) => t.route === "/settings")!.id, 2);
      expect(routes()).toEqual(["/", "/k/pods", "/settings", "/k/configmaps", "/k/services"]);

      toStage();
      expect(routes()).toEqual(["/", "/settings", "/overview"]);
      expect(subFor("/settings")).toBeUndefined();

      toProd();
      expect(routes()).toEqual(["/", "/k/pods", "/settings", "/k/configmaps", "/k/services"]);
    });

    it("keeps an app-wide tab opened on another cluster when the first comes back", () => {
      toStage();
      store.openTab("/connections");
      toProd();
      expect(routes()).toEqual(["/", "/k/pods", "/k/configmaps", "/k/services", "/connections"]);
    });

    /**
     * The editor names its own cluster in its route and shows it whatever the
     * rail says. Putting a tab away unmounts its screen, which for an editor
     * is somebody's unsaved changes.
     */
    it("leaves an editor on screen, under its own cluster's name, when the focus moves", () => {
      store.openTab("/edit/prod-eu/ConfigMap/default/settings", { clusterName: "prod-eu" });
      store.openTab("/new/prod-eu", { clusterName: "prod-eu" });
      toStage();
      expect(routes()).toEqual(["/", "/edit/prod-eu/ConfigMap/default/settings", "/new/prod-eu", "/overview"]);
      expect(subs()).toEqual([undefined, "prod-eu", "prod-eu", "staging-eu"]);
    });

    /**
     * A detail tab is pinned to its cluster in the route (#834) so that one
     * cluster's object can stay open beside another's. It shows that cluster
     * under that cluster's name whatever the rail does, so it is not put away.
     */
    it("leaves a detail tab on screen, under its own cluster's name, when the focus moves", () => {
      store.openTab("/k/prod-eu/Node/_/worker-1", { clusterName: "prod-eu" });
      toStage();
      store.openTab("/k/staging-eu/Node/_/worker-1", { clusterName: "staging-eu" });
      expect(routes()).toEqual(["/", "/k/prod-eu/Node/_/worker-1", "/overview", "/k/staging-eu/Node/_/worker-1"]);
      expect(subs()).toEqual([undefined, "prod-eu", "staging-eu", "staging-eu"]);
      toProd();
      expect(routes()).toContain("/k/staging-eu/Node/_/worker-1");
      expect(subFor("/k/staging-eu/Node/_/worker-1")).toBe("staging-eu");
    });

    it("puts a cluster's tabs away when no cluster is in focus, and brings them back", () => {
      store.setActiveCluster(null);
      expect(routes()).toEqual(["/"]);
      toProd();
      expect(routes()).toEqual(["/", "/k/pods", "/k/configmaps", "/k/services"]);
    });

    it("gives tabs opened with no cluster in focus to the cluster that takes it", () => {
      store.setState(defaultState([]));
      store.setWorkspaceClusters(store.currentWorkspace().id, []);
      store.openTab("/k/pods");
      const w = store.currentWorkspace();
      store.setState({ ...store.getState(), workspaces: [{ ...w, clusters: ["id-prod", "id-stage"] }] });
      toStage();
      expect(routes()).toEqual(["/", "/k/pods"]);
      expect(subFor("/k/pods")).toBe("staging-eu");
    });

    it("labels the tabs that come back for the cluster's name now, since a context can be renamed while they are away", () => {
      toStage();
      store.setActiveCluster("id-prod", "prod-renamed");
      expect(subs()).toEqual([undefined, "prod-renamed", "prod-renamed", "prod-renamed"]);
    });

    it("never writes a stableId onto the strip", () => {
      toStage();
      toProd();
      expect(store.currentWorkspace().tabs.filter((t) => t.sub === "id-prod" || t.sub === "id-stage")).toEqual([]);
    });

    it("changes nothing, and wakes nobody, when the cluster in focus is chosen again", () => {
      const listener = vi.fn();
      const stop = store.subscribe(listener);
      toProd();
      stop();
      expect(listener).not.toHaveBeenCalled();
    });

    it("drops the tabs of a cluster taken out of the workspace, in focus or not", () => {
      toStage();
      store.openTab("/k/nodes", { clusterName: "staging-eu" });
      const id = store.currentWorkspace().id;
      // Not in focus: its shelved tabs go.
      store.setWorkspaceClusters(id, ["id-stage"]);
      expect(store.currentWorkspace().shelved).toBeUndefined();
      expect(routes()).toEqual(["/", "/overview", "/k/nodes"]);
      // In focus: its tabs on screen go, and nothing of it is shelved.
      store.setWorkspaceClusters(id, []);
      expect(routes()).toEqual(["/"]);
      expect(store.currentWorkspace().shelved).toBeUndefined();
    });

    it("keeps each workspace's clusters' tabs apart from another workspace's", () => {
      const first = store.currentWorkspace().id;
      store.createWorkspace("Other", ["id-prod", "id-stage"]);
      expect(routes()).toEqual(["/"]);
      toStage();
      store.switchWorkspace(first);
      expect(routes()).toEqual(["/", "/k/pods", "/k/configmaps", "/k/services"]);
    });

    it("forgets a removed cluster's namespaces in tabs that are put away, too", () => {
      const pods = store.currentWorkspace().tabs[1].id;
      store.setTabNamespaces(pods, "id-prod", ["payments"]);
      store.setTabNamespaces(pods, "id-gone", ["old"]);
      toStage();
      store.forgetClusterNamespaces("id-gone");
      toProd();
      expect(store.currentWorkspace().tabs[1].namespaces).toEqual({ "id-prod": ["payments"] });
    });
  });

  it("leaves a detail tab labelled with the cluster it is pinned to (#834)", () => {
    // The tab keeps showing prod's worker-1 when the rail moves to staging, so
    // its label must keep saying prod; the list beside it is prod's own and is
    // put away with it (#908), where it used to follow the rail.
    store.setState(defaultState([named("id-prod", "prod-eu"), named("id-stage", "staging-eu")]));
    const node = detailRoute("Node", null, "worker-1", { cluster: "prod-eu" });
    store.openTab(node, { clusterName: "prod-eu" });
    store.openTab("/k/pods", { clusterName: "prod-eu" });

    store.setActiveCluster("id-stage", "staging-eu");

    expect(subFor(node)).toBe("prod-eu");
    expect(routes()).toEqual(["/", node, "/overview"]);
    expect(subFor("/overview")).toBe("staging-eu");
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
    store.setTabView(id, { regex: true });
    expect(store.currentWorkspace().tabs.find((t) => t.id === id)?.view).toEqual({
      filter: "abc",
      sort: { key: "name", direction: "asc" },
      regex: true,
    });
  });

  it("tells the hook when the view changes", () => {
    const id = active().id;
    const { result } = renderHook(() => store.useTabView(id));
    act(() => store.setTabView(id, { filter: "x" }));
    expect(result.current.filter).toBe("x");
  });

  it("does not discard a regex-only view change", () => {
    const id = active().id;
    store.setTabView(id, { regex: true });
    expect(store.currentWorkspace().tabs.find((t) => t.id === id)?.view?.regex).toBe(true);
  });

  it("ignores an id that is not there", () => {
    const before = store.getState();
    store.setTabView("nope", { filter: "x" });
    expect(store.getState()).toBe(before);
  });

  it("holds a Logs tab's query and tells the hook", () => {
    const id = active().id;
    const { result } = renderHook(() => store.useTabView(id));
    act(() => store.setTabView(id, { query: "level:error" }));
    expect(result.current.query).toBe("level:error");
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

  silent("renameWorkspace to a blank name", () => {}, () =>
    store.renameWorkspace(store.getState().currentId, "   "),
  );

  silent("setWorkspaceColor to a colour outside the palette", () => {}, () =>
    store.setWorkspaceColor(store.getState().currentId, "#ff0000"),
  );

  silent(
    "setWorkspaceColor to the colour it already has",
    () => store.setWorkspaceColor(store.getState().currentId, "var(--mark-teal)"),
    () => store.setWorkspaceColor(store.getState().currentId, "var(--mark-teal)"),
  );

  silent("clearing a colour the workspace does not have", () => {}, () =>
    store.setWorkspaceColor(store.getState().currentId, undefined),
  );

  silent(
    "setTabView with values it already holds",
    () => store.setTabView(activeId(), { filter: "abc", sort: { key: "name", direction: "asc" } }),
    () => store.setTabView(activeId(), { filter: "abc", sort: { key: "name", direction: "asc" } }),
  );

  silent(
    "setTabView with a query it already holds",
    () => {
      store.setTabView(activeId(), { query: "level:error" });
      // Without this the test would pass for a store that never kept the query at all.
      expect(store.currentWorkspace().tabs.find((t) => t.id === activeId())?.view?.query).toBe("level:error");
    },
    () => store.setTabView(activeId(), { query: "level:error" }),
  );

  // A Logs tab writes its box on every mount, and an empty box is not a query:
  // a tab that never had one must not gain a view for it.
  silent(
    "setTabView with an empty query on a tab that has none",
    () => {},
    () => store.setTabView(activeId(), { query: "" }),
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

describe("moveTab", () => {
  it("moves pinned tabs without changing selection or other workspace state", () => {
    seed();
    store.openTab("/overview");
    store.openTab("/settings");
    const before = store.currentWorkspace();
    const home = before.tabs[0];
    store.moveTab(home.id, 2);
    const after = store.currentWorkspace();
    expect(after.tabs.map(t => t.id)).toEqual([before.tabs[1].id, before.tabs[2].id, home.id]);
    expect(after.activeId).toBe(before.activeId);
    expect(after.tabs[2]).toEqual(home);
    store.moveTab(home.id, 2);
    expect(store.currentWorkspace()).toBe(after);
    store.openTab("/notes");
    expect(routes().at(-1)).toBe("/notes");
  });
  it("ignores unknown tabs and invalid indices, and clamps to the ends", () => {
    seed(); store.openTab("/overview");
    const before = store.currentWorkspace();
    store.moveTab("missing", 1);
    store.moveTab(before.tabs[0].id, NaN);
    expect(store.currentWorkspace()).toBe(before);
    store.moveTab(before.tabs[0].id, 99);
    expect(store.currentWorkspace().tabs.at(-1)?.id).toBe(before.tabs[0].id);
  });
});

it("round-trips the chosen document order through persisted state", async () => {
  const {parseStoredState, STORAGE_VERSION}=await import("./tabsPersist");
  seed(); store.openTab("/overview");store.openTab("/settings");
  store.moveTab(active().id,0);
  const state=store.getState();
  const restored=parseStoredState(JSON.stringify({version:STORAGE_VERSION,...state}));
  expect(restored?.workspaces[0].tabs.map(t=>t.id)).toEqual(state.workspaces[0].tabs.map(t=>t.id));
  expect(restored?.workspaces[0].activeId).toBe(state.workspaces[0].activeId);
});

describe("namespace collections on a tab (#881)", () => {
  const C = "cluster-id";
  const tabNow = () => store.currentWorkspace().tabs.find((t) => t.id === store.currentWorkspace().activeId)!;

  beforeEach(() => {
    store.openTab("/k/pods");
  });

  it("are kept per cluster, and none means none", () => {
    store.setTabNamespaceCollections(tabNow().id, C, ["prod", "staging"]);
    store.setTabNamespaceCollections(tabNow().id, "other", ["dev"]);
    expect(store.tabNamespaceCollections(tabNow().id, C)).toEqual(["prod", "staging"]);
    store.setTabNamespaceCollections(tabNow().id, C, []);
    expect(store.tabNamespaceCollections(tabNow().id, C)).toBeUndefined();
    expect(tabNow().namespaceCollections).toEqual({ other: ["dev"] });
    store.setTabNamespaceCollections(tabNow().id, "other", []);
    expect(tabNow().namespaceCollections).toBeUndefined();
  });

  it("does not wake anyone when set to what they already are", () => {
    store.setTabNamespaceCollections(tabNow().id, C, ["prod"]);
    const heard = vi.fn();
    const stop = store.subscribe(heard);
    store.setTabNamespaceCollections(tabNow().id, C, ["prod"]);
    stop();
    expect(heard).not.toHaveBeenCalled();
  });

  it("follow a renamed collection, in open and recently closed tabs alike, and no other", () => {
    store.setTabNamespaceCollections(tabNow().id, C, ["prod", "staging"]);
    store.openTab("/k/services");
    store.setTabNamespaceCollections(tabNow().id, C, ["prod"]);
    store.closeTab(tabNow().id);
    store.renameNamespaceCollection("prod", "production");
    const open = store.currentWorkspace().tabs.find((t) => t.route === "/k/pods")!;
    expect(open.namespaceCollections).toEqual({ [C]: ["production", "staging"] });
    expect(store.currentWorkspace().closed[0].namespaceCollections).toEqual({ [C]: ["production"] });
  });

  it("does not wake anyone when no tab names the collection renamed", () => {
    const heard = vi.fn();
    const stop = store.subscribe(heard);
    store.renameNamespaceCollection("nobody", "else");
    stop();
    expect(heard).not.toHaveBeenCalled();
  });

  it("are forgotten with a removed cluster, in open and closed tabs alike", () => {
    store.setTabNamespaceCollections(tabNow().id, C, ["prod"]);
    store.setTabNamespaces(tabNow().id, "other", ["a"]);
    store.forgetClusterNamespaces(C);
    expect(tabNow().namespaceCollections).toBeUndefined();
    expect(tabNow().namespaces).toEqual({ other: ["a"] });
  });
});
