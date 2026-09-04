import { describe, expect, it } from "vitest";
import type { DiffRow } from "@srelens/core";
import { collapseDiff, diffCounts } from "./diffCollapse";

const same = (text: string): DiffRow => ({ tag: "same", left: text, right: text });
const insert = (text: string): DiffRow => ({ tag: "insert", left: null, right: text });
const remove = (text: string): DiffRow => ({ tag: "delete", left: text, right: null });
const replace = (left: string, right: string): DiffRow => ({ tag: "replace", left, right });

/** N unchanged lines, numbered so a slice can be checked for which ones. */
const filler = (n: number, from = 0) => Array.from({ length: n }, (_, i) => same(`line ${from + i}`));

describe("diffCounts", () => {
  it("counts a replace as both an addition and a deletion", () => {
    expect(diffCounts([same("a"), insert("b"), remove("c"), replace("d", "e")])).toEqual({
      added: 2,
      removed: 2,
    });
  });

  it("counts nothing in a diff with no changes", () => {
    expect(diffCounts(filler(5))).toEqual({ added: 0, removed: 0 });
  });
});

describe("collapseDiff", () => {
  it("hides a long unchanged run and keeps context either side of the change", () => {
    // The shape that made this necessary: one changed label inside a
    // Deployment whose last-applied-configuration annotation is 30 lines.
    const rows = [...filler(30), insert("  tier: web"), ...filler(30, 30)];
    const segments = collapseDiff(rows, 3);
    expect(segments.map((s) => s.kind)).toEqual(["gap", "rows", "gap"]);
    expect(segments[0].rows).toHaveLength(27);
    // Three lines before, the change, three after.
    expect(segments[1].rows).toHaveLength(7);
    expect(segments[1].rows[3]).toEqual(insert("  tier: web"));
    expect(segments[2].rows).toHaveLength(27);
  });

  it("carries the hidden rows in the gap, so opening one needs nothing else", () => {
    const rows = [...filler(20), insert("x")];
    const [gap] = collapseDiff(rows, 3);
    expect(gap.kind).toBe("gap");
    expect(gap.rows[0]).toEqual(same("line 0"));
    expect(gap.rows.at(-1)).toEqual(same("line 16"));
    expect(gap.from).toBe(0);
  });

  it("keeps a short unchanged run rather than making a gap that saves nothing", () => {
    // Two changes with two unchanged lines between them: a gap here would be
    // a click to hide less than the gap marker itself takes up.
    const rows = [insert("a"), same("1"), same("2"), insert("b"), ...filler(30, 10)];
    const segments = collapseDiff(rows, 1, 4);
    expect(segments[0].kind).toBe("rows");
    expect(segments[0].rows.slice(0, 4)).toEqual([insert("a"), same("1"), same("2"), insert("b")]);
  });

  it("leaves a diff with no changes whole, rather than hiding all of it", () => {
    // "Unchanged" is an answer; an empty panel is not.
    const rows = filler(40);
    expect(collapseDiff(rows)).toEqual([{ kind: "rows", rows, from: 0 }]);
  });

  it("gives an empty diff no segments at all", () => {
    expect(collapseDiff([])).toEqual([]);
  });

  it("shows every line of a diff that is all changes", () => {
    const rows = [insert("a"), remove("b"), replace("c", "d")];
    expect(collapseDiff(rows)).toEqual([{ kind: "rows", rows, from: 0 }]);
  });

  it("keeps the run between two changes when it is short, and hides it when it is long", () => {
    const near = [insert("a"), ...filler(5), insert("b")];
    expect(collapseDiff(near, 3).map((s) => s.kind)).toEqual(["rows"]);
    const far = [insert("a"), ...filler(40), insert("b")];
    expect(collapseDiff(far, 3).map((s) => s.kind)).toEqual(["rows", "gap", "rows"]);
  });
});
