import { describe, it, expect } from "vitest";
import {
  LEVELS,
  MAX_RENDERED,
  filterLines,
  logLineLevel,
  parseAppLog,
  type Level,
} from "./appLogLines";

/** A line in the shape tauri-plugin-log writes. */
const line = (level: string, message: string, time = "09:12:03") =>
  `[2026-08-21][${time}][srelens::cluster][${level}] ${message}`;

describe("logLineLevel", () => {
  it("reads every level the logger emits", () => {
    for (const level of LEVELS) {
      expect(logLineLevel(line(level, "something happened"))).toBe(level);
    }
  });

  it("defaults to INFO for a line with no level bracket", () => {
    expect(logLineLevel("    at srelens::cluster::connect")).toBe("INFO");
    expect(logLineLevel("")).toBe("INFO");
    // A bracket that is not one of the five is not a level either.
    expect(logLineLevel("[2026-08-21][09:12:03][srelens][NOISE] hi")).toBe("INFO");
  });
});

describe("parseAppLog", () => {
  it("puts only the time in ts — the date stays in raw", () => {
    const [entry] = parseAppLog(line("WARN", "context prod is unreachable"));
    expect(entry.ts).toBe("09:12:03");
    expect(entry.raw).toContain("2026-08-21");
  });

  it("extracts the log target into source", () => {
    const [entry] = parseAppLog(line("INFO", "connected to prod"));
    expect(entry.source).toBe("srelens::cluster");
  });

  it("keeps a line the logger did not write, whole, indent and all", () => {
    const [entry] = parseAppLog("    at srelens::cluster::connect");
    expect(entry).toEqual({
      ts: "",
      source: "",
      level: "INFO",
      message: "    at srelens::cluster::connect",
      raw: "    at srelens::cluster::connect",
    });
  });

  it("drops a line's trailing whitespace, a Windows line ending's \\r included", () => {
    const parsed = parseAppLog(`${line("INFO", "one")}\r\n    at two\r\n`);
    expect(parsed.map((e) => e.message)).toEqual(["one", "    at two"]);
  });

  it("drops only the one space after the level bracket", () => {
    const [entry] = parseAppLog(line("ERROR", "  two spaces in"));
    expect(entry.message).toBe("  two spaces in");
  });

  it("gives a line with no prefix the level of the entry it continues", () => {
    const parsed = parseAppLog(
      [
        line("ERROR", "thread main panicked"),
        "   0: srelens::cluster::connect",
        // A timestamp with no level it knows opens an entry of its own.
        "[2026-08-21][09:12:04] no level here",
        "   1: srelens::main",
      ].join("\n"),
    );
    expect(parsed.map((e) => e.level)).toEqual(["ERROR", "ERROR", "INFO", "INFO"]);
  });

  it("drops blank lines and keeps the rest in order", () => {
    const parsed = parseAppLog(
      [line("INFO", "one"), "", line("ERROR", "two", "09:12:04"), ""].join("\n"),
    );
    expect(parsed.map((e) => e.message)).toEqual(["one", "two"]);
    expect(parsed.map((e) => e.level)).toEqual(["INFO", "ERROR"]);
  });

  it("is empty for an empty log", () => {
    expect(parseAppLog("")).toEqual([]);
  });
});

describe("filterLines", () => {
  const lines = parseAppLog(
    [
      line("INFO", "connected to prod"),
      line("ERROR", "RBAC denied for Pods", "09:12:04"),
      line("WARN", "slow response from prod", "09:12:05"),
    ].join("\n"),
  );

  it("keeps everything at level 'all' with no text", () => {
    expect(filterLines(lines, "", "all").lines).toHaveLength(3);
  });

  it("reports total equal to the shown count when nothing is capped", () => {
    expect(filterLines(lines, "", "all").total).toBe(3);
  });

  it("filters by level", () => {
    expect(filterLines(lines, "", "ERROR").lines.map((e) => e.message)).toEqual([
      "RBAC denied for Pods",
    ]);
  });

  it("filters by text, case-insensitively", () => {
    expect(filterLines(lines, "RBAC", "all").lines.map((e) => e.message)).toEqual([
      "RBAC denied for Pods",
    ]);
    expect(filterLines(lines, "rbac", "all").lines.map((e) => e.message)).toEqual([
      "RBAC denied for Pods",
    ]);
  });

  it("keeps an entry's continuation lines under the entry's level", () => {
    const backtrace = parseAppLog(
      [line("INFO", "connected"), line("ERROR", "panicked", "09:12:04"), "   0: frame"].join("\n"),
    );
    expect(filterLines(backtrace, "", "ERROR").lines.map((e) => e.message)).toEqual([
      "panicked",
      "   0: frame",
    ]);
  });

  it("applies text and level together", () => {
    expect(filterLines(lines, "prod", "WARN").lines.map((e) => e.message)).toEqual([
      "slow response from prod",
    ]);
    expect(filterLines(lines, "prod", "ERROR").lines).toEqual([]);
    expect(filterLines(lines, "prod", "ERROR").total).toBe(0);
  });

  it("keeps the newest MAX_RENDERED of a log that exceeds the cap, and reports the pre-cap total", () => {
    const many = parseAppLog(
      Array.from({ length: MAX_RENDERED + 1 }, (_, i) => line("INFO", `entry ${i}`)).join("\n"),
    );
    const { lines: capped, total } = filterLines(many, "", "all");
    expect(MAX_RENDERED).toBe(5000);
    expect(capped).toHaveLength(MAX_RENDERED);
    // The oldest is the one dropped, so the window ends at the newest line.
    expect(capped[0].message).toBe("entry 1");
    expect(capped[capped.length - 1].message).toBe(`entry ${MAX_RENDERED}`);
    // total is the pre-cap match count, not the (already-capped) shown count —
    // this is what lets the screen tell a real cap apart from zero matches.
    expect(total).toBe(MAX_RENDERED + 1);
  });
});

describe("LEVELS", () => {
  it("is the logger's five, most severe first", () => {
    const expected: Level[] = ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"];
    expect([...LEVELS]).toEqual(expected);
  });
});
