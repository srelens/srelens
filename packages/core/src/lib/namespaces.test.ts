import { describe, it, expect } from "vitest";
import {
  NO_NAMESPACE,
  SELECTS_NOTHING,
  namespacePhrase,
  parseNamespaceSelection,
  rowInSelection,
  selectsNothing,
  serializeNamespaceSelection,
  watchNamespaceForSelection,
} from "./namespaces";

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

describe("namespacePhrase", () => {
  it("names one, two or three namespaces in words", () => {
    expect(namespacePhrase(["a"])).toBe("a");
    expect(namespacePhrase(["a", "b"])).toBe("a and b");
    expect(namespacePhrase(["a", "b", "c"])).toBe("a, b and c");
  });
  it("counts a longer run rather than listing it", () => {
    expect(namespacePhrase(["a", "b", "c", "d", "e"])).toBe("5 namespaces: a, b, c and 2 more");
  });
});

/**
 * The empty selection means every namespace, so "nothing matches" (#881) needs
 * a value of its own, and every helper over a selection has to read it as
 * nothing rather than as a namespace called that.
 */
describe("a selection of nothing", () => {
  it("is recognised, and nothing else is", () => {
    expect(selectsNothing(SELECTS_NOTHING)).toBe(true);
    expect(selectsNothing([NO_NAMESPACE, NO_NAMESPACE])).toBe(true);
    expect(selectsNothing([])).toBe(false);
    expect(selectsNothing(["default"])).toBe(false);
    expect(selectsNothing([NO_NAMESPACE, "default"])).toBe(false);
  });

  it("can never be a namespace a cluster has", () => {
    expect(NO_NAMESPACE).not.toMatch(/^[a-z0-9]([-a-z0-9]*[a-z0-9])?$/);
  });

  it("keeps every row out, and names no namespace to watch", () => {
    expect(rowInSelection("default", [...SELECTS_NOTHING])).toBe(false);
    expect(rowInSelection("", [...SELECTS_NOTHING])).toBe(false);
    expect(watchNamespaceForSelection([...SELECTS_NOTHING])).toBe("");
  });
});
