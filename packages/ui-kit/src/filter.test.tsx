import { describe, expect, it } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { filterMatcher, useListFilter } from "./filter";

describe("filterMatcher", () => {
  it("finds a substring whatever its case", () => {
    const matches = filterMatcher("  API ");
    expect(matches("my-api-0")).toBe(true);
    expect(matches("web")).toBe(false);
  });

  it("matches everything while the query is blank", () => {
    expect(filterMatcher("   ")("anything")).toBe(true);
  });

  it("reads the query as a regular expression when asked", () => {
    const matches = filterMatcher("^api-\\d$", true);
    expect(matches("API-1")).toBe(true);
    expect(matches("my-api-1")).toBe(false);
  });

  it("gives the same answer however many values one matcher is asked about", () => {
    const matches = filterMatcher("a", true);
    expect([matches("a"), matches("a"), matches("a")]).toEqual([true, true, true]);
  });

  it("hides nothing while a regular expression is half typed", () => {
    expect(filterMatcher("api(", true)("web")).toBe(true);
  });
});

describe("useListFilter", () => {
  it("hands FilterBar its value and regex toggle", () => {
    const { result } = renderHook(() => useListFilter());
    expect(result.current).toMatchObject({ value: "", regex: false, invalid: false });
    act(() => result.current.onValueChange("api("));
    act(() => result.current.onRegexChange(true));
    expect(result.current).toMatchObject({ value: "api(", regex: true, invalid: true });
  });

  it("answers for the filter it holds", () => {
    const { result } = renderHook(() => useListFilter());
    act(() => result.current.onValueChange("api"));
    expect(result.current.matches("my-api-0")).toBe(true);
    expect(result.current.matches("web")).toBe(false);
  });
});
