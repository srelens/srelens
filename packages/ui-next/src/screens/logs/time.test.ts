import { describe, it, expect } from "vitest";
import { clockTime, dateTime, rangeLabel, spansDays, stepLabel } from "./time";

// Local-time constructors, so these hold in every zone.
const at = (day: number, h = 0, m = 0, s = 0) => new Date(2026, 9, day, h, m, s).getTime();

describe("clockTime", () => {
  it("pads each part to two digits", () => {
    expect(clockTime(at(12, 1, 2, 3))).toBe("01:02:03");
  });
});

describe("rangeLabel", () => {
  it("leaves the date off a window within one day", () => {
    expect(rangeLabel({ from: at(12, 10, 33), to: at(12, 10, 34, 30) })).toBe("10:33:00 – 10:34:30");
  });

  it("dates both ends once the window crosses midnight", () => {
    expect(rangeLabel({ from: at(12, 23, 59, 30), to: at(13, 0, 0, 30) })).toBe(
      "2026-10-12 23:59:30 – 2026-10-13 00:00:30",
    );
  });

  it("keeps a window that ends exactly at midnight on its own day", () => {
    // `to` is exclusive: the last instant inside is 23:59:59.999.
    expect(rangeLabel({ from: at(12, 23), to: at(13) })).toBe("23:00:00 – 00:00:00");
  });
});

describe("a window of a whole day", () => {
  it("is dated, even though it ends on its own day's midnight", () => {
    expect(rangeLabel({ from: at(12), to: at(12) + 86_400_000 })).toBe("2026-10-12 00:00:00 – 2026-10-13 00:00:00");
  });

  it("is a span of days, and one second short of that is not", () => {
    expect(spansDays(at(12), at(12) + 86_400_000)).toBe(true);
    expect(spansDays(at(12), at(12) + 86_400_000 - 1000)).toBe(false);
  });
});

describe("dateTime", () => {
  it("names the day and the minute", () => {
    expect(dateTime(at(12, 10, 33, 45))).toBe("2026-10-12 10:33");
  });

  it("keeps the seconds when asked", () => {
    expect(dateTime(at(12, 10, 33, 45), true)).toBe("2026-10-12 10:33:45");
  });
});

describe("stepLabel", () => {
  it.each([
    [1_000, "1s"],
    [30_000, "30s"],
    [60_000, "1m"],
    [15 * 60_000, "15m"],
    [3_600_000, "1h"],
    [12 * 3_600_000, "12h"],
    [86_400_000, "1d"],
    [3 * 86_400_000, "3d"],
  ])("names %d ms as %s", (step, label) => {
    expect(stepLabel(step)).toBe(label);
  });
});
