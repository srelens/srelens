import { describe, expect, it } from "vitest";
import { ageSeconds, ageSortValue } from "./age";

describe("ageSeconds", () => {
  it("converts every backend unit to seconds", () => {
    expect(ageSeconds("45s")).toBe(45);
    expect(ageSeconds("3m")).toBe(180);
    expect(ageSeconds("2h")).toBe(7_200);
    expect(ageSeconds("300d")).toBe(300 * 86_400);
    expect(ageSeconds("1y")).toBe(365 * 86_400);
  });

  it("orders across units — the #236 report: 1y must outrank 300d", () => {
    expect(ageSeconds("1y")).toBeGreaterThan(ageSeconds("300d"));
    expect(ageSeconds("2d")).toBeGreaterThan(ageSeconds("47h"));
    expect(ageSeconds("10m")).toBeGreaterThan(ageSeconds("599s"));
  });

  it("reads a row's age through the shared column helper", () => {
    expect(ageSortValue({ age: "1y" })).toBe(365 * 86_400);
    expect(ageSortValue({})).toBe(-1);
  });

  it("groups unset and unrecognized ages below every real age", () => {
    expect(ageSeconds("-")).toBe(-1);
    expect(ageSeconds("")).toBe(-1);
    expect(ageSeconds("5x")).toBe(-1);
    expect(ageSeconds("y5")).toBe(-1);
    expect(ageSeconds("-")).toBeLessThan(ageSeconds("0s"));
  });
});
