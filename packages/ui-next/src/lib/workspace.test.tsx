import { describe, it, expect, beforeEach, vi } from "vitest";
import { renderHook, act } from "@testing-library/react";
import type { ReactNode } from "react";
import * as ws from "./workspace";
import { activateTab, getState, setState, subscribe as subscribeTabs } from "./tabsStore";
import { makeTab } from "./tabs";
import { TabScope } from "./tabScope";
import { settingsStorage } from "@srelens/core";

function fakeStorage() {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
    m,
  };
}

beforeEach(() => { settingsStorage.removeItem("srelens.defaultNamespace"); ws.resetView(); });

describe("workspace view", () => {
  it("starts with no links and nothing expanded", () => {
    expect(ws.getView()).toEqual({ links: {}, expanded: {} });
  });

  it("tells the hook when a link changes", () => {
    const { result } = renderHook(() => ws.useWorkspaceView());
    act(() => ws.setLink("prod", "connected"));
    expect(result.current.links.prod).toEqual({ state: "connected" });
  });

  it("records a link state per cluster, with an error when there is one", () => {
    ws.setLink("a", "connecting");
    ws.setLink("b", "error", "dial tcp: refused");
    expect(ws.getView().links).toEqual({
      a: { state: "connecting" },
      b: { state: "error", error: "dial tcp: refused" },
    });
  });

  it("drops a stale error when the state moves on", () => {
    ws.setLink("a", "error", "x");
    ws.setLink("a", "connected");
    expect(ws.getView().links.a).toEqual({ state: "connected" });
  });

  it("toggles expansion", () => {
    ws.toggleExpanded("prod", "workloads");
    expect(ws.getView().expanded.prod).toEqual(["workloads"]);
    ws.toggleExpanded("prod", "workloads");
    expect(ws.getView().expanded.prod).toEqual([]);
  });

  it("replaces expansion wholesale when told", () => {
    ws.toggleExpanded("prod", "a");
    ws.setExpanded("prod", ["b", "c"]);
    expect(ws.getView().expanded.prod).toEqual(["b", "c"]);
  });

  it("does not notify for a no-op", () => {
    let n = 0;
    const { result } = renderHook(() => ws.useWorkspaceView());
    void result;
    const off = ws.subscribe(() => n++);
    ws.setLink("a", "connected");
    ws.setLink("a", "connected");
    expect(n).toBe(1);
    off();
  });

  it("does not notify when setExpanded is handed the same list again", () => {
    ws.setExpanded("prod", ["a", "b"]);
    let n = 0;
    const off = ws.subscribe(() => n++);
    ws.setExpanded("prod", ["a", "b"]);
    ws.setExpanded("prod", [...ws.getView().expanded.prod]);
    expect(n).toBe(0);
    ws.setExpanded("prod", ["b", "a"]);
    expect(n).toBe(1);
    off();
  });

  it("does not notify when resetView is called on an already-initial view", () => {
    let n = 0;
    const off = ws.subscribe(() => n++);
    ws.resetView();
    expect(n).toBe(0);
    ws.setLink("prod", "connected");
    ws.resetView();
    expect(n).toBe(2);
    off();
  });

});

function twoTabs() {
  const a = { ...makeTab("/pods"), id: "tab-a" };
  const b = { ...makeTab("/deployments"), id: "tab-b" };
  setState({
    workspaces: [{ id: "w", name: "W", clusters: ["prod", "dev"], activeCluster: "prod", tabs: [a, b], activeId: "tab-a", closed: [] }],
    currentId: "w",
  });
}

const tab = (id: string) => getState().workspaces[0].tabs.find((t) => t.id === id)!;
const inTab = (tabId: string) => ({ children }: { children: ReactNode }) => <TabScope.Provider value={tabId}>{children}</TabScope.Provider>;

describe("namespace selection", () => {
  beforeEach(twoTabs);

  it("belongs to the tab it was made in — narrowing one tab leaves another tab on the same cluster alone", () => {
    const a = renderHook(() => ws.useNamespaces("prod"), { wrapper: inTab("tab-a") });
    const b = renderHook(() => ws.useNamespaces("prod"), { wrapper: inTab("tab-b") });
    const setA = renderHook(() => ws.useSetNamespaces(), { wrapper: inTab("tab-a") }).result.current;
    act(() => setA("prod", ["payments"]));
    expect(a.result.current).toEqual(["payments"]);
    expect(b.result.current).toEqual([]);
  });

  it("is kept per cluster within a tab, so the rail switching cluster does not carry one cluster's namespaces to another", () => {
    ws.setNamespaces("prod", ["default"], "tab-a");
    ws.setNamespaces("dev", ["kube-system"], "tab-a");
    expect(tab("tab-a").namespaces).toEqual({ prod: ["default"], dev: ["kube-system"] });
    expect(tab("tab-b").namespaces).toBeUndefined();
  });

  it("is written on the tab, so it persists and restores with the tab", () => {
    ws.setNamespaces("prod", ["default", "billing"], "tab-b");
    expect(tab("tab-b").namespaces).toEqual({ prod: ["default", "billing"] });
    expect(tab("tab-a").namespaces).toBeUndefined();
  });

  it("falls back to the active tab outside any tab — the dock asks about what is on screen", () => {
    ws.setNamespaces("prod", ["billing"], "tab-a");
    ws.setNamespaces("prod", ["shop"], "tab-b");
    const { result } = renderHook(() => ws.useNamespaces("prod"));
    expect(result.current).toEqual(["billing"]);
    act(() => activateTab("tab-b"));
    expect(result.current).toEqual(["shop"]);
  });

  it("does not notify when the selection is set to what it already is", () => {
    ws.setNamespaces("prod", ["default"], "tab-a");
    const seen = vi.fn();
    const off = subscribeTabs(seen);
    ws.setNamespaces("prod", ["default"], "tab-a");
    off();
    expect(seen).not.toHaveBeenCalled();
  });

  it("keeps an explicit all-namespaces choice when a selection is cleared", () => {
    ws.setNamespaces("prod", ["default"], "tab-a");
    ws.setNamespaces("prod", [], "tab-a");
    expect(tab("tab-a").namespaces).toEqual({ prod: [] });
  });

  it("forgets a removed cluster's selection in every tab without disturbing other clusters", () => {
    ws.setNamespaces("prod", ["default"], "tab-a");
    ws.setNamespaces("dev", ["kube-system"], "tab-a");
    ws.setNamespaces("prod", ["billing"], "tab-b");
    ws.removeNamespaces("prod");
    expect(tab("tab-a").namespaces).toEqual({ dev: ["kube-system"] });
    expect(tab("tab-b").namespaces).toEqual({});
  });

  it("keeps the same array reference across reads until the selection actually changes, so useSyncExternalStore cannot loop", () => {
    const { result, rerender } = renderHook(() => ws.useNamespaces("prod"), { wrapper: inTab("tab-a") });
    const first = result.current;
    rerender();
    expect(result.current).toBe(first);
    act(() => ws.setNamespaces("prod", ["default"], "tab-a"));
    expect(result.current).not.toBe(first);
    expect(result.current).toEqual(["default"]);
  });

  it("uses the default only where the tab has no choice for the cluster, and keeps an explicit all", () => {
    const { result } = renderHook(() => ws.useNamespaces("prod"), { wrapper: inTab("tab-a") });
    act(() => ws.setNamespaceDefault("team"));
    expect(result.current).toEqual(["team"]);
    act(() => ws.setNamespaces("prod", [], "tab-a"));
    expect(result.current).toEqual([]);
    act(() => ws.setNamespaceDefault("other"));
    expect(result.current).toEqual([]);
    const other = renderHook(() => ws.useNamespaces("prod"), { wrapper: inTab("tab-b") });
    expect(other.result.current).toEqual(["other"]);
  });
});

describe("persisted sidebar groups", () => {
  it("saves through the settings adapter and restores independent cluster choices", () => {
    const s = fakeStorage();
    const save = vi.spyOn(settingsStorage, "setItem").mockImplementation(s.setItem);
    ws.toggleExpanded("stable-prod", "workloads");
    ws.toggleExpanded("stable-stage", "network");
    expect(save).toHaveBeenLastCalledWith(ws.EXPANDED_KEY, JSON.stringify({
      "stable-prod": ["workloads"], "stable-stage": ["network"],
    }));
    ws.resetView();
    ws.loadExpanded(s);
    expect(ws.getView().expanded).toEqual({ "stable-prod": ["workloads"], "stable-stage": ["network"] });
    save.mockRestore();
  });

  it("drops malformed entries without losing other clusters or live connection state", () => {
    const s = fakeStorage();
    s.setItem(ws.EXPANDED_KEY, JSON.stringify({ good: ["workloads"], bad: true, mixed: [3], closed: [] }));
    ws.setLink("good", "connected");
    ws.loadExpanded(s);
    expect(ws.getView().expanded).toEqual({ good: ["workloads"], closed: [] });
    expect(ws.getView().links.good.state).toBe("connected");
    s.setItem(ws.EXPANDED_KEY, "invalid json");
    ws.loadExpanded(s);
    expect(ws.getView().expanded).toEqual({});
  });

  it("still allows navigation when storage is unavailable", () => {
    const bad = { getItem: () => { throw Error("offline"); }, setItem: () => { throw Error("offline"); }, removeItem: () => {} };
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    expect(() => ws.loadExpanded(bad)).not.toThrow();
    expect(() => ws.toggleExpanded("prod", "workloads", bad)).not.toThrow();
    expect(ws.getView().expanded.prod).toEqual(["workloads"]);
    error.mockRestore();
  });
});
