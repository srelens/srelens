import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NativeComponent } from "./NativeComponent";
import { MAX_DRAWN_POINTS } from "./timeseries";

// Restored here, not at the end of a test, so a failed assertion cannot leak a stub into later cases.
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

/** Production formats in the reader's locale; expectations are written in en-US and translated to it. */
const parts = new Intl.NumberFormat(undefined).formatToParts(1234.5);
const group = parts.find(part => part.type === "group")?.value ?? ",";
const decimal = parts.find(part => part.type === "decimal")?.value ?? ".";
const localized = (text: string) => text.replace(/(\d)([.,])(?=\d)/g, (_, digit: string, separator: string) => digit + (separator === "." ? decimal : group));
const start = Date.UTC(2026, 8, 24, 12);
const minute = 60_000;
const chart = (data: Record<string, unknown> = {}) => ({ version: 1, type: "Timeseries", data: {
  label: "CPU usage", unit: "cores", range: { start, end: start + 4 * minute },
  times: [start, start + minute, start + 2 * minute, start + 3 * minute],
  series: [{ name: "web", values: [0.2, null, 0.7, 0.4] }, { name: "api", values: [0, 0.1, 0.3, 0.6] }],
  thresholds: [{ label: "Limit", value: 0.5, direction: "above", tone: "sev" }],
  ...data,
} });
const showValues = () => fireEvent.click(screen.getByRole("button", { name: /Show values table/ }));

describe("Timeseries", () => {
  it("draws a labelled plot with its unit and time range in text", () => {
    render(<NativeComponent label="Pod CPU" payload={chart()}/>);
    const plot = screen.getByRole("img");
    expect(plot.getAttribute("aria-label")).toMatch(/CPU usage \(cores\).*2 series/);
    expect(plot.getAttribute("aria-label")).toContain("Limit");
    expect(screen.getByText("CPU usage (cores)")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });
  it("names each series with a pattern as well as a colour", () => {
    const { container } = render(<NativeComponent label="Pod CPU" payload={chart()}/>);
    const legend = screen.getByRole("list", { name: "Series" });
    expect(within(legend).getByText("web")).toBeTruthy();
    expect(within(legend).getByText("api")).toBeTruthy();
    const dashes = [...container.querySelectorAll("[data-dash]")].map(line => line.getAttribute("data-dash"));
    expect(new Set(dashes).size).toBe(2);
    expect(within(legend).getByText(localized("latest 0.4 cores · min 0.2 cores · max 0.7 cores"))).toBeTruthy();
  });
  it("says in text which series breach a threshold", () => {
    render(<NativeComponent label="Pod CPU" payload={chart()}/>);
    const legend = screen.getByRole("list", { name: "Series" });
    expect(within(legend).getByText("Above Limit: 1 of 3 samples")).toBeTruthy();
    expect(within(legend).getByText("Above Limit: 1 of 4 samples")).toBeTruthy();
    expect(within(screen.getByRole("list", { name: "Thresholds" })).getByText(localized("Limit: above 0.5 cores"))).toBeTruthy();
  });
  it("offers a values table that matches the data, gaps and breaches included", () => {
    render(<NativeComponent label="Pod CPU" payload={chart()}/>);
    expect(screen.queryByRole("table")).toBeNull();
    showValues();
    const table = screen.getByRole("table", { name: /CPU usage/ });
    const rows = within(table).getAllByRole("row");
    expect(rows.map(row => within(row).queryAllByRole("columnheader").map(cell => cell.textContent))[0]).toEqual(["Time", "web", "api"]);
    const body = rows.slice(1).map(row => [...row.children].slice(1).map(cell => cell.textContent));
    expect(body).toEqual([
      ["0.2 cores", "0 cores"], ["No sample", "0.1 cores"], ["0.7 cores · above Limit", "0.3 cores"], ["0.4 cores", "0.6 cores · above Limit"],
    ].map(row => row.map(localized)));
    expect(rows[1].querySelector("time")?.getAttribute("dateTime")).toBe(new Date(start).toISOString());
    expect(screen.getByRole("button", { name: /Hide values table/ }).getAttribute("aria-expanded")).toBe("true");
  });
  it("pages a long values table without dropping samples", () => {
    const times = Array.from({ length: 1000 }, (_, i) => start + i * 1000);
    render(<NativeComponent label="Pod CPU" payload={chart({ range: { start, end: start + 1000 * 1000 }, times,
      series: [{ name: "web", values: times.map((_, i) => i) }], thresholds: [] })}/>);
    fireEvent.click(screen.getByRole("button", { name: localized("Show values table (1,000 samples)") }));
    expect(within(screen.getByRole("table")).getAllByRole("row")).toHaveLength(21);
    expect(screen.getByRole("button", { name: /Show 20 more \(980 remaining\)/ })).toBeTruthy();
  });
  it("draws at most the cap per series, keeps its extremes and says it reduced them", () => {
    const times = Array.from({ length: 1000 }, (_, i) => start + i * 1000);
    const values = times.map((_, i) => i === 777 ? 50 : i % 3);
    const { container } = render(<NativeComponent label="Pod CPU" payload={chart({ range: { start, end: start + 1000 * 1000 }, times,
      series: [{ name: "web", values }], thresholds: [], unit: "number" })}/>);
    // Line vertices plus isolated-sample markers, in the plot only (the legend swatch draws a marker too).
    const drawn = [...container.querySelectorAll("svg[role=img] path[data-dash]")].reduce((total, path) => total + (path.getAttribute("d")!.match(/[ML]/g) ?? []).length, 0)
      + container.querySelectorAll("svg[role=img] [data-marker]").length;
    expect(drawn).toBeLessThanOrEqual(MAX_DRAWN_POINTS);
    expect(screen.getByText(/latest 0 · min 0 · max 50/)).toBeTruthy();
    expect(screen.getByText(/reduced to at most 400 points/)).toBeTruthy();
  });
  it("prints table values and thresholds with the digits that decide a breach", () => {
    render(<NativeComponent label="Pod CPU" payload={chart({ times: [start], range: { start, end: start + minute },
      series: [{ name: "web", values: [0.50004] }], thresholds: [{ label: "Limit", value: 0.50002, direction: "above", tone: "sev" }] })}/>);
    expect(within(screen.getByRole("list", { name: "Thresholds" })).getByText(localized("Limit: above 0.50002 cores"))).toBeTruthy();
    showValues();
    const cell = within(screen.getByRole("table")).getAllByRole("cell")[0];
    expect(cell.textContent).toBe(localized("0.50004 cores · above Limit"));
  });
  it.each([
    ["one second", [start, start + 1000], false],
    ["one millisecond", [start, start + 1], true],
  ] as const)("tells table rows %s apart over a long range", (_, times, milliseconds) => {
    render(<NativeComponent label="Pod CPU" payload={chart({ range: { start, end: start + 7 * 24 * 60 * minute }, times: [...times],
      series: [{ name: "web", values: [1, 2] }], thresholds: [] })}/>);
    showValues();
    const shown = within(screen.getByRole("table")).getAllByRole("rowheader").map(cell => cell.textContent);
    expect(shown[0]).not.toBe(shown[1]);
    expect(/[.,]000\b/.test(shown[0]!)).toBe(milliseconds);
  });
  describe("across a daylight-saving fall-back", () => {
    const zone = process.env.TZ;
    afterEach(() => { if (zone === undefined) delete process.env.TZ; else process.env.TZ = zone; });
    // 05:30Z and 06:30Z are both 01:30 in New York on 1 November 2026: EDT, then EST.
    const repeated = [Date.parse("2026-11-01T05:30:00Z"), Date.parse("2026-11-01T06:30:00Z")];
    it("tells the repeated hour apart by its UTC offset in table rows and axis labels", () => {
      process.env.TZ = "America/New_York";
      const { container } = render(<NativeComponent label="Pod CPU" payload={chart({ range: { start: Date.parse("2026-11-01T05:00:00Z"), end: Date.parse("2026-11-01T07:00:00Z") },
        times: repeated, series: [{ name: "web", values: [1, 2] }], thresholds: [] })}/>);
      const ticks = [...container.querySelectorAll("text[data-axis='time']")].map(tick => tick.textContent);
      expect(new Set(ticks).size).toBe(ticks.length);
      showValues();
      const rows = within(screen.getByRole("table")).getAllByRole("rowheader").map(cell => cell.textContent!);
      expect(rows[0]).not.toBe(rows[1]);
      expect(rows[0]).toMatch(/GMT-4/);
      expect(rows[1]).toMatch(/GMT-5/);
    });
    it("fits offset-bearing labels into the 352px peek across the fall-back", () => {
      process.env.TZ = "America/New_York";
      vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(352);
      vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
      const from = Date.parse("2026-11-01T04:00:00Z"), to = Date.parse("2026-11-01T09:45:00Z");
      const { container } = render(<NativeComponent label="Pod CPU" payload={chart({ range: { start: from, end: to },
        times: [from, to], series: [{ name: "web", values: [1, 2] }], thresholds: [] })}/>);
      const labels = [...container.querySelectorAll("text[data-axis='time']")].map(tick => tick.textContent!);
      // The step is chosen by the labels it will really draw, offsets included: two, not three.
      expect(labels).toHaveLength(2);
      expect(new Set(labels).size).toBe(labels.length);
      // Every label says its offset, the local-midnight date label included.
      expect(labels.every(label => /GMT-[45]/.test(label))).toBe(true);
      const grid = container.querySelector("line.native-timeseries-grid")!;
      const plotWidth = Number(grid.getAttribute("x2")) - Number(grid.getAttribute("x1"));
      expect(labels.reduce((total, label) => total + label.length * 7 + 16, 0)).toBeLessThanOrEqual(plotWidth);
    });
    const axisLabels = (from: string, to: string) => [...render(<NativeComponent label="Pod CPU" payload={chart({
      range: { start: Date.parse(from), end: Date.parse(to) }, times: [Date.parse(from), Date.parse(to)],
      series: [{ name: "web", values: [1, 2] }], thresholds: [] })}/>).container.querySelectorAll("text[data-axis='time']")].map(tick => tick.textContent!);
    it("shows the offset on every day-step label when the axis spans a DST change, and on none otherwise", () => {
      process.env.TZ = "America/New_York";
      const across = axisLabels("2026-10-28T12:00:00Z", "2026-11-04T12:00:00Z");
      expect(across.length).toBeGreaterThanOrEqual(2);
      expect(across.every(label => /GMT-[45]/.test(label))).toBe(true);
      cleanup();
      const within_ = axisLabels("2026-09-20T12:00:00Z", "2026-09-27T12:00:00Z");
      expect(within_.length).toBeGreaterThanOrEqual(2);
      expect(within_.some(label => /GMT/.test(label))).toBe(false);
    });
    it("fits year-bearing labels into the 352px peek across a year boundary", () => {
      process.env.TZ = "UTC";
      vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(352);
      vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
      const from = Date.parse("2026-12-29T00:00:00Z"), to = Date.parse("2027-01-03T00:00:00Z");
      const { container } = render(<NativeComponent label="Pod CPU" payload={chart({ range: { start: from, end: to },
        times: [from, to], series: [{ name: "web", values: [1, 2] }], thresholds: [] })}/>);
      const labels = [...container.querySelectorAll("text[data-axis='time']")].map(tick => tick.textContent!);
      expect(labels.length).toBeGreaterThanOrEqual(2);
      expect(labels.some(label => /2026/.test(label)) && labels.some(label => /2027/.test(label))).toBe(true);
      const grid = container.querySelector("line.native-timeseries-grid")!;
      const plotWidth = Number(grid.getAttribute("x2")) - Number(grid.getAttribute("x1"));
      expect(labels.reduce((total, label) => total + label.length * 7 + 16, 0)).toBeLessThanOrEqual(plotWidth);
    });
    it("leaves the offset out when every time shown shares one", () => {
      process.env.TZ = "America/New_York";
      const { container } = render(<NativeComponent label="Pod CPU" payload={chart({ thresholds: [] })}/>);
      expect([...container.querySelectorAll("text[data-axis='time']")].some(tick => /GMT/.test(tick.textContent!))).toBe(false);
      showValues();
      expect(within(screen.getByRole("table")).getAllByRole("rowheader").some(cell => /GMT/.test(cell.textContent!))).toBe(false);
    });
  });
  it("keeps a sub-second range's axis labels and summary distinct with milliseconds", () => {
    const { container } = render(<NativeComponent label="Pod CPU" payload={chart({ range: { start, end: start + 1 }, times: [start, start + 1],
      series: [{ name: "web", values: [1, 2] }], thresholds: [] })}/>);
    const ticks = [...container.querySelectorAll("text[data-axis='time']")].map(tick => tick.textContent!);
    expect(ticks.length).toBeGreaterThanOrEqual(2);
    expect(new Set(ticks).size).toBe(ticks.length);
    expect(ticks[0]).toMatch(/[.,]000\b/);
    expect(ticks[ticks.length - 1]).toMatch(/[.,]001\b/);
    const summary = screen.getByRole("img").getAttribute("aria-label")!;
    const [, from, to] = summary.match(/from (.*) to (.*?)\./)!;
    expect(from).not.toBe(to);
  });
  describe("time axis on round steps", () => {
    const axis = (container: HTMLElement) => [...container.querySelectorAll("text[data-axis='time']")]
      .map(tick => ({ text: tick.textContent!, time: Date.parse(tick.getAttribute("data-time")!) }));
    const minutes = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
    const draw = (from: number, to: number) => render(<NativeComponent label="Pod CPU" payload={chart({ range: { start: from, end: to },
      times: [from, to], series: [{ name: "web", values: [1, 2] }], thresholds: [] })}/>).container;
    it("labels a 5 min + 1 ms range at 640px only with minutes its ticks are on", () => {
      const ticks = axis(draw(start + 7_000, start + 7_000 + 5 * minute + 1));
      expect(ticks.length).toBeGreaterThanOrEqual(2);
      for (const tick of ticks) {
        expect(tick.time % minute).toBe(0);
        expect(tick.text).toBe(minutes.format(tick.time));
      }
    });
    it("marks a 1 h range at 5, 10 or 15 minutes", () => {
      const ticks = axis(draw(start + 3 * minute + 17_000, start + 63 * minute + 17_000));
      const gaps = ticks.slice(1).map((tick, index) => tick.time - ticks[index].time);
      expect(new Set(gaps).size).toBe(1);
      expect([5, 10, 15].map(step => step * minute)).toContain(gaps[0]);
      expect(ticks.every(tick => new Date(tick.time).getMinutes() % (gaps[0] / minute) === 0)).toBe(true);
    });
    it("marks a 7-day range at local days or half days", () => {
      const ticks = axis(draw(start + 5 * 60 * minute, start + 7 * 24 * 60 * minute + 5 * 60 * minute));
      expect(ticks.length).toBeGreaterThanOrEqual(2);
      expect(ticks.every(tick => new Date(tick.time).getHours() % 12 === 0 && new Date(tick.time).getMinutes() === 0)).toBe(true);
    });
    it("fits its labels into the 352px peek without crowding", () => {
      vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(352);
      vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
      for (const [from, to] of [[start, start + 1], [start, start + 5 * minute + 1], [start, start + 7 * 24 * 60 * minute]]) {
        const container = draw(from, to);
        const ticks = axis(container);
        expect(ticks.length).toBeGreaterThanOrEqual(2);
        expect(new Set(ticks.map(tick => tick.text)).size).toBe(ticks.length);
        expect(ticks.reduce((total, tick) => total + tick.text.length * 7 + 16, 0)).toBeLessThanOrEqual(352);
        cleanup();
      }
    });
  });
  it("shows no milliseconds on an ordinary range", () => {
    const { container } = render(<NativeComponent label="Pod CPU" payload={chart({ thresholds: [] })}/>);
    const shown = [...container.querySelectorAll("text[data-axis='time']")].map(tick => tick.textContent!)
      .concat(screen.getByRole("img").getAttribute("aria-label")!);
    expect(shown.some(text => /\d[.,]\d{3}\b/.test(text))).toBe(false);
  });
  it("draws extreme and subnormal finite samples without losing the axis", () => {
    for (const values of [[-1e308, 1e308], [0, 1.7e308], [-Number.MAX_VALUE, Number.MAX_VALUE], [0, Number.MIN_VALUE], [-Number.MIN_VALUE, 1e-310]]) {
      const { container, unmount } = render(<NativeComponent label="Pod CPU" payload={chart({ times: [start, start + minute],
        series: [{ name: "web", values }], thresholds: [], unit: "number" })}/>);
      const geometry = [...container.querySelectorAll("path[data-series], line, text[data-axis='value']")]
        .map(element => `${element.getAttribute("d") ?? ""} ${element.getAttribute("y1") ?? ""} ${element.getAttribute("y") ?? ""}`).join(" ");
      expect(geometry).not.toMatch(/NaN|Infinity/);
      expect(container.querySelectorAll("text[data-axis='value']").length).toBeGreaterThanOrEqual(2);
      unmount();
    }
  });
  it("labels an extreme value axis in short scientific figures and keeps the table exact", () => {
    const { container } = render(<NativeComponent label="Pod CPU" payload={chart({ times: [start, start + minute],
      series: [{ name: "web", values: [0, 1e308] }], thresholds: [], unit: "number" })}/>);
    const labels = [...container.querySelectorAll("text[data-axis='value']")].map(label => label.textContent!);
    expect(labels.length).toBeGreaterThanOrEqual(2);
    expect(labels.every(label => label.length <= 8)).toBe(true);
    expect(labels).toContain(localized("1E308"));
    showValues();
    expect(within(screen.getByRole("table")).getAllByRole("cell")[1].textContent).toBe(new Intl.NumberFormat(undefined, { maximumSignificantDigits: 1 }).format(1e308));
  });
  it("keeps ordinary axis labels in plain figures", () => {
    const { container } = render(<NativeComponent label="Pod CPU" payload={chart({ thresholds: [] })}/>);
    expect([...container.querySelectorAll("text[data-axis='value']")].map(label => label.textContent))
      .toEqual(["0 cores", "0.2 cores", "0.4 cores", "0.6 cores", "0.8 cores"].map(localized));
  });
  it("tells isolated samples of different series apart by marker shape, in the plot and the legend", () => {
    const { container } = render(<NativeComponent label="Pod CPU" payload={chart({ thresholds: [],
      series: [{ name: "web", values: [null, 0.3, null, null] }, { name: "api", values: [null, 0.3, null, null] }] })}/>);
    const markers = [...container.querySelectorAll("svg[role='img'] [data-marker]")];
    expect(markers.map(marker => marker.getAttribute("data-series"))).toEqual(["web", "api"]);
    const shapes = markers.map(marker => marker.getAttribute("data-marker"));
    expect(new Set(shapes).size).toBe(2);
    expect(markers[0].getAttribute("d")).not.toBe(markers[1].getAttribute("d"));
    // Outlines only: a marker drawn later at the same point cannot paint over an earlier one.
    expect(markers.every(marker => marker.getAttribute("fill") === "none")).toBe(true);
    const legend = screen.getByRole("list", { name: "Series" });
    expect([...legend.querySelectorAll("[data-marker]")].map(marker => marker.getAttribute("data-marker"))).toEqual(shapes);
  });
  it("ends the value axis on its own gridlines, with none off the plot", () => {
    const { container } = render(<NativeComponent label="Pod CPU" payload={chart({ series: [{ name: "web", values: [0, 3, 11.5, 7] }], thresholds: [] })}/>);
    const labels = [...container.querySelectorAll("text[data-axis='value']")];
    expect(labels[labels.length - 1].textContent).toBe(localized("12 cores"));
    expect(labels.every(label => Number(label.getAttribute("y")) >= 0)).toBe(true);
  });
  it("marks a lone sample between gaps so it is not lost", () => {
    const { container } = render(<NativeComponent label="Pod CPU" payload={chart({ series: [{ name: "web", values: [null, 0.3, null, null] }] })}/>);
    expect(container.querySelectorAll("svg[role=img] [data-marker]")).toHaveLength(1);
  });
  it("keeps loading, failure and no data as three different answers", () => {
    const { rerender } = render(<NativeComponent label="Pod CPU" payload={chart()} state={{ status: "loading" }}/>);
    expect(screen.getByRole("status")).toBeTruthy();
    expect(screen.queryByRole("img")).toBeNull();
    rerender(<NativeComponent label="Pod CPU" payload={chart()} state={{ status: "error", error: "Prometheus unreachable" }} onRetry={() => {}}/>);
    expect(screen.getByRole("alert").textContent).toContain("Prometheus unreachable");
    expect(screen.queryByText("No data reported.")).toBeNull();
    rerender(<NativeComponent label="Pod CPU" payload={chart({ times: [], series: [{ name: "web", values: [] }] })}/>);
    expect(screen.getByText("No data reported.")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
    rerender(<NativeComponent label="Pod CPU" payload={chart({ series: [{ name: "web", values: [null, null, null, null] }] })}/>);
    expect(screen.getByText("No data reported.")).toBeTruthy();
  });
  it("follows the width of the region it is given, down to the narrow peek", () => {
    const width = vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(352);
    let resize: () => void = () => {};
    const disconnect = vi.fn();
    vi.stubGlobal("ResizeObserver", class { constructor(callback: () => void) { resize = callback; } observe() {} disconnect = disconnect; });
    const { unmount } = render(<NativeComponent label="Pod CPU" payload={chart()}/>);
    expect(screen.getByRole("img").getAttribute("width")).toBe("352");
    width.mockReturnValue(900);
    act(() => resize());
    expect(screen.getByRole("img").getAttribute("width")).toBe("900");
    unmount();
    expect(disconnect).toHaveBeenCalled();
  });
  it("fails closed on a malformed chart", () => {
    render(<NativeComponent label="Pod CPU" payload={chart({ unit: "furlongs" })}/>);
    expect(screen.getByRole("alert").textContent).toContain("Could not display Pod CPU");
    expect(screen.queryByRole("img")).toBeNull();
  });
});
