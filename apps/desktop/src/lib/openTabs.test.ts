import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { CrdRef } from "./crds";
import {
  loadOpenTabs,
  saveOpenTabs,
  nextTabId,
  pruneMissingContexts,
  reconcileActiveTab,
  reconcileCrdTabs,
} from "./openTabs";

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

  // #159 changed this deliberately: the desktop used to be a no-op and start
  // blank every launch. It now persists through the same settingsStorage
  // path as the web, which is file-backed once the durable store loads.
  it("persists and restores like the web does", () => {
    setDesktop(true);
    saveOpenTabs([tab({ id: 1 }), tab({ id: 2, kind: "services" })], 2);
    const restored = loadOpenTabs();
    expect(restored?.tabs.map((t) => t.id)).toEqual([1, 2]);
    expect(restored?.activeTabId).toBe(2);
  });
});

describe("session restore opt-out", () => {
  afterEach(() => {
    localStorage.clear();
  });

  it("starts fresh when disabled, without discarding the stored session", () => {
    saveOpenTabs([tab({ id: 4 })], 4);
    expect(loadOpenTabs()?.tabs).toHaveLength(1);

    localStorage.setItem("srelens.restoreSession", "false");
    expect(loadOpenTabs()).toBeNull();
    // A later save must not clobber the snapshot either, so re-enabling the
    // setting brings back the real session rather than an empty one.
    saveOpenTabs([tab({ id: 5 })], 5);
    localStorage.setItem("srelens.restoreSession", "true");
    expect(loadOpenTabs()?.tabs.map((t) => t.id)).toEqual([4]);
  });

  it("is on unless explicitly disabled", () => {
    saveOpenTabs([tab({ id: 6 })], 6);
    localStorage.setItem("srelens.restoreSession", "garbage");
    expect(loadOpenTabs()?.tabs).toHaveLength(1);
  });
});

describe("pruneMissingContexts", () => {
  it("drops tabs whose cluster is gone and reports how many", () => {
    const result = pruneMissingContexts(
      [tab({ id: 1, cluster: "prod" }), tab({ id: 2, cluster: "retired" })],
      ["prod"],
    );
    expect(result.tabs.map((t) => t.id)).toEqual([1]);
    expect(result.dropped).toBe(1);
  });

  it("keeps cluster-less tabs, which do not depend on a context", () => {
    // Settings/Toolbox/landing tabs must survive a context disappearing.
    const result = pruneMissingContexts(
      [tab({ id: 1, cluster: null }), tab({ id: 2, cluster: "gone" })],
      [],
    );
    expect(result.tabs.map((t) => t.id)).toEqual([1]);
    expect(result.dropped).toBe(1);
  });

  it("reports nothing dropped when every context is still present", () => {
    const tabs = [tab({ id: 1, cluster: "a" }), tab({ id: 2, cluster: "b" })];
    expect(pruneMissingContexts(tabs, ["a", "b"])).toEqual({ tabs, dropped: 0 });
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

describe("reconcileActiveTab", () => {
  it("keeps the active tab when it survived the prune", () => {
    expect(reconcileActiveTab([tab({ id: 1 }), tab({ id: 2 })], 2)).toBe(2);
  });

  it("falls back to the first survivor when the active tab was pruned", () => {
    // Otherwise the workspace renders blank behind a populated tab strip.
    expect(reconcileActiveTab([tab({ id: 3 }), tab({ id: 4 })], 99)).toBe(3);
  });

  it("clears the active id when nothing survives", () => {
    // A stale id here also costs the native close command its no-tabs path.
    expect(reconcileActiveTab([], 7)).toBeNull();
  });
});

describe("reconcileCrdTabs", () => {
  const crd = (over: Partial<CrdRef> = {}): CrdRef => ({
    name: "widgets.example.com",
    group: "example.com",
    version: "v1",
    kind: "Widget",
    plural: "widgets",
    namespaced: true,
    ...over,
  });
  const crdTab = (id: number, over: Partial<CrdRef> = {}) =>
    ({ ...tab({ id, cluster: "prod" }), crd: crd(over) }) as Tab;

  it("drops a tab whose CRD is no longer installed", () => {
    const result = reconcileCrdTabs([crdTab(1)], "prod", []);
    expect(result.tabs).toHaveLength(0);
    expect(result.dropped).toBe(1);
  });

  it("adopts the currently served version instead of dropping the tab", () => {
    // The CRD still exists, just at v2 — keeping the stale v1 ref would query
    // a version the cluster no longer serves.
    const result = reconcileCrdTabs([crdTab(1)], "prod", [crd({ version: "v2" })]);
    expect(result.dropped).toBe(0);
    expect(result.tabs[0].crd?.version).toBe("v2");
  });

  it("leaves tabs from other contexts and non-CRD tabs untouched", () => {
    const other = { ...tab({ id: 2, cluster: "staging" }), crd: crd() } as Tab;
    const plain = tab({ id: 3, cluster: "prod" });
    const result = reconcileCrdTabs([other, plain], "prod", []);
    expect(result.tabs.map((t) => t.id)).toEqual([2, 3]);
    expect(result.dropped).toBe(0);
  });

  it("returns the same tab objects when nothing changed", () => {
    const tabs = [crdTab(1)];
    const result = reconcileCrdTabs(tabs, "prod", [crd()]);
    expect(result.tabs[0]).toBe(tabs[0]);
    expect(result.dropped).toBe(0);
  });
});
