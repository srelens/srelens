// jsdom, not node: `./tabs` imports `./routes`, whose screen table reaches
// `@xterm/addon-fit` — a UMD bundle that reads `self` as it evaluates.
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { ClusterContext } from "@srelens/core";
import { defaultState, makeTab, type TabsState } from "./tabs";
import {
  STORAGE_KEY, STORAGE_VERSION, flushSave, loadTabsState, parseStoredState, saveTabsState, scheduleSave,
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
  name: id, stableId: id, cluster: id, server: `https://${id}`, isCurrent: false,
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
  it("round-trips a state written by saveTabsState", () => {
    const storage = memory();
    const state = valid();
    saveTabsState(state, storage);
    expect(parseStoredState(storage.getItem(STORAGE_KEY))).toEqual(state);
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
    const doc = JSON.parse(storage.getItem(STORAGE_KEY)!);
    doc.version = STORAGE_VERSION + 1;
    expect(parseStoredState(JSON.stringify(doc))).toBeNull();
  });

  it("accepts a document from an older version, so a bump can migrate it", () => {
    // `version !== STORAGE_VERSION` refused older documents too, so the first
    // bump to 2 would have silently discarded every user's workspaces.
    const storage = memory();
    const state = valid();
    saveTabsState(state, storage);
    const doc = JSON.parse(storage.getItem(STORAGE_KEY)!);
    doc.version = 0;
    expect(parseStoredState(JSON.stringify(doc))).toEqual(state);
  });

  it("refuses a document with no version at all", () => {
    const storage = memory();
    saveTabsState(valid(), storage);
    const doc = JSON.parse(storage.getItem(STORAGE_KEY)!);
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

  it("keeps a tab's sort through a save and a load", () => {
    const s = valid();
    s.workspaces[0].tabs[1].view = { sort: { key: "restarts", direction: "desc" }, filter: "crash", filterKey: "status" };
    const storage = memory();
    saveTabsState(s, storage);
    const parsed = parseStoredState(storage.getItem(STORAGE_KEY));
    expect(parsed!.workspaces[0].tabs[1].view).toEqual({
      sort: { key: "restarts", direction: "desc" },
      filter: "crash",
      filterKey: "status",
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

  it("round-trips a null sort and a null filterKey", () => {
    const s = valid();
    s.workspaces[0].tabs[1].view = { sort: null, filterKey: null };
    const storage = memory();
    saveTabsState(s, storage);
    const parsed = parseStoredState(storage.getItem(STORAGE_KEY));
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
    expect(parseStoredState(storage.getItem(STORAGE_KEY))?.workspaces[0].name).toBe("Later");
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
