import { describe, it, expect } from "vitest";
import { parseNamespaceSelection, serializeNamespaceSelection, watchNamespaceForSelection, rowInSelection } from "./namespaces";

describe("parseNamespaceSelection", () => {
  it("splits a comma string, trims, drops blanks, and dedupes", () => {
    expect(parseNamespaceSelection("default,kube-system, default ,")).toEqual(["default", "kube-system"]);
  });
  it("treats empty / whitespace as 'all namespaces' (empty set)", () => {
    expect(parseNamespaceSelection("")).toEqual([]);
    expect(parseNamespaceSelection("  ")).toEqual([]);
  });
});

describe("serializeNamespaceSelection", () => {
  it("round-trips a set back to a comma string", () => {
    expect(serializeNamespaceSelection(["default", "kube-system"])).toBe("default,kube-system");
    expect(serializeNamespaceSelection([])).toBe("");
  });
});

describe("watchNamespaceForSelection", () => {
  it("watches the single namespace when exactly one is selected", () => {
    expect(watchNamespaceForSelection(["prod"])).toBe("prod");
  });
  it("watches all namespaces for none or many (filtered client-side)", () => {
    expect(watchNamespaceForSelection([])).toBe("");
    expect(watchNamespaceForSelection(["a", "b"])).toBe("");
  });
});

describe("rowInSelection", () => {
  it("keeps every row when nothing is selected (all)", () => {
    expect(rowInSelection("anything", [])).toBe(true);
  });
  it("keeps only rows whose namespace is in the set", () => {
    expect(rowInSelection("a", ["a", "b"])).toBe(true);
    expect(rowInSelection("c", ["a", "b"])).toBe(false);
  });
});
