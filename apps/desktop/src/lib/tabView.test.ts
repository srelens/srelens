import { describe, expect, it } from "vitest";
import { applyViewPatch, nextSort } from "./tabView";

describe("nextSort", () => {
  it("cycles ascending, descending, then unsorted", () => {
    const first = nextSort(null, "name");
    expect(first).toEqual({ key: "name", direction: "asc" });
    const second = nextSort(first, "name");
    expect(second).toEqual({ key: "name", direction: "desc" });
    expect(nextSort(second, "name")).toBeNull();
  });

  it("starts a different column ascending, whatever the previous one was", () => {
    expect(nextSort({ key: "name", direction: "desc" }, "age")).toEqual({
      key: "age",
      direction: "asc",
    });
  });

  it("treats undefined like unsorted", () => {
    expect(nextSort(undefined, "age")).toEqual({ key: "age", direction: "asc" });
  });
});

describe("applyViewPatch", () => {
  it("merges a patch into existing view state", () => {
    const result = applyViewPatch({ query: "nginx" }, { sort: { key: "age", direction: "asc" } });
    expect(result).toEqual({ query: "nginx", sort: { key: "age", direction: "asc" } });
  });

  it("returns undefined when nothing is left, so empty tabs stay clean", () => {
    // Otherwise every untouched tab would serialize a `"view": {}` into the
    // session file for no reason.
    expect(applyViewPatch(undefined, {})).toBeUndefined();
    expect(applyViewPatch({ query: "x" }, { query: "" })).toBeUndefined();
    expect(applyViewPatch({ sort: { key: "a", direction: "asc" } }, { sort: null })).toBeUndefined();
  });

  it("drops cleared fields rather than storing empty placeholders", () => {
    const result = applyViewPatch(
      { query: "nginx", filterColumn: "name", sort: { key: "age", direction: "asc" } },
      { filterColumn: null },
    );
    expect(result).toEqual({ query: "nginx", sort: { key: "age", direction: "asc" } });
    expect(result && "filterColumn" in result).toBe(false);
  });

  it("keeps unrelated fields when one is patched", () => {
    const result = applyViewPatch(
      { query: "nginx", sort: { key: "age", direction: "desc" } },
      { query: "coredns" },
    );
    expect(result).toEqual({ query: "coredns", sort: { key: "age", direction: "desc" } });
  });
});
