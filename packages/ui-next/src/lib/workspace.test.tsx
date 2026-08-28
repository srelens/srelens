import { describe, it, expect, beforeEach, vi } from "vitest";
import { renderHook, act } from "@testing-library/react";
import * as ws from "./workspace";

function fakeStorage() {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
    m,
  };
}

beforeEach(() => ws.resetView());

describe("workspace view", () => {
  it("starts with no links and nothing expanded", () => {
    expect(ws.getView()).toEqual({ links: {}, expanded: [], namespaces: {} });
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
    ws.toggleExpanded("workloads");
    expect(ws.getView().expanded).toEqual(["workloads"]);
    ws.toggleExpanded("workloads");
    expect(ws.getView().expanded).toEqual([]);
  });

  it("replaces expansion wholesale when told", () => {
    ws.toggleExpanded("a");
    ws.setExpanded(["b", "c"]);
    expect(ws.getView().expanded).toEqual(["b", "c"]);
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
    ws.setExpanded(["a", "b"]);
    let n = 0;
    const off = ws.subscribe(() => n++);
    ws.setExpanded(["a", "b"]);
    ws.setExpanded([...ws.getView().expanded]);
    expect(n).toBe(0);
    ws.setExpanded(["b", "a"]);
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

  it("keeps a namespace selection per cluster, so two clusters do not share one", () => {
    ws.setNamespaces("prod", ["default"]);
    ws.setNamespaces("dev", ["kube-system"]);
    expect(ws.getView().namespaces).toEqual({ prod: ["default"], dev: ["kube-system"] });
  });

  it("reads an unset cluster as all namespaces", () => {
    expect(ws.getView().namespaces["never-set"]).toBeUndefined();
  });

  it("does not notify when the selection is set to what it already is", () => {
    ws.setNamespaces("prod", ["default"]);
    const seen = vi.fn();
    const off = ws.subscribe(seen);
    ws.setNamespaces("prod", ["default"]);
    off();
    expect(seen).not.toHaveBeenCalled();
  });

  it("drops the key entirely when narrowed and then set back to all namespaces", () => {
    ws.setNamespaces("prod", ["default"]);
    ws.setNamespaces("prod", []);
    expect("prod" in ws.getView().namespaces).toBe(false);
  });

  it("does not notify when an already-unset cluster is set to all namespaces", () => {
    const seen = vi.fn();
    const off = ws.subscribe(seen);
    ws.setNamespaces("never-set", []);
    off();
    expect(seen).not.toHaveBeenCalled();
  });
});

describe("persisted namespace selection", () => {
  it("persists a set selection and reads it back after a reload", () => {
    const s = fakeStorage();
    ws.loadNamespaces(s);
    ws.setNamespaces("prod", ["default", "billing"], s);
    expect(JSON.parse(s.m.get(ws.NAMESPACES_KEY)!)).toEqual({ prod: ["default", "billing"] });
    ws.loadNamespaces(fakeStorage()); // forget the in-memory state
    ws.loadNamespaces(s); // and read it back off the same storage
    expect(ws.getView().namespaces.prod).toEqual(["default", "billing"]);
  });

  it("keeps the selection when the cluster is looked up again — the key is the stableId, never a name the store never sees", () => {
    const s = fakeStorage();
    ws.setNamespaces("ctx-1", ["prod"], s);
    ws.loadNamespaces(fakeStorage()); // forget
    ws.loadNamespaces(s); // reload, as a fresh launch would
    // A rename in the kubeconfig cannot touch this: `setNamespaces`/`useNamespaces`
    // take a `stableId` and nothing else, so there is no name for a rename to change.
    expect(ws.getView().namespaces["ctx-1"]).toEqual(["prod"]);
  });

  it("removes the key from storage when cleared, rather than persisting an empty array", () => {
    const s = fakeStorage();
    ws.setNamespaces("prod", ["default"], s);
    ws.setNamespaces("prod", [], s);
    const stored = JSON.parse(s.m.get(ws.NAMESPACES_KEY) ?? "{}");
    expect("prod" in stored).toBe(false);
  });

  it("survives a storage that throws on both read and write, costing only the selection", () => {
    const bad = {
      getItem: () => {
        throw new Error("no reads");
      },
      setItem: () => {
        throw new Error("no writes");
      },
      removeItem: () => {},
    };
    ws.setLink("prod", "connected");
    expect(() => ws.loadNamespaces(bad)).not.toThrow();
    expect(() => ws.setNamespaces("prod", ["default"], bad)).not.toThrow();
    // Only the selection is at the mercy of storage; links stay untouched.
    expect(ws.getView().links.prod).toEqual({ state: "connected" });
  });

  it("loading namespaces leaves links and expanded exactly as they were", () => {
    ws.setLink("prod", "connected");
    ws.toggleExpanded("workloads");
    const s = fakeStorage();
    ws.loadNamespaces(s);
    expect(ws.getView().links.prod).toEqual({ state: "connected" });
    expect(ws.getView().expanded).toEqual(["workloads"]);
  });

  it("reads a document that is not valid JSON, or not a map, as nothing stored", () => {
    for (const raw of ["{oops", "[]", "7", "null", '"hello"']) {
      expect(ws.parseStoredNamespaces(raw)).toEqual({});
    }
    expect(ws.parseStoredNamespaces(null)).toEqual({});
  });

  it("drops one cluster's malformed entry without losing the others", () => {
    const raw = JSON.stringify({ prod: ["default", "billing"], dev: "not-an-array", stage: [1, 2] });
    expect(ws.parseStoredNamespaces(raw)).toEqual({ prod: ["default", "billing"] });
  });

  it("keeps the same array reference across reads until the selection actually changes, so useSyncExternalStore cannot loop", () => {
    const s = fakeStorage();
    ws.loadNamespaces(s);
    const { result, rerender } = renderHook(() => ws.useNamespaces("prod"));
    const first = result.current;
    rerender();
    expect(result.current).toBe(first);
    act(() => ws.setNamespaces("prod", ["default"], s));
    expect(result.current).not.toBe(first);
    expect(result.current).toEqual(["default"]);
  });
});
