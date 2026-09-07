import { describe, it, expect } from "vitest";
import { ageFromTimestamp, durationBetween, absoluteTimestamp, timestampWithAge } from "./k8sTime";

const NOW = Date.parse("2026-01-01T00:00:00Z");

describe("ageFromTimestamp", () => {
  it("formats seconds, minutes, hours, and days", () => {
    expect(ageFromTimestamp("2026-01-01T00:00:00Z", NOW + 30_000)).toBe("30s");
    expect(ageFromTimestamp("2026-01-01T00:00:00Z", NOW + 5 * 60_000)).toBe("5m");
    expect(ageFromTimestamp("2026-01-01T00:00:00Z", NOW + 3 * 3_600_000)).toBe("3h");
    expect(ageFromTimestamp("2026-01-01T00:00:00Z", NOW + 2 * 86_400_000)).toBe("2d");
  });

  it("returns a dash for missing or invalid input", () => {
    expect(ageFromTimestamp(undefined, NOW)).toBe("—");
    expect(ageFromTimestamp("not-a-date", NOW)).toBe("—");
  });
});

describe("durationBetween", () => {
  it("formats a normal span as minutes and seconds", () => {
    expect(durationBetween("2026-01-01T00:00:00Z", "2026-01-01T00:02:30Z")).toBe("2m 30s");
  });

  it("returns a dash when the start is missing", () => {
    expect(durationBetween(undefined, "2026-01-01T00:02:30Z")).toBe("—");
  });

  it("returns a dash when the end is missing", () => {
    expect(durationBetween("2026-01-01T00:00:00Z", undefined)).toBe("—");
  });

  it("clamps to 0s when the end is before the start (clock skew)", () => {
    // The body clamps the negative diff with Math.max(0, ...) rather than
    // treating it as invalid, so a skewed end lands on "0s", not a dash.
    expect(durationBetween("2026-01-01T00:02:00Z", "2026-01-01T00:00:00Z")).toBe("0s");
  });
});

describe("absoluteTimestamp", () => {
  it("formats a valid ISO string into a non-empty, locale-formatted timestamp", () => {
    // toLocaleString(undefined, …) is locale- and timezone-dependent, so the
    // exact string varies by machine; assert the parts that don't.
    const result = absoluteTimestamp("2026-06-10T12:52:33Z");
    expect(result).not.toBe("");
    expect(result).toContain("2026");
  });

  it("returns an empty string for undefined", () => {
    expect(absoluteTimestamp(undefined)).toBe("");
  });
});

describe("timestampWithAge", () => {
  it("combines the relative age and the absolute timestamp, driven by an injected now", () => {
    const iso = "2026-01-01T00:00:00Z";
    const now = NOW + 30_000;
    expect(timestampWithAge(iso, now)).toBe(`${ageFromTimestamp(iso, now)} ago (${absoluteTimestamp(iso)})`);
    expect(timestampWithAge(iso, now)).toBe(`30s ago (${absoluteTimestamp(iso)})`);
  });

  it("returns an empty string for an empty iso, without touching now", () => {
    expect(timestampWithAge("", NOW)).toBe("");
  });
});
