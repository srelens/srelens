import { describe, it, expect } from "vitest";
import { printerColumnKeys, printerSortValue } from "./crds";
import { ageSeconds } from "./age";

const col = (name: string, jsonPath: string, type = "string") => ({ name, jsonPath, type });

/** printerSortValue narrowed to bigint, for the integer cases. */
const intKey = (text: string) => printerSortValue("integer", text) as bigint;

describe("printerColumnKeys", () => {
  it("identifies a column by its definition, not its position", () => {
    // An operator upgrade that prepends a column must not change the key of the
    // ones already there: those keys persist hidden/sort/filter state.
    const before = printerColumnKeys([col("Ready", ".status.ready"), col("Version", ".spec.version")]);
    const after = printerColumnKeys([
      col("Phase", ".status.phase"),
      col("Ready", ".status.ready"),
      col("Version", ".spec.version"),
    ]);
    expect(after.slice(1)).toEqual(before);
  });

  it("distinguishes columns sharing a heading but reading different fields", () => {
    const keys = printerColumnKeys([col("Status", ".status.a"), col("Status", ".status.b")]);
    expect(new Set(keys).size).toBe(2);
  });

  it("still disambiguates genuinely identical definitions", () => {
    const keys = printerColumnKeys([col("Ready", ".status.ready"), col("Ready", ".status.ready")]);
    expect(new Set(keys).size).toBe(2);
  });
});

describe("printerSortValue", () => {
  it("orders signed and decimal numbers numerically", () => {
    // The table's collator gets both of these backwards on the rendered text.
    expect(intKey("-10") < intKey("-2")).toBe(true);
    expect(printerSortValue("number", "1.15")).toBeLessThan(printerSortValue("number", "1.2") as number);
    expect(intKey("2") < intKey("10")).toBe(true);
  });

  it("keeps 64-bit integers exact (#267 review)", () => {
    // Number() maps both of these to 9007199254740992, tying the two rows.
    const lo = intKey("9007199254740992");
    const hi = intKey("9007199254740993");
    expect(lo).not.toBe(hi);
    expect(lo < hi).toBe(true);
    expect(intKey("-9007199254740993") < lo).toBe(true);
  });

  it("falls back for integer text that is not an integer", () => {
    // A decimal in an `integer` column is malformed; BigInt() would throw.
    expect(printerSortValue("integer", "1.5")).toBe(Number.NEGATIVE_INFINITY);
    expect(printerSortValue("integer", "n/a")).toBe(Number.NEGATIVE_INFINITY);
  });

  it("orders dates by duration, not by the text of the age", () => {
    // "10d" vs "2h": text collation puts 10d first by leading digit.
    expect(printerSortValue("date", "2h")).toBeLessThan(printerSortValue("date", "10d") as number);
    expect(printerSortValue("date", "300d")).toBeLessThan(printerSortValue("date", "1y") as number);
  });

  it("separates dates that render as the same age (#267 review)", () => {
    // 65 and 115 minutes old both display "1h"; only the raw value can order them.
    const now = Date.parse("2026-01-01T12:00:00Z");
    const older = printerSortValue("date", "1h", "2026-01-01T10:05:00Z", now) as number;
    const newer = printerSortValue("date", "1h", "2026-01-01T10:55:00Z", now) as number;
    expect(older).toBeGreaterThan(newer); // larger = older, as ageSeconds orders
  });

  it("sorts raw timestamps in the same direction as compact ages", () => {
    // A mixed list must not reverse when some rows carry a raw value.
    const now = Date.parse("2026-01-01T12:00:00Z");
    const raw = printerSortValue("date", "2h", "2026-01-01T10:00:00Z", now) as number;
    expect(raw).toBeGreaterThan(ageSeconds("1h"));
    expect(raw).toBeLessThan(ageSeconds("10d"));
  });

  it("falls back to the rendered age when no raw value is present", () => {
    expect(printerSortValue("date", "2h", "")).toBe(ageSeconds("2h"));
    expect(printerSortValue("date", "2h", "not a date")).toBe(ageSeconds("2h"));
  });

  it("groups unset and unparseable values below real ones", () => {
    expect(printerSortValue("integer", "")).toBe(Number.NEGATIVE_INFINITY);
    expect(printerSortValue("number", "n/a")).toBe(Number.NEGATIVE_INFINITY);
    expect(printerSortValue("date", "-")).toBe(-1);
  });

  it("leaves string columns as text", () => {
    expect(printerSortValue("string", "GREEN")).toBe("GREEN");
  });
});
