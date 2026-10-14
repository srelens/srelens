import { describe, expect, it } from "vitest";
import { answer } from "./regexMatch";

describe("answer", () => {
  it("says whether each tested string matches, with the clause's flags", () => {
    expect(answer("a+b", "i", [{ test: "xAAB" }, { test: "zz" }])).toEqual([{ hit: true }, { hit: false }]);
    expect(answer("a+b", "", [{ test: "xAAB" }])).toEqual([{ hit: false }]);
  });

  it("marks every match over the shown string, not the tested one, case-insensitively with `i`", () => {
    expect(answer("sat\\w+", "i", [{ test: "pool saturated", shown: "Pool timeout: pool Saturated, SATURATED" }])).toEqual([
      { hit: true, ranges: [[19, 28], [30, 39]] },
    ]);
  });

  it("steps past an empty match rather than looping on it", () => {
    expect(answer("x*", "i", [{ test: "abc", shown: "abc" }])).toEqual([{ hit: true, ranges: [] }]);
  });

  it("gives no ranges for an item with no shown string", () => {
    expect(answer("a", "i", [{ test: "a" }])[0]).not.toHaveProperty("ranges");
  });
});
