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
