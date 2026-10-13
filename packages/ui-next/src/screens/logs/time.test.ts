import { describe, it, expect } from "vitest";
import { VOLUME_STEPS } from "@srelens/core";
import { clockTime, DAY, dateTime, MINUTE, niceCeil, rangeLabel, spansDays, stepLabel, tickLabels, timeTicks } from "./time";

// Local-time constructors, so these hold in every zone.
const at = (day: number, h = 0, m = 0, s = 0) => new Date(2026, 9, day, h, m, s).getTime();

/**
 * Runs `body` in `zone`, then hands back the zone the process was in, by name:
 * on Windows, deleting TZ leaves Node in the pinned zone. (core's logQuery tests
 * do the same.)
 */
function inZone(zone: string, body: () => void): void {
  const found = Intl.DateTimeFormat().resolvedOptions().timeZone;
  process.env.TZ = zone;
  try {
    body();
  } finally {
    process.env.TZ = found;
  }
}

describe("the time units", () => {
  it("are defined here once, for the axis and the histogram to share", () => {
    expect(MINUTE).toBe(60_000);
    expect(DAY).toBe(86_400_000);
  });
});

describe("timeTicks, steps", () => {
  it("cuts on the round steps the histogram's bars are cut on", () => {
    // Ticks one step apart, whichever step fits: each is a step core's bucketing also offers.
    const gaps = new Set<number>();
    for (const span of [20_000, 400_000, 5 * 3_600_000, 3 * DAY]) {
      const ticks = timeTicks(at(12), at(12) + span);
      if (ticks.length > 1) gaps.add(ticks[1] - ticks[0]);
    }
    expect(gaps.size).toBeGreaterThan(1);
    for (const gap of gaps) expect(VOLUME_STEPS).toContain(gap);
  });
});

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
  it("is dated in a 23-hour day too, from one local midnight to the next", () => {
    inZone("Europe/Berlin", () => {
      // Guard: the clocks jump on 29 March, so that day has 23 hours and a plain `>= DAY` test misses it.
      const from = new Date(2026, 2, 29).getTime();
      const to = new Date(2026, 2, 30).getTime();
      expect(to - from).toBe(23 * 3_600_000);
      expect(rangeLabel({ from, to })).toBe("2026-03-29 00:00:00 – 2026-03-30 00:00:00");
      expect(spansDays(from, to)).toBe(true);
    });
  });

  it("leaves a window that merely ends at midnight undated, even in a 23-hour day", () => {
    inZone("Europe/Berlin", () => {
      const from = new Date(2026, 2, 29, 23).getTime();
      const to = new Date(2026, 2, 30).getTime();
      expect(rangeLabel({ from, to })).toBe("23:00:00 – 00:00:00");
    });
  });

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

describe("niceCeil", () => {
  // The smallest of 1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8 times a power of ten that is at least n.
  it.each([
    [0, 1],
    [1, 1],
    [2, 2],
    [3, 3],
    [4, 4],
    [5, 5],
    [7, 8],
    [9, 10],
    [11, 12],
    [13, 15],
    [16, 20],
    [21, 25],
    [110, 120],
    [450, 500],
    [1000, 1000],
    [1001, 1200],
    [1_300_000, 1_500_000],
  ])("rounds %d up to %d", (n, nice) => {
    expect(niceCeil(n)).toBe(nice);
  });
});

describe("timeTicks", () => {
  const minutes = (ticks: number[]) => ticks.map((t) => new Date(t).getMinutes());

  it("puts at most six ticks on round minutes across ten minutes", () => {
    const ticks = timeTicks(at(12, 10, 3, 20), at(12, 10, 13, 20));
    expect(ticks).toEqual([at(12, 10, 4), at(12, 10, 6), at(12, 10, 8), at(12, 10, 10), at(12, 10, 12)]);
  });

  it("keeps the seconds of a span under a minute", () => {
    const ticks = timeTicks(at(12, 10, 0, 3), at(12, 10, 0, 28));
    expect(ticks.length).toBeLessThanOrEqual(6);
    expect(ticks.length).toBeGreaterThan(1);
    // Every tick is a whole number of five seconds on the clock, and not every one is on a minute.
    for (const t of ticks) expect(new Date(t).getSeconds() % 5).toBe(0);
    expect(ticks.some((t) => new Date(t).getSeconds() !== 0)).toBe(true);
  });

  it("includes both ends when they are round, and never a tick outside", () => {
    const from = at(12, 10, 0);
    const to = at(12, 10, 10);
    const ticks = timeTicks(from, to);
    expect(ticks[0]).toBeGreaterThanOrEqual(from);
    expect(ticks[ticks.length - 1]).toBeLessThanOrEqual(to);
    expect(ticks).toContain(from);
    expect(ticks).toContain(to);
  });

  it("honours a smaller maxTicks by taking a wider interval", () => {
    expect(timeTicks(at(12, 10, 0), at(12, 10, 10), 3)).toEqual([at(12, 10, 0), at(12, 10, 5), at(12, 10, 10)]);
  });

  it("gives whole days to a span of weeks rather than none", () => {
    const ticks = timeTicks(at(1), at(1) + 45 * 86_400_000);
    expect(ticks.length).toBeGreaterThan(1);
    expect(ticks.length).toBeLessThanOrEqual(6);
    for (const t of ticks) expect(clockTime(t)).toBe("00:00:00");
  });

  it("reads a maxTicks that is not a finite number as the default six", () => {
    const from = at(12, 10, 0);
    const to = at(12, 10, 10);
    for (const bad of [Number.NaN, Infinity, -Infinity]) {
      expect(timeTicks(from, to, bad), `maxTicks ${bad}`).toEqual(timeTicks(from, to));
    }
  });

  it("keeps a span as wide as a Date can hold to a handful of ticks, and returns", () => {
    const ticks = timeTicks(-8.64e15, 8.64e15);
    expect(ticks.length).toBeGreaterThan(1);
    expect(ticks.length).toBeLessThanOrEqual(6);
    expect(ticks.every((t) => Number.isFinite(t))).toBe(true);
    expect([...ticks].sort((a, b) => a - b)).toEqual(ticks);
  });

  it("returns nothing for an empty or backwards span", () => {
    expect(timeTicks(at(12), at(12))).toEqual([]);
    expect(timeTicks(at(12, 1), at(12))).toEqual([]);
    expect(timeTicks(Number.NaN, at(12))).toEqual([]);
  });

  it("lands on round local times in a zone whose offset is not a whole hour", () => {
    inZone("Asia/Kathmandu", () => {
      // 10:10Z is 15:55 at +05:45, and a UTC grid would put a half-hour tick on :15 or :45.
      expect(new Date(Date.UTC(2026, 9, 12, 10, 10)).getTimezoneOffset()).toBe(-345);
      const ticks = timeTicks(Date.UTC(2026, 9, 12, 10, 10), Date.UTC(2026, 9, 12, 12, 10));
      expect(ticks.length).toBeGreaterThan(1);
      for (const m of minutes(ticks)) expect(m % 30).toBe(0);
      for (const t of ticks) expect(new Date(t).getSeconds()).toBe(0);
    });
  });

  it("never repeats a tick across a skipped hour", () => {
    inZone("Europe/Berlin", () => {
      // 02:00-03:00 on 29 March does not exist, so 02:00 and 02:30 would both be the transition.
      const ticks = timeTicks(Date.UTC(2026, 2, 29, 0, 0), Date.UTC(2026, 2, 29, 2, 30), 12);
      expect(new Set(ticks).size).toBe(ticks.length);
      expect([...ticks].sort((a, b) => a - b)).toEqual(ticks);
    });
  });
});

describe("tickLabels", () => {
  it("names a tick that starts a local day by its date", () => {
    const ticks = timeTicks(at(12, 23), at(13, 1));
    expect(ticks).toEqual([at(12, 23), at(12, 23, 30), at(13), at(13, 0, 30), at(13, 1)]);
    expect(tickLabels(ticks)).toEqual(["23:00", "23:30", "10-13 00:00", "00:30", "01:00"]);
  });

  it("keeps the seconds when the ticks are under a minute apart", () => {
    const ticks = timeTicks(at(12, 10, 0, 3), at(12, 10, 0, 28));
    expect(tickLabels(ticks)[0]).toMatch(/^10:00:[0-9]{2}$/);
  });

  it("dates each tick of a span of days", () => {
    expect(tickLabels([at(12), at(13), at(14)])).toEqual(["10-12 00:00", "10-13 00:00", "10-14 00:00"]);
  });
});
