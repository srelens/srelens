import { describe, it, expect } from "vitest";
import { formatBytes, groupNumber } from "./numbers";

describe("groupNumber", () => {
  it("leaves anything under a thousand alone", () => {
    expect(groupNumber(0)).toBe("0");
    expect(groupNumber(7)).toBe("7");
    expect(groupNumber(999)).toBe("999");
  });

  it("groups every three digits, from the right", () => {
    expect(groupNumber(1000)).toBe("1 000");
    expect(groupNumber(1200)).toBe("1 200");
    expect(groupNumber(12345)).toBe("12 345");
    expect(groupNumber(1234567)).toBe("1 234 567");
  });

  it("separates with a space, not a comma or a full stop", () => {
    // The whole reason this is not `toLocaleString`: the same buffer size
    // would read `1,200` for one reader and `1.200` for another, and the two
    // mean different numbers to the people who use them.
    const grouped = groupNumber(1234567);
    expect(grouped).not.toContain(",");
    expect(grouped).not.toContain(".");
    expect(grouped.split(" ")).toEqual(["1", "234", "567"]);
  });

  it("groups a negative number's digits without cutting into the sign", () => {
    // `-1200` alone proves nothing about the sign: its leading group is one
    // digit long, so the only place a separator can land is between the digits
    // whether or not the `\B` guard is there. The guard has something to stop
    // only when the leading group is exactly three digits — that is where a
    // word boundary sits immediately after the `-` and an unguarded lookahead
    // writes `- 120`.
    expect(groupNumber(-120)).toBe("-120");
    expect(groupNumber(-999)).toBe("-999");
    expect(groupNumber(-120000)).toBe("-120 000");
    expect(groupNumber(-1200)).toBe("-1 200");
    expect(groupNumber(-1234567)).toBe("-1 234 567");
  });
});

describe("formatBytes", () => {
  it("writes the units the design writes", () => {
    // §13 and §17 spell these decimal — `54.2 MB` is kubectl, which is ~54 MB
    // decimal and ~51 MiB. So 1000, not 1024: the design is describing what a
    // download says, not what a filesystem does.
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(312_000)).toBe("312 KB");
    expect(formatBytes(1_200_000)).toBe("1.2 MB");
    expect(formatBytes(54_200_000)).toBe("54.2 MB");
    expect(formatBytes(12_000_000)).toBe("12.0 MB");
  });

  it("keeps a whole megabyte's trailing zero, so a column stays in line", () => {
    // `12.0 MB` not `12 MB`: these are tabular figures in a right-aligned
    // column, and a row that drops its decimal shifts against its neighbours.
    expect(formatBytes(12_000_000)).toBe("12.0 MB");
    expect(formatBytes(2_000_000)).toBe("2.0 MB");
  });

  it("carries on past a gigabyte rather than reading as thousands of MB", () => {
    expect(formatBytes(4_500_000_000)).toBe("4.5 GB");
  });

  it("says nothing about a reading it does not have", () => {
    // A missing metric is never zero. A tool with no readable path has no
    // size, and `0 B` would be a measurement nobody took.
    expect(formatBytes(undefined)).toBe("");
    expect(formatBytes(null)).toBe("");
  });
});
