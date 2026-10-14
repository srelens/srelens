// jsdom, not node: `./tabs` imports `./routes`, whose screen table reaches
// `@xterm/addon-fit` — a UMD bundle that reads `self` as it evaluates.
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { ClusterContext } from "@srelens/core";
import { defaultState, makeTab, type TabsState } from "./tabs";
import {
  BASE_STORAGE_KEY, STORAGE_VERSION, flushSave, getStorageKey, loadTabsState, parseStoredState, saveTabsState, scheduleSave,
  installFlushOnUnload, type Storage,
} from "./tabsPersist";

function memory(): Storage & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return {
    data,
    getItem: (k) => data.get(k) ?? null,
    setItem: (k, v) => void data.set(k, v),
    removeItem: (k) => void data.delete(k),
  };
}

const ctx = (id: string): ClusterContext => ({
  name: id, stableId: id, key: id, cluster: id, server: `https://${id}`, isCurrent: false,
  sourceFile: "/home/dana/.kube/config", authKind: "client certificate",
});

const valid = (): TabsState => {
  const s = defaultState([]);
  s.workspaces[0].tabs.push(makeTab("/k/pods", { clusterName: "c" }));
  return s;
};

beforeEach(() => vi.useFakeTimers());
afterEach(() => { flushSave(); vi.useRealTimers(); });

const throwing = (which: "getItem" | "setItem"): Storage => ({
  getItem: () => { if (which === "getItem") throw new DOMException("denied", "SecurityError"); return null; },
  setItem: () => { if (which === "setItem") throw new DOMException("denied", "SecurityError"); },
  removeItem: () => {},
});

describe("storage that refuses", () => {
  // `settingsStorage` falls back to raw `localStorage` when the backend file
  // is unavailable, and `localStorage` throws outright in a WebView with
  // storage disabled. Every sibling helper in core wraps its access; these did
  // not, so a throwing read rejected the Window's boot and left the spinner up
  // forever, and a throwing write escaped a `setTimeout` and the
  // `beforeunload` listener.
  it("reads nothing rather than throwing when getItem refuses", () => {
    expect(loadTabsState(throwing("getItem"), () => true)).toBeNull();
  });

  it("drops the write rather than throwing when setItem refuses", () => {
    expect(() => saveTabsState(valid(), throwing("setItem"))).not.toThrow();
  });

  it("does not let a refused write escape the debounce", () => {
    scheduleSave(valid(), throwing("setItem"));
    expect(() => vi.runAllTimers()).not.toThrow();
  });
});

describe("parseStoredState", () => {
  it("restores legacy Control room tabs as app-wide Home without changing identity or saved work", () => {
    const state = valid();
    const home = state.workspaces[0].tabs[0];
    home.title = "Control room";
    home.sub = "old-prod";
    const parsed = parseStoredState(JSON.stringify({ version: STORAGE_VERSION, ...state }))!;
    expect(parsed.workspaces[0].tabs[0]).toEqual({ id: home.id, route: "/", title: "Home", kind: "control", pinned: true });
    expect(parsed.workspaces[0].tabs[1]).toEqual(state.workspaces[0].tabs[1]);
    expect(parsed.workspaces[0].activeId).toBe(state.workspaces[0].activeId);
  });

  it("round-trips a state written by saveTabsState", () => {
    const storage = memory();
    const state = valid();
    saveTabsState(state, storage);
    expect(parseStoredState(storage.getItem(BASE_STORAGE_KEY))).toEqual(state);
  });

  /**
   * #834. An earlier build's detail route had no cluster in it, and the tab
   * showed whichever cluster the rail was on — the cluster its label was last
   * relabelled for, which is what was saved as `sub`. Pinning it to that
   * cluster restores the tab showing what it showed, and makes it the same
   * tab a fresh open of that resource on that cluster focuses.
   */
  it("pins an earlier build's detail tab to the cluster it was labelled with", () => {
    const state = valid();
    state.workspaces[0].tabs.push({ ...makeTab("/k/Node/-/worker-1"), sub: "prod" });
    state.workspaces[0].closed.push({ ...makeTab("/k/Pod/default/web"), sub: "staging" });
    const parsed = parseStoredState(JSON.stringify({ version: STORAGE_VERSION, ...state }))!;
    expect(parsed.workspaces[0].tabs[2]).toMatchObject({ route: "/k/prod/Node/-/worker-1", sub: "prod" });
    expect(parsed.workspaces[0].closed[0]).toMatchObject({ route: "/k/staging/Pod/default/web", sub: "staging" });
  });

  it("keeps an earlier build's detail tab with no label as it was, following the rail", () => {
    const state = valid();
    state.workspaces[0].tabs.push(makeTab("/k/Node/-/worker-1"));
    const parsed = parseStoredState(JSON.stringify({ version: STORAGE_VERSION, ...state }))!;
    expect(parsed.workspaces[0].tabs[2].route).toBe("/k/Node/-/worker-1");
  });

  it("never re-pins a detail tab that already names its cluster, whatever its label says", () => {
    // The route is the tab's identity; a stale label is the thing to fix, not it.
    const state = valid();
    state.workspaces[0].tabs.push({ ...makeTab("/k/prod/Node/-/worker-1"), sub: "staging" });
    const parsed = parseStoredState(JSON.stringify({ version: STORAGE_VERSION, ...state }))!;
    expect(parsed.workspaces[0].tabs[2].route).toBe("/k/prod/Node/-/worker-1");
  });

  it("returns null for nothing, for garbage, and for the wrong shape", () => {
    expect(parseStoredState(null)).toBeNull();
    expect(parseStoredState("not json")).toBeNull();
    expect(parseStoredState(JSON.stringify({ version: STORAGE_VERSION, workspaces: "nope" }))).toBeNull();
    expect(parseStoredState(JSON.stringify({ version: STORAGE_VERSION, workspaces: [], currentId: 1 }))).toBeNull();
  });

  it("refuses a document from a future version rather than half-applying it", () => {
    const storage = memory();
    saveTabsState(valid(), storage);
    const doc = JSON.parse(storage.getItem(BASE_STORAGE_KEY)!);
    doc.version = STORAGE_VERSION + 1;
    expect(parseStoredState(JSON.stringify(doc))).toBeNull();
  });

  it("accepts a document from an older version, so a bump can migrate it", () => {
    // `version !== STORAGE_VERSION` refused older documents too, so the first
    // bump to 2 would have silently discarded every user's workspaces.
    const storage = memory();
    const state = valid();
    saveTabsState(state, storage);
    const doc = JSON.parse(storage.getItem(BASE_STORAGE_KEY)!);
    doc.version = 0;
    expect(parseStoredState(JSON.stringify(doc))).toEqual(state);
  });

  it("refuses a document with no version at all", () => {
    const storage = memory();
    saveTabsState(valid(), storage);
    const doc = JSON.parse(storage.getItem(BASE_STORAGE_KEY)!);
    delete doc.version;
    expect(parseStoredState(JSON.stringify(doc))).toBeNull();
    expect(parseStoredState(JSON.stringify({ ...doc, version: "1" }))).toBeNull();
  });

  it("round-trips activeCluster and drops one that is not a string", () => {
    const s = defaultState([ctx("a")]);
    s.workspaces[0].activeCluster = "a";
    expect(parseStoredState(JSON.stringify({ version: 1, ...s }))?.workspaces[0].activeCluster).toBe("a");
    const raw = JSON.stringify({ version: 1, ...s }).replace('"activeCluster":"a"', '"activeCluster":7');
    expect(parseStoredState(raw)?.workspaces[0].activeCluster).toBeUndefined();
  });

  it("drops an activeCluster that names a cluster the workspace does not have", () => {
    const s = defaultState([ctx("a")]);
    s.workspaces[0].activeCluster = "a";
    const raw = JSON.stringify({ version: 1, ...s }).replace('"activeCluster":"a"', '"activeCluster":"gone"');
    expect(parseStoredState(raw)?.workspaces[0].activeCluster).toBeUndefined();
  });

  it("round-trips a workspace's colour", () => {
    const s = valid();
    s.workspaces[0].color = "var(--mark-teal)";
    expect(parseStoredState(JSON.stringify({ version: 1, ...s }))?.workspaces[0].color).toBe("var(--mark-teal)");
  });

  it("drops a colour that is not in the mark palette", () => {
    for (const junk of ['"#ff0000"', "7", '"var(--accent)"']) {
      const s = valid();
      s.workspaces[0].color = "var(--mark-teal)";
      const raw = JSON.stringify({ version: 1, ...s }).replace('"color":"var(--mark-teal)"', `"color":${junk}`);
      expect(parseStoredState(raw)?.workspaces[0], junk).not.toHaveProperty("color");
    }
  });

  it("reads a workspace saved before colours existed with no colour", () => {
    expect(parseStoredState(JSON.stringify({ version: 1, ...valid() }))?.workspaces[0]).not.toHaveProperty("color");
  });

  /** The tabs of the clusters not in focus (#908). */
  describe("the strips put away", () => {
    const shelving = (): TabsState => {
      const s = defaultState([ctx("a"), ctx("b")]);
      const w = s.workspaces[0];
      const services = makeTab("/k/services", { clusterName: "b" });
      services.view = { filter: "api" };
      services.namespaces = { b: ["payments"] };
      w.shelved = {
        b: { tabs: [services], order: [w.tabs[0].id, services.id], activeId: services.id, closed: [makeTab("/k/pods", { clusterName: "b" })] },
      };
      return s;
    };

    it("keeps when a closed tab was closed, and drops a time that is not a number", () => {
      const s = shelving();
      s.workspaces[0].shelved!.b.closed[0].closedAt = 1_700_000_000_000;
      const doc = JSON.parse(JSON.stringify({ version: 1, ...s }));
      expect(parseStoredState(JSON.stringify(doc))!.workspaces[0].shelved!.b.closed[0].closedAt).toBe(1_700_000_000_000);
      doc.workspaces[0].shelved.b.closed[0].closedAt = "yesterday";
      expect(parseStoredState(JSON.stringify(doc))!.workspaces[0].shelved!.b.closed[0].closedAt).toBeUndefined();
    });

    it("round-trips them: tabs, order, active tab and recently closed", () => {
      const s = shelving();
      expect(parseStoredState(JSON.stringify({ version: 1, ...s }))).toEqual(s);
    });

    it("accepts a document written before clusters had tabs of their own", () => {
      const s = defaultState([ctx("a"), ctx("b")]);
      expect(parseStoredState(JSON.stringify({ version: 1, ...s }))?.workspaces[0].shelved).toBeUndefined();
    });

    it("drops a strip for a cluster the workspace does not have, and one for the cluster in focus", () => {
      const s = shelving();
      const strip = s.workspaces[0].shelved!.b;
      s.workspaces[0].shelved = { gone: strip, a: strip, b: strip };
      expect(Object.keys(parseStoredState(JSON.stringify({ version: 1, ...s }))!.workspaces[0].shelved!)).toEqual(["b"]);
    });

    it("loses one cluster's unreadable strip, not the workspace or the other clusters' tabs", () => {
      const s = shelving();
      s.workspaces[0].clusters.push("c");
      const doc = JSON.parse(JSON.stringify({ version: 1, ...s }));
      doc.workspaces[0].shelved.c = "not a strip";
      const out = parseStoredState(JSON.stringify(doc))!.workspaces[0];
      expect(Object.keys(out.shelved!)).toEqual(["b"]);
      expect(out.tabs).toHaveLength(1);
    });

    it("drops a malformed tab from a strip, and reads a strip with no order or active tab", () => {
      const s = shelving();
      const doc = JSON.parse(JSON.stringify({ version: 1, ...s }));
      const strip = doc.workspaces[0].shelved.b;
      strip.tabs.push({ id: 3 });
      delete strip.order;
      delete strip.activeId;
      const out = parseStoredState(JSON.stringify(doc))!.workspaces[0].shelved!.b;
      expect(out.tabs).toHaveLength(1);
      expect(out.order).toEqual([out.tabs[0].id]);
      expect(out.activeId).toBe(out.tabs[0].id);
    });

    it("does not keep a strip with nothing in it", () => {
      const s = shelving();
      s.workspaces[0].shelved = { b: { tabs: [], order: [], activeId: "", closed: [] } };
      expect(parseStoredState(JSON.stringify({ version: 1, ...s }))?.workspaces[0].shelved).toBeUndefined();
    });
  });

  it("restores pauses only for clusters that remain in the workspace", () => {
    const s = defaultState([ctx("a"), ctx("b")]);
    s.workspaces[0].pausedClusters = ["a"];
    const raw = JSON.stringify({ version: STORAGE_VERSION, ...s }).replace('["a"]', '["a","gone",7]');
    expect(parseStoredState(raw)?.workspaces[0].pausedClusters).toEqual(["a"]);
  });

  it("keeps a tab's sort through a save and a load", () => {
    const s = valid();
    s.workspaces[0].tabs[1].view = {
      sort: { key: "restarts", direction: "desc" },
      filter: "crash",
      filterKey: "status",
      regex: true,
    };
    const storage = memory();
    saveTabsState(s, storage);
    const parsed = parseStoredState(storage.getItem(BASE_STORAGE_KEY));
    expect(parsed!.workspaces[0].tabs[1].view).toEqual({
      sort: { key: "restarts", direction: "desc" },
      filter: "crash",
      filterKey: "status",
      regex: true,
    });
  });

  it("keeps a Logs tab's query through a save and a load, up to 2000 characters", () => {
    for (const query of ["level:error -pool", "", "a".repeat(2000)]) {
      const s = valid();
      s.workspaces[0].tabs[1].view = { query };
      const storage = memory();
      saveTabsState(s, storage);
      const parsed = parseStoredState(storage.getItem(BASE_STORAGE_KEY));
      expect(parsed!.workspaces[0].tabs[1].view).toEqual({ query });
    }
  });

  it.each([
    ["is not a string", 5],
    ["is longer than 2000 characters", "a".repeat(2001)],
  ])("drops a query that %s, keeping the rest of the view", (_why, query) => {
    const s = valid();
    s.workspaces[0].tabs[1].view = { filter: "crash" };
    const doc = JSON.parse(JSON.stringify({ version: 1, ...s }));
    doc.workspaces[0].tabs[1].view.query = query;
    expect(parseStoredState(JSON.stringify(doc))!.workspaces[0].tabs[1].view).toEqual({ filter: "crash" });
  });

  it("drops a non-boolean regex mode without losing the rest of the view", () => {
    const s = valid();
    s.workspaces[0].tabs[1].view = { filter: "crash" };
    const doc = JSON.parse(JSON.stringify({ version: 1, ...s }));
    doc.workspaces[0].tabs[1].view.regex = "yes";
    expect(parseStoredState(JSON.stringify(doc))!.workspaces[0].tabs[1].view).toEqual({
      filter: "crash",
    });
  });

  it("accepts a stored tab written before this field existed", () => {
    const doc = {
      version: 1,
      currentId: "w",
      workspaces: [{
        id: "w", name: "N", clusters: [], activeId: "t1", closed: [],
        tabs: [{ id: "t1", route: "/k/pods", title: "Pods", kind: "workloads" }],
      }],
    };
    const parsed = parseStoredState(JSON.stringify(doc));
    expect(parsed!.workspaces[0].tabs[0].view).toBeUndefined();
  });

  it("drops a view that is not the shape this build reads, keeping the tab", () => {
    const doc = {
      version: 1,
      currentId: "w",
      workspaces: [{
        id: "w", name: "N", clusters: [], activeId: "t1", closed: [],
        tabs: [{ id: "t1", route: "/k/pods", title: "Pods", kind: "workloads", view: "sorted-by-name" }],
      }],
    };
    const parsed = parseStoredState(JSON.stringify(doc));
    expect(parsed!.workspaces[0].tabs[0].view).toBeUndefined();
    expect(parsed!.workspaces[0].tabs[0].route).toBe("/k/pods");
  });

  it("drops a sort that is not the shape this build reads, keeping the rest of the view", () => {
    const doc = {
      version: 1,
      currentId: "w",
      workspaces: [{
        id: "w", name: "N", clusters: [], activeId: "t1", closed: [],
        tabs: [{
          id: "t1", route: "/k/pods", title: "Pods", kind: "workloads",
          view: { sort: "by-name", filter: "crash" },
        }],
      }],
    };
    const parsed = parseStoredState(JSON.stringify(doc));
    expect(parsed!.workspaces[0].tabs[0].view).toEqual({ filter: "crash" });
  });

  it("round-trips a tab's per-cluster namespace selection, including an explicit all", () => {
    const s = valid();
    s.workspaces[0].tabs[1].namespaces = { prod: ["billing", "shop"], dev: [] };
    const storage = memory();
    saveTabsState(s, storage);
    const parsed = parseStoredState(storage.getItem(BASE_STORAGE_KEY));
    expect(parsed!.workspaces[0].tabs[1].namespaces).toEqual({ prod: ["billing", "shop"], dev: [] });
    expect(parsed!.workspaces[0].tabs[0].namespaces).toBeUndefined();
  });

  it("drops one cluster's malformed namespace entry without losing the others or the tab", () => {
    const doc = {
      version: 1,
      currentId: "w",
      workspaces: [{
        id: "w", name: "N", clusters: [], activeId: "t1", closed: [],
        tabs: [{ id: "t1", route: "/k/pods", title: "Pods", kind: "workloads", namespaces: { prod: ["a"], dev: "b", stage: [1] } }],
      }],
    };
    const parsed = parseStoredState(JSON.stringify(doc));
    expect(parsed!.workspaces[0].tabs[0].namespaces).toEqual({ prod: ["a"] });
  });

  it("round-trips a null sort and a null filterKey", () => {
    const s = valid();
    s.workspaces[0].tabs[1].view = { sort: null, filterKey: null };
    const storage = memory();
    saveTabsState(s, storage);
    const parsed = parseStoredState(storage.getItem(BASE_STORAGE_KEY));
    expect(parsed!.workspaces[0].tabs[1].view).toEqual({ sort: null, filterKey: null });
  });

  it("drops fields it does not know and tabs that are malformed", () => {
    const doc = {
      version: STORAGE_VERSION,
      currentId: "w",
      stray: true,
      workspaces: [{
        id: "w", name: "N", clusters: ["a"], activeId: "t1", closed: [], extra: 1,
        tabs: [
          { id: "t1", route: "/", title: "Home", kind: "control", pinned: true, junk: "x" },
          { id: 7, route: "/bad" },
          "nope",
        ],
      }],
    };
    const out = parseStoredState(JSON.stringify(doc))!;
    expect(out.workspaces[0].tabs).toEqual([{ id: "t1", route: "/", title: "Home", kind: "control", pinned: true }]);
    expect((out as unknown as { stray?: unknown }).stray).toBeUndefined();
    expect((out.workspaces[0] as unknown as { extra?: unknown }).extra).toBeUndefined();
  });
});

describe("loadTabsState", () => {
  it("reads from the given storage", () => {
    const storage = memory();
    const state = valid();
    saveTabsState(state, storage);
    expect(loadTabsState(storage, () => true)).toEqual(state);
  });

  it("returns null when the user has turned session restore off", () => {
    // Classic honours the same preference; the new design must not be the
    // one place that remembers anyway.
    const storage = memory();
    saveTabsState(valid(), storage);
    expect(loadTabsState(storage, () => false)).toBeNull();
  });

  it("returns null when nothing was saved", () => {
    expect(loadTabsState(memory(), () => true)).toBeNull();
  });
});

describe("scheduleSave / flushSave", () => {
  it("debounces, writing once after the delay", () => {
    const storage = memory();
    const spy = vi.spyOn(storage, "setItem");
    scheduleSave(valid(), storage, 300);
    scheduleSave(valid(), storage, 300);
    expect(spy).not.toHaveBeenCalled();
    vi.advanceTimersByTime(300);
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it("writes the latest state, not the first scheduled", () => {
    const storage = memory();
    const a = valid();
    const b = valid();
    b.workspaces[0].name = "Later";
    scheduleSave(a, storage, 300);
    scheduleSave(b, storage, 300);
    vi.advanceTimersByTime(300);
    expect(parseStoredState(storage.getItem(BASE_STORAGE_KEY))?.workspaces[0].name).toBe("Later");
  });

  it("flush writes immediately and cancels the timer", () => {
    const storage = memory();
    const spy = vi.spyOn(storage, "setItem");
    scheduleSave(valid(), storage, 300);
    flushSave();
    expect(spy).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(300);
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it("flush with nothing pending writes nothing", () => {
    const storage = memory();
    const spy = vi.spyOn(storage, "setItem");
    flushSave();
    expect(spy).not.toHaveBeenCalled();
  });
});

describe("installFlushOnUnload", () => {
  it("flushes on beforeunload and detaches when told", () => {
    const handlers = new Map<string, () => void>();
    const target = {
      addEventListener: (n: string, h: () => void) => void handlers.set(n, h),
      removeEventListener: (n: string) => void handlers.delete(n),
    } as unknown as Window;
    const storage = memory();
    const spy = vi.spyOn(storage, "setItem");
    const off = installFlushOnUnload(target);
    scheduleSave(valid(), storage, 300);
    handlers.get("beforeunload")!();
    expect(spy).toHaveBeenCalledTimes(1);
    off();
    expect(handlers.has("beforeunload")).toBe(false);
  });
});

describe("getStorageKey", () => {
  it("hashes an over-long context window label before writing", () => {
    const storage = memory();
    const label = `ctx-${"61".repeat(120)}`;
    const state = valid();
    saveTabsState(state, storage, label);
    const key = getStorageKey(label);
    expect(key.length).toBeLessThanOrEqual(256);
    expect(storage.getItem(key)).not.toBeNull();
    expect(storage.getItem(`${BASE_STORAGE_KEY}-${label}`)).toBeNull();
    expect(loadTabsState(storage, () => true, label)).toEqual(state);
  });
});
