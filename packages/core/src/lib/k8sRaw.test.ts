import { describe, it, expect } from "vitest";
import { asRecord, asArray, str, plural } from "./k8sRaw";

describe("asRecord", () => {
  it("returns a plain object unchanged", () => {
    const obj = { a: 1 };
    expect(asRecord(obj)).toBe(obj);
  });

  it("returns {} for null", () => {
    expect(asRecord(null)).toEqual({});
  });

  it("returns {} for undefined", () => {
    expect(asRecord(undefined)).toEqual({});
  });

  it("returns {} for a primitive (not typeof object)", () => {
    expect(asRecord("hello")).toEqual({});
    expect(asRecord(42)).toEqual({});
    expect(asRecord(true)).toEqual({});
  });
});

describe("asArray", () => {
  it("returns an array unchanged", () => {
    const arr = [1, 2, 3];
    expect(asArray(arr)).toBe(arr);
  });

  it("returns [] for a non-array value", () => {
    expect(asArray({ a: 1 })).toEqual([]);
    expect(asArray(undefined)).toEqual([]);
    expect(asArray(null)).toEqual([]);
    expect(asArray("not an array")).toEqual([]);
  });
});

describe("str", () => {
  it("returns '' for null or undefined", () => {
    expect(str(null)).toBe("");
    expect(str(undefined)).toBe("");
  });

  it("returns a string value unchanged", () => {
    expect(str("hello")).toBe("hello");
    expect(str("")).toBe("");
  });

  it("stringifies a non-string, non-nullish value", () => {
    expect(str(42)).toBe("42");
    expect(str(true)).toBe("true");
    expect(str(false)).toBe("false");
  });
});

describe("plural", () => {
  it("uses the singular form when n is 1", () => {
    expect(plural(1, "item")).toBe("1 item");
  });

  it("defaults the plural form to singular + 's' when n is not 1", () => {
    expect(plural(0, "item")).toBe("0 items");
    expect(plural(2, "item")).toBe("2 items");
  });

  it("uses an explicit irregular plural when n is not 1", () => {
    expect(plural(2, "child", "children")).toBe("2 children");
    expect(plural(0, "child", "children")).toBe("0 children");
  });

  it("uses the singular form when n is 1 even if an explicit plural is given", () => {
    expect(plural(1, "child", "children")).toBe("1 child");
  });
});
