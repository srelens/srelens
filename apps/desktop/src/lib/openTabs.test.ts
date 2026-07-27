import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { loadOpenTabs, saveOpenTabs, nextTabId } from "./openTabs";

// ViewTab is a type; tests build plain objects matching its shape.
type Tab = Parameters<typeof saveOpenTabs>[0][number];

const KEY = "srelens.openTabs";

function tab(over: Partial<Tab> & { id: number }): Tab {
  return { cluster: "internal", kind: "pods", ...over } as Tab;
}

function setDesktop(on: boolean) {
  if (on) {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  } else {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  }
}

describe("openTabs persistence (web mode)", () => {
  beforeEach(() => {
    setDesktop(false);
    localStorage.clear();
  });
  afterEach(() => {
    setDesktop(false);
    localStorage.clear();
  });

  it("round-trips open tabs and the active tab", () => {
    const tabs = [tab({ id: 1, kind: "pods" }), tab({ id: 2, kind: "deployments" })];
    saveOpenTabs(tabs, 2);
    const restored = loadOpenTabs();
    expect(restored).not.toBeNull();
    expect(restored!.tabs.map((t) => t.id)).toEqual([1, 2]);
    expect(restored!.tabs.map((t) => t.kind)).toEqual(["pods", "deployments"]);
    expect(restored!.activeTabId).toBe(2);
  });

  it("strips deep-link focus so a reload doesn't re-trigger it", () => {
    const tabs = [
      tab({ id: 1, focus: { name: "web", namespace: "default", nonce: 7 } }),
    ];
    saveOpenTabs(tabs, 1);
    const restored = loadOpenTabs();
    expect(restored!.tabs[0].focus).toBeUndefined();
  });

  it("excludes transient new/edit editor tabs", () => {
    const tabs = [
      tab({ id: 1, kind: "pods" }),
      tab({ id: 2, kind: "newresource", create: { initialKind: "Pod" } }),
      tab({ id: 3, kind: "editresource", edit: { kind: "Pod", namespace: "default", name: "x" } }),
    ];
    saveOpenTabs(tabs, 2);
    const restored = loadOpenTabs();
    expect(restored!.tabs.map((t) => t.id)).toEqual([1]);
    // active pointed at an excluded tab → falls back to the first survivor.
    expect(restored!.activeTabId).toBe(1);
  });

  it("falls back to the first tab when the stored active id is gone", () => {
    saveOpenTabs([tab({ id: 5 }), tab({ id: 6 })], 999);
    expect(loadOpenTabs()!.activeTabId).toBe(5);
  });

  it("clears storage and returns null when nothing restorable remains", () => {
    saveOpenTabs([tab({ id: 1, kind: "newresource", create: {} })], 1);
    expect(localStorage.getItem(KEY)).toBeNull();
    expect(loadOpenTabs()).toBeNull();
  });

  it("returns null on empty or malformed storage without throwing", () => {
    expect(loadOpenTabs()).toBeNull();
    localStorage.setItem(KEY, "{not json");
    expect(loadOpenTabs()).toBeNull();
    localStorage.setItem(KEY, JSON.stringify({ tabs: "nope" }));
    expect(loadOpenTabs()).toBeNull();
  });

  it("drops malformed tab entries on load", () => {
    localStorage.setItem(
      KEY,
      JSON.stringify({ tabs: [{ id: 1, kind: "pods" }, { id: "x" }, null], activeTabId: 1 }),
    );
    const restored = loadOpenTabs();
    expect(restored!.tabs.map((t) => t.id)).toEqual([1]);
  });
});

describe("openTabs on desktop", () => {
  afterEach(() => {
    setDesktop(false);
    localStorage.clear();
  });

  it("never reads or writes localStorage", () => {
    setDesktop(true);
    saveOpenTabs([tab({ id: 1 })], 1);
    expect(localStorage.getItem(KEY)).toBeNull();
    // Even with data present, desktop restore is a no-op.
    localStorage.setItem(KEY, JSON.stringify({ tabs: [tab({ id: 9 })], activeTabId: 9 }));
    expect(loadOpenTabs()).toBeNull();
  });
});

describe("nextTabId", () => {
  it("is one past the highest restored id", () => {
    expect(nextTabId([tab({ id: 3 }), tab({ id: 7 }), tab({ id: 2 })])).toBe(8);
  });
  it("is 1 for an empty set", () => {
    expect(nextTabId([])).toBe(1);
  });
});
