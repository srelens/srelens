import { describe, it, expect } from "vitest";
import { parseQuantity, usagePercent, formatBytes, decodedByteLength } from "./k8sQuantity";

// Moved verbatim from apps/desktop/src/components/ResourceOverview.test.tsx
// (only the import path changed).
describe("parseQuantity", () => {
  it("parses plain, milli, binary, and decimal suffixes", () => {
    expect(parseQuantity("4")).toBe(4);
    expect(parseQuantity("500m")).toBe(0.5);
    expect(parseQuantity("2Gi")).toBe(2 * 2 ** 30);
    expect(parseQuantity("1G")).toBe(1e9);
  });
  it("returns null for unparseable input", () => {
    expect(parseQuantity("")).toBeNull();
    expect(parseQuantity("abc")).toBeNull();
  });
});

// classic's moved tests exercise the "" / "abc" input, which both fail to
// match the leading regex at all. Written here: the second, distinct null
// path — input that matches the regex's [0-9.]+ shape but parses to NaN —
// which no moved test reaches.
describe("parseQuantity — the NaN-after-match path", () => {
  it("returns null when the numeric part matches the regex shape but is not a valid number", () => {
    // ".." matches `[0-9.]+` (dots only) but Number.parseFloat("..") is NaN.
    expect(parseQuantity("..")).toBeNull();
  });
});

// classic's moved test never reaches an unrecognized unit suffix (every
// tested suffix is either "", "m", or a key in the binary/decimal maps).
// Written here to prove the final fallback line is live, not dead.
describe("parseQuantity — unrecognized unit fallback", () => {
  it("treats an unrecognized unit as a no-op multiplier, keeping the numeric value as-is", () => {
    expect(parseQuantity("5Q")).toBe(5);
  });
});

// classic's ResourceOverview.test.tsx did not cover usagePercent; written here
// against its actual branches.
describe("usagePercent", () => {
  it("computes a rounded percentage of used over hard", () => {
    expect(usagePercent("500m", "1")).toBe(50);
  });

  it("returns null when 'used' does not parse", () => {
    expect(usagePercent("abc", "1")).toBeNull();
  });

  it("returns null when 'hard' does not parse", () => {
    expect(usagePercent("1", "abc")).toBeNull();
  });

  it("returns null when 'hard' parses to zero, to avoid dividing by zero", () => {
    expect(usagePercent("1", "0")).toBeNull();
  });
});

// classic's ResourceOverview.test.tsx did not cover formatBytes; written here
// against its actual thresholds.
describe("formatBytes", () => {
  it("formats sub-1024 byte counts as bytes", () => {
    expect(formatBytes(500)).toBe("500 B");
  });

  it("formats the 1024-byte boundary as KiB, not B", () => {
    expect(formatBytes(1024)).toBe("1.0 KiB");
  });

  it("formats sub-1MiB counts as KiB with one decimal", () => {
    expect(formatBytes(2048)).toBe("2.0 KiB");
  });

  it("formats the 1MiB boundary and above as MiB, not KiB", () => {
    expect(formatBytes(1024 * 1024)).toBe("1.0 MiB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MiB");
  });
});

// classic's ResourceOverview.test.tsx did not cover decodedByteLength; written
// here against its two branches (successful decode vs. the catch fallback).
describe("decodedByteLength", () => {
  it("returns the decoded byte length of valid base64", () => {
    // btoa("hello") === "aGVsbG8=" — 5 decoded bytes.
    expect(decodedByteLength("aGVsbG8=")).toBe(5);
  });

  it("falls back to the raw string's UTF-8 byte length when atob throws on invalid base64", () => {
    const invalid = "not valid base64 !!";
    expect(decodedByteLength(invalid)).toBe(new TextEncoder().encode(invalid).length);
  });
});
