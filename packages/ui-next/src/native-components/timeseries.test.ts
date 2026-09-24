import { afterEach, describe, expect, it } from "vitest";
import { breaches, downsample, formatValue, MAX_DRAWN_POINTS, TIME_STEPS, timeTicks, unitName, valueTicks, yTicks } from "./timeseries";

describe("timeTicks", () => {
  const zone = process.env.TZ;
  afterEach(() => { if (zone === undefined) delete process.env.TZ; else process.env.TZ = zone; });
  const MIN = 60_000, HOUR = 60 * MIN, DAY = 24 * HOUR;
  const start = Date.UTC(2026, 8, 24, 12, 0, 0);
  const local = (time: number) => new Date(time);
  it("steps through a fixed ladder of round intervals", () => {
    expect(TIME_STEPS.slice(0, 3)).toEqual([1, 2, 5]);
    for (const step of [1000, 15_000, 30 * MIN, 3 * HOUR, 12 * HOUR, DAY, 7 * DAY]) expect(TIME_STEPS).toContain(step);
    expect([...TIME_STEPS].sort((a, b) => a - b)).toEqual(TIME_STEPS);
  });
  it("puts a 5 min + 1 ms range's ticks on whole minutes, never between them", () => {
    const { step, ticks } = timeTicks(start + 7_000, start + 7_000 + 5 * MIN + 1, () => 5);
    expect(step % MIN).toBe(0);
    expect(ticks.length).toBeGreaterThanOrEqual(2);
    expect(ticks.every(tick => tick % MIN === 0)).toBe(true);
  });
  it("lands a 1 h range on 5, 10 or 15 minute marks", () => {
    const { step, ticks } = timeTicks(start + 3 * MIN + 17_000, start + HOUR + 3 * MIN + 17_000, () => 5);
    expect([5 * MIN, 10 * MIN, 15 * MIN]).toContain(step);
    expect(ticks.every(tick => local(tick).getMinutes() % (step / MIN) === 0 && tick % MIN === 0)).toBe(true);
  });
  it("lands a 7-day range on local midnights or 12-hour boundaries, even east of UTC", () => {
    process.env.TZ = "Asia/Kolkata";
    const { step, ticks } = timeTicks(start + 5 * HOUR + 1234, start + 7 * DAY + 5 * HOUR + 1234, () => 5);
    expect([12 * HOUR, DAY, 2 * DAY]).toContain(step);
    expect(ticks.every(tick => local(tick).getHours() % 12 === 0 && local(tick).getMinutes() === 0 && local(tick).getSeconds() === 0)).toBe(true);
  });
  it("keeps a 1 ms range to its two whole-millisecond ends", () => {
    expect(timeTicks(start, start + 1, () => 5).ticks).toEqual([start, start + 1]);
  });
  it("uses fewer ticks when fewer fit, but never fewer than two", () => {
    const wide = timeTicks(start, start + DAY, () => 6).ticks.length;
    const narrow = timeTicks(start, start + DAY, () => 3).ticks.length;
    expect(narrow).toBeLessThan(wide);
    expect(narrow).toBeGreaterThanOrEqual(2);
    expect(timeTicks(start, start + DAY + HOUR, () => 2).ticks.length).toBeGreaterThanOrEqual(2);
  });
  it("thins the finer step to what fits when the fitting step leaves one tick", () => {
    // 5 days: 7-day steps give one tick, 2-day steps three; only two fit.
    const { step, ticks } = timeTicks(start, start + 5 * DAY, () => 2);
    expect(step).toBe(2 * DAY);
    expect(ticks).toHaveLength(2);
    expect(ticks.every(tick => local(tick).getHours() === 0)).toBe(true);
  });
  it("labels the true instants across a fall-back, without repeating one", () => {
    process.env.TZ = "America/New_York";
    const { ticks } = timeTicks(Date.parse("2026-11-01T04:10:00Z"), Date.parse("2026-11-01T08:10:00Z"), () => 6);
    expect(new Set(ticks).size).toBe(ticks.length);
    expect(ticks.every(tick => local(tick).getMinutes() === 0)).toBe(true);
  });
  it("puts month steps on the 1st and year steps on 1 January", () => {
    const months = timeTicks(start, start + 200 * DAY, () => 5);
    expect(months.ticks.every(tick => local(tick).getDate() === 1 && local(tick).getHours() === 0)).toBe(true);
    const years = timeTicks(start, start + 40 * 365 * DAY, () => 5);
    expect(years.ticks.length).toBeGreaterThanOrEqual(2);
    expect(years.ticks.length).toBeLessThanOrEqual(5);
    expect(years.ticks.every(tick => local(tick).getMonth() === 0 && local(tick).getDate() === 1)).toBe(true);
  });
  it("reaches the largest valid range", () => {
    const { ticks } = timeTicks(0, 8.64e15, () => 5);
    expect(ticks.length).toBeGreaterThanOrEqual(2);
    expect(ticks.every(Number.isFinite)).toBe(true);
  });
});

/** Production formats in the reader's locale; expectations are written in en-US and translated to it. */
const parts = new Intl.NumberFormat(undefined).formatToParts(1234.5);
const group = parts.find(part => part.type === "group")?.value ?? ",";
const decimal = parts.find(part => part.type === "decimal")?.value ?? ".";
const localized = (text: string) => text.replace(/(\d)([.,])(?=\d)/g, (_, digit: string, separator: string) => digit + (separator === "." ? decimal : group));

const count = (segments: Array<Array<[number, number]>>) => segments.reduce((total, segment) => total + segment.length, 0);

describe("downsample", () => {
  const times = Array.from({ length: 1000 }, (_, i) => i * 1000);
  it("keeps every sample under the cap and breaks lines at gaps", () => {
    expect(downsample([0, 1, 2, 3, 4], [1, 2, null, 4, 5], 400)).toEqual([[[0, 1], [1, 2]], [[3, 4], [4, 5]]]);
    expect(downsample([0, 1, 2], [null, null, null], 400)).toEqual([]);
  });
  it("respects the cap and keeps both extremes of a long series", () => {
    const values = times.map((_, i) => Math.sin(i / 7) * 10 + (i === 613 ? 90 : 0) + (i === 211 ? -70 : 0));
    const segments = downsample(times, values, MAX_DRAWN_POINTS);
    expect(count(segments)).toBeLessThanOrEqual(MAX_DRAWN_POINTS);
    const drawn = segments.flat();
    expect(drawn).toContainEqual([613_000, values[613]]);
    expect(drawn).toContainEqual([211_000, values[211]]);
    expect(drawn.every(([time, value]) => values[time / 1000] === value)).toBe(true);
    expect(drawn.map(([time]) => time)).toEqual([...drawn.map(([time]) => time)].sort((a, b) => a - b));
  });
  it("keeps a lone spike in every bucket size", () => {
    for (const cap of [2, 3, 10, 399]) {
      const values = times.map((_, i) => i === 997 ? 5 : 0);
      const drawn = downsample(times, values, cap).flat();
      expect(drawn.length).toBeLessThanOrEqual(cap);
      expect(drawn).toContainEqual([997_000, 5]);
    }
  });
  it("still breaks the line at a gap inside a bucket", () => {
    const values = times.map((_, i) => i >= 500 && i < 510 ? null : i);
    expect(downsample(times, values, 100).length).toBe(2);
  });
});

describe("formatValue", () => {
  it.each([
    [0, "number", "0"], [1234.5, "number", "1,235"], [97.3, "percent", "97.3%"], [0.973, "ratio", "97.3%"],
    [512, "bytes", "512 B"], [1536, "bytes", "1.5 KiB"], [3 * 1024 ** 3, "bytes", "3 GiB"], [2048, "bytesPerSecond", "2 KiB/s"],
    [0.25, "seconds", "250 ms"], [0.0005, "seconds", "500 µs"], [42, "seconds", "42 s"], [90, "seconds", "1.5 min"], [7200, "seconds", "2 h"],
    [0.25, "cores", "0.25 cores"], [1, "cores", "1 core"], [12.5, "perSecond", "12.5/s"], [-2048, "bytes", "-2 KiB"],
  ] as const)("formats %s as %s: %s", (value, unit, text) => expect(formatValue(value, unit)).toBe(localized(text)));
  it.each([
    [1e308, "number", "1E308"], [-1.5e9, "number", "-1.5E9"], [5e-5, "number", "5E-5"], [2.5e12, "perSecond", "2.5E12/s"],
    [3e-7, "cores", "3E-7 cores"], [1e9, "percent", "1E9%"],
    [999_999_999, "number", "1,000,000,000"], [1e-4, "number", "0.0001"], [123456, "number", "123,500"],
  ] as const)("rounds %s in %s to a short figure, scientific only outside 1e-4 to 1e9: %s", (value, unit, text) =>
    expect(formatValue(value, unit)).toBe(localized(text)));
  it("never prints a signed zero", () => {
    expect(formatValue(-0, "number")).toBe("0");
    expect(formatValue(-0 * 1e-7, "cores", true)).toBe("0 cores");
  });
  it("keeps every digit of a huge value in exact mode", () => {
    expect(formatValue(1.5e9, "number", true)).toBe(localized("1,500,000,000"));
  });
  it.each([
    [0.50004, "cores", "0.50004 cores"], [0.1, "number", "0.1"], [1234.5, "number", "1,234.5"], [0.973, "ratio", "97.3%"],
    [0.123456789, "percent", "0.123456789%"], [1500, "bytes", "1.46484375 KiB"], [90, "seconds", "90 s"], [3725.5, "seconds", "3,725.5 s"],
    [0.00025, "seconds", "250 µs"], [0.0125, "seconds", "12.5 ms"],
  ] as const)("formats %s exactly as %s: %s", (value, unit, text) => expect(formatValue(value, unit, true)).toBe(localized(text)));
  it("names units for the axis", () => {
    expect(unitName("cores")).toBe("cores");
    expect(unitName("ratio")).toBe("%");
    expect(unitName("number")).toBe("");
  });
});

describe("axis", () => {
  const ends = (ticks: number[]) => [ticks[0], ticks[ticks.length - 1]];
  it("anchors non-negative data at zero and includes thresholds", () => {
    const [lo, hi] = ends(yTicks([[3, 4, null]], [{ label: "Limit", value: 10, direction: "above", tone: "sev" }], "number"));
    expect(lo).toBe(0);
    expect(hi).toBeGreaterThanOrEqual(10);
  });
  it("spans negative data and a flat series, always including zero", () => {
    expect(ends(yTicks([[-5, 5]], [], "number"))[0]).toBeLessThanOrEqual(-5);
    expect(ends(yTicks([[-5, -3]], [], "number"))[1]).toBe(0);
    const [lo, hi] = ends(yTicks([[0, 0]], [], "number"));
    expect(hi).toBeGreaterThan(lo);
  });
  it.each([[-1e308, 1e308], [0, 1.7e308], [-Number.MAX_VALUE, Number.MAX_VALUE], [0, Number.MIN_VALUE], [-Number.MIN_VALUE, 1e-310]])("keeps finite ticks around extreme samples %s and %s", (lo, hi) => {
    const ticks = yTicks([[lo, hi]], [], "number");
    expect(ticks.length).toBeGreaterThanOrEqual(2);
    expect(ticks.every(Number.isFinite)).toBe(true);
    expect(ticks[0]).toBeLessThanOrEqual(lo);
    expect(ticks[ticks.length - 1]).toBeGreaterThanOrEqual(hi);
  });
  it.each([11.5, 3.1, 0.73, 97, 1234])("ends the axis on its own ticks, evenly stepped, for a maximum of %s", max => {
    const ticks = yTicks([[0, max]], [], "cores");
    expect(ticks[ticks.length - 1]).toBeGreaterThanOrEqual(max);
    const steps = ticks.slice(1).map((tick, index) => Number((tick - ticks[index]).toPrecision(9)));
    expect(new Set(steps).size).toBe(1);
    expect(ticks[ticks.length - 1] - steps[0]).toBeLessThan(max);
  });
  it("chooses round ticks, in binary steps for bytes", () => {
    expect(valueTicks(0, 100, "percent")).toEqual([0, 20, 40, 60, 80, 100]);
    expect(valueTicks(0, 3 * 1024 ** 3, "bytes").map(tick => formatValue(tick, "bytes"))).toEqual(["0 B", "1 GiB", "2 GiB", "3 GiB"].map(localized));
  });
});

describe("breaches", () => {
  it("counts samples strictly beyond the threshold in its direction and ignores gaps", () => {
    expect(breaches([1, 5, 6, null, 9], { label: "High", value: 5, direction: "above", tone: "sev" })).toBe(2);
    expect(breaches([1, 5, 6, null, 0], { label: "Low", value: 5, direction: "below", tone: "warn" })).toBe(2);
  });
});
