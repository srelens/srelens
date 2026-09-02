import { describe, it, expect, beforeEach } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useColumnVisibility } from "./useColumnVisibility";
import type { Column } from "./Table";

interface Row {
  name: string;
}

const columns: Column<Row>[] = [
  { key: "name", header: "Name" },
  { key: "status", header: "Status" },
  { key: "age", header: "Age" },
  { key: "actions", header: "" }, // headerless — a row-actions cell
];

beforeEach(() => localStorage.clear());

describe("useColumnVisibility", () => {
  it("lists only labelled columns and pins the first", () => {
    const { result } = renderHook(() => useColumnVisibility("nodes", columns));
    expect(result.current.pinnedKey).toBe("name");
    // The headerless "actions" column is not offered.
    expect(result.current.columnOptions.map((c) => c.key)).toEqual(["name", "status", "age"]);
    // All columns visible by default.
    expect(result.current.visibleColumns.map((c) => c.key)).toEqual(["name", "status", "age", "actions"]);
  });

  it("hides a column, keeps identifier and headerless columns, and persists", () => {
    const { result } = renderHook(() => useColumnVisibility("nodes", columns));
    act(() => result.current.toggle("status"));
    expect(result.current.visibleColumns.map((c) => c.key)).toEqual(["name", "age", "actions"]);
    expect(JSON.parse(localStorage.getItem("srelens.hiddenColumns")!)).toEqual({ nodes: ["status"] });

    // The pinned identifier and headerless columns can't be hidden.
    act(() => result.current.toggle("name"));
    act(() => result.current.toggle("actions"));
    expect(result.current.visibleColumns.map((c) => c.key)).toEqual(["name", "age", "actions"]);
  });

  it("loads persisted hidden columns and isolates views by key", () => {
    localStorage.setItem("srelens.hiddenColumns", JSON.stringify({ nodes: ["age"], pods: ["status"] }));
    const nodes = renderHook(() => useColumnVisibility("nodes", columns));
    expect(nodes.result.current.visibleColumns.map((c) => c.key)).toEqual(["name", "status", "actions"]);
    const pods = renderHook(() => useColumnVisibility("pods", columns));
    expect(pods.result.current.visibleColumns.map((c) => c.key)).toEqual(["name", "age", "actions"]);
  });
});

/**
 * #426 — the classic half of "a column that starts hidden". Same trap as the
 * new design's store: the record holds hidden keys, so an absent entry has to
 * mean "no choice yet, use the default" while a stored entry — an empty one
 * included — is the reader's own answer and outranks it.
 */
describe("a column that starts hidden", () => {
  const withTaints: Column<Row>[] = [
    { key: "name", header: "Name" },
    { key: "status", header: "Status" },
    { key: "taints", header: "Taints", defaultHidden: true },
    { key: "age", header: "Age" },
  ];
  const keys = (result: { visibleColumns: Column<Row>[] }) => result.visibleColumns.map((c) => c.key);

  it("is out of the table before the reader has said anything", () => {
    const { result } = renderHook(() => useColumnVisibility("nodes", withTaints));
    expect(keys(result.current)).toEqual(["name", "status", "age"]);
    expect(result.current.hidden.has("taints")).toBe(true);
  });

  it("is still offered by the picker, so there is a way to turn it on", () => {
    const { result } = renderHook(() => useColumnVisibility("nodes", withTaints));
    expect(result.current.columnOptions.map((o) => o.key)).toContain("taints");
  });

  it("stays on once turned on, and survives a remount", () => {
    const first = renderHook(() => useColumnVisibility("nodes", withTaints));
    act(() => first.result.current.toggle("taints"));
    expect(keys(first.result.current)).toEqual(["name", "status", "taints", "age"]);

    const second = renderHook(() => useColumnVisibility("nodes", withTaints));
    expect(keys(second.result.current)).toEqual(["name", "status", "taints", "age"]);
  });

  it("goes back off when turned off again", () => {
    const { result } = renderHook(() => useColumnVisibility("nodes", withTaints));
    act(() => result.current.toggle("taints"));
    act(() => result.current.toggle("taints"));
    expect(keys(result.current)).toEqual(["name", "status", "age"]);
  });

  it("hides only itself — turning it on does not disturb another column", () => {
    const { result } = renderHook(() => useColumnVisibility("nodes", withTaints));
    act(() => result.current.toggle("taints"));
    act(() => result.current.toggle("status"));
    expect(keys(result.current)).toEqual(["name", "taints", "age"]);
  });

  it("leaves a view whose columns declare no default exactly as it was", () => {
    const { result } = renderHook(() => useColumnVisibility("pods", columns));
    expect(result.current.hidden.size).toBe(0);
  });
});
