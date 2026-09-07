import { describe, it, expect, beforeEach } from "vitest";
import { loadColumnPrefs, hiddenColumns, toggleColumn, resetColumns, useHiddenColumns, COLUMN_PREFS_KEY } from "./columnPrefs";

describe("column preferences", () => {
  beforeEach(() => {
    localStorage.clear();
    loadColumnPrefs();
  });

  it("hides a column for one kind without touching another's", () => {
    toggleColumn("pods", "node");
    toggleColumn("deployments", "available");
    expect([...hiddenColumns("pods")]).toEqual(["node"]);
    expect([...hiddenColumns("deployments")]).toEqual(["available"]);
  });

  it("survives a reload", () => {
    toggleColumn("pods", "node");
    loadColumnPrefs();
    expect(hiddenColumns("pods").has("node")).toBe(true);
  });

  it("hands back the same set until it changes, so a subscriber cannot tear", () => {
    expect(hiddenColumns("pods")).toBe(hiddenColumns("pods"));
  });

  it("does not erase another kind's stored entry when one kind is written first", () => {
    localStorage.setItem(COLUMN_PREFS_KEY, JSON.stringify({ pods: ["node"] }));
    loadColumnPrefs();
    toggleColumn("deployments", "available");
    expect(JSON.parse(localStorage.getItem(COLUMN_PREFS_KEY)!).pods).toEqual(["node"]);
  });

  it("costs the preferences and nothing else when storage refuses", () => {
    const throwing = {
      getItem() {
        throw new DOMException("denied");
      },
      setItem() {
        throw new DOMException("denied");
      },
      removeItem() {
        throw new DOMException("denied");
      },
    };
    expect(() => loadColumnPrefs(throwing)).not.toThrow();
    expect(() => toggleColumn("pods", "node", throwing)).not.toThrow();
  });

  it("toggling a column back off removes it again", () => {
    toggleColumn("pods", "node");
    toggleColumn("pods", "node");
    expect([...hiddenColumns("pods")]).toEqual([]);
  });

  it("resetColumns clears a kind without touching another's", () => {
    toggleColumn("pods", "node");
    toggleColumn("deployments", "available");
    resetColumns("pods");
    expect([...hiddenColumns("pods")]).toEqual([]);
    expect([...hiddenColumns("deployments")]).toEqual(["available"]);
  });

  it("re-renders a subscriber when its kind's columns are toggled and reset", async () => {
    const { renderHook, act } = await import("@testing-library/react");
    const { result } = renderHook(() => useHiddenColumns("pods"));
    expect([...result.current]).toEqual([]);
    act(() => toggleColumn("pods", "node"));
    expect([...result.current]).toEqual(["node"]);
    act(() => resetColumns("pods"));
    expect([...result.current]).toEqual([]);
  });
});

/**
 * #426 needed a column that starts hidden — a Nodes list's taint tally, useful
 * when you go looking for it and clutter when you are not. The record holds
 * HIDDEN keys, so the whole difficulty is that "the reader turned the
 * default-off column on" and "the reader has said nothing" are both an absent
 * key, and the naive version springs the column back to hidden on relaunch.
 */
describe("a column that starts hidden", () => {
  beforeEach(() => {
    localStorage.clear();
    loadColumnPrefs();
  });

  it("is hidden before the reader has said anything", () => {
    expect(hiddenColumns("nodes", ["taints"]).has("taints")).toBe(true);
  });

  it("leaves every other column of that kind alone", () => {
    expect(hiddenColumns("nodes", ["taints"]).has("age")).toBe(false);
  });

  it("stays on once turned on — including across a reload", () => {
    toggleColumn("nodes", "taints", undefined, ["taints"]);
    expect(hiddenColumns("nodes", ["taints"]).has("taints")).toBe(false);
    loadColumnPrefs();
    expect(hiddenColumns("nodes", ["taints"]).has("taints")).toBe(false);
  });

  it("goes back off when turned off again", () => {
    toggleColumn("nodes", "taints", undefined, ["taints"]);
    toggleColumn("nodes", "taints", undefined, ["taints"]);
    expect(hiddenColumns("nodes", ["taints"]).has("taints")).toBe(true);
  });

  it("does not drag another column down with it on that first toggle", () => {
    toggleColumn("nodes", "taints", undefined, ["taints"]);
    toggleColumn("nodes", "version", undefined, ["taints"]);
    const hidden = hiddenColumns("nodes", ["taints"]);
    expect(hidden.has("taints")).toBe(false);
    expect(hidden.has("version")).toBe(true);
  });

  it("comes back to its default once the kind is reset", () => {
    toggleColumn("nodes", "taints", undefined, ["taints"]);
    resetColumns("nodes");
    expect(hiddenColumns("nodes", ["taints"]).has("taints")).toBe(true);
  });

  it("is still stable per set of defaults, so a subscriber cannot tear", () => {
    expect(hiddenColumns("nodes", ["taints"])).toBe(hiddenColumns("nodes", ["taints"]));
    // …and asking with different defaults is a different question, not the
    // memoised answer to the first one.
    expect(hiddenColumns("nodes", ["taints"])).not.toBe(hiddenColumns("nodes", []));
  });

  it("changes nothing for a kind that declares no defaults", () => {
    toggleColumn("pods", "node");
    expect([...hiddenColumns("pods")]).toEqual(["node"]);
  });
});
