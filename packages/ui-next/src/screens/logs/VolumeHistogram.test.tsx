import { afterEach, describe, it, expect, vi } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  logLineHealth,
  logLineLevel,
  logLineWords,
  parseLogFields,
  type QueryLine,
  type Volume,
} from "@srelens/core";
import { statusTone, toneColor } from "@srelens/ui-kit";
import { VolumeHistogram } from "./VolumeHistogram";

// Real bars come from the reader's time zone, and a bar with no width needs a
// DST change in it. A test that must pass in every zone hands the component one.
// `inRangeCalls` counts the passes over the lines that ask whether one is in the window.
const stub = vi.hoisted(() => ({ volume: undefined as Volume | undefined, inRangeCalls: 0 }));
vi.mock("@srelens/core", async (importOriginal) => {
  const core = await importOriginal<typeof import("@srelens/core")>();
  return {
    ...core,
    volumeBuckets: (...args: Parameters<typeof core.volumeBuckets>) => stub.volume ?? core.volumeBuckets(...args),
    inRange: (...args: Parameters<typeof core.inRange>) => {
      stub.inRangeCalls += 1;
      return core.inRange(...args);
    },
  };
});
afterEach(() => {
  stub.volume = undefined;
  stub.inRangeCalls = 0;
});

const T0 = Date.parse("2026-10-12T10:00:00Z");

function ql(text: string, at?: number): QueryLine {
  const { message, fields } = parseLogFields(text);
  const health = logLineHealth(text);
  return {
    text,
    haystack: `${text} ${health}`.toLowerCase(),
    words: logLineWords(message),
    fields,
    level: logLineLevel(text) ?? "",
    health,
    ...(at === undefined ? {} : { at }),
  };
}

/** Bars at :00, :01 and :03, an empty one at :02, and one line with no time. */
const LINES = [ql("error a", T0), ql("info b", T0 + 1000), ql("info c", T0 + 3000), ql("no time")];

function draw(range?: { from: number; to: number }) {
  const onRangeChange = vi.fn();
  const view = render(<VolumeHistogram lines={LINES} range={range} onRangeChange={onRangeChange} />);
  const bars = () => Array.from(view.container.querySelectorAll('[data-slot="bucket"]'));
  const caption = () => view.container.querySelector('[data-slot="volume-caption"]')?.textContent ?? "";
  return { ...view, onRangeChange, bars, caption };
}

describe("VolumeHistogram", () => {
  it("draws one bar per bucket, gaps included, and says what it counted", () => {
    const { bars, caption } = draw();
    expect(bars()).toHaveLength(4);
    expect(caption()).toMatch(/^3 lines · \d\d:\d\d:00–\d\d:\d\d:04 · 1s bars · 1 without a time$/);
  });

  it("narrows to a clicked bar", () => {
    const { bars, onRangeChange } = draw();
    fireEvent.pointerDown(bars()[1]);
    fireEvent.pointerUp(bars()[1]);
    expect(onRangeChange).toHaveBeenCalledWith({ from: T0 + 1000, to: T0 + 2000 });
  });

  it("narrows to the span a drag covers, whichever way it went", () => {
    const { bars, onRangeChange } = draw();
    fireEvent.pointerDown(bars()[3]);
    fireEvent.pointerEnter(bars()[1]);
    fireEvent.pointerUp(bars()[1]);
    expect(onRangeChange).toHaveBeenCalledWith({ from: T0 + 1000, to: T0 + 4000 });
  });

  it("works from the keyboard: arrows pick a bar, Enter narrows to it, Escape clears", async () => {
    const { onRangeChange, rerender } = draw();
    screen.getByRole("group", { name: /log volume over time/i }).focus();
    await userEvent.keyboard("{ArrowRight}{ArrowRight}{Enter}");
    expect(onRangeChange).toHaveBeenLastCalledWith({ from: T0 + 1000, to: T0 + 2000 });
    rerender(
      <VolumeHistogram lines={LINES} range={{ from: T0 + 1000, to: T0 + 2000 }} onRangeChange={onRangeChange} />,
    );
    await userEvent.keyboard("{Escape}");
    expect(onRangeChange).toHaveBeenLastCalledWith(undefined);
  });

  it("dims the bars outside the window and counts what it leaves out", () => {
    const { bars, caption } = draw({ from: T0, to: T0 + 1000 });
    expect(bars().map((b) => b.getAttribute("opacity"))).toEqual(["1", "0.35", "0.35", "0.35"]);
    expect(caption()).toContain("2 outside the window");
  });

  it("counts a line whose time can't be placed as untimed, not as outside the window", () => {
    // `inRange` is false for it too, but it has no time to be outside of.
    const lines = [ql("a", T0), ql("b", T0 + 1000), ql("bad stamp", NaN)];
    const { container } = render(
      <VolumeHistogram lines={lines} range={{ from: T0, to: T0 + 1000 }} onRangeChange={vi.fn()} />,
    );
    const caption = container.querySelector('[data-slot="volume-caption"]')?.textContent ?? "";
    expect(caption).toMatch(/^2 lines · .* · 1 without a time · 1 outside the window$/);
  });

  it("still picks a window when the stream coarsens its bars mid-drag", () => {
    // A live tail re-buckets under the pointer: the bar pressed may be gone by the release.
    const onRangeChange = vi.fn();
    const view = render(<VolumeHistogram lines={LINES} range={undefined} onRangeChange={onRangeChange} />);
    const bars = () => view.container.querySelectorAll('[data-slot="bucket"]');
    fireEvent.pointerDown(bars()[3]);
    view.rerender(
      <VolumeHistogram lines={[ql("info b", T0 + 1000)]} range={undefined} onRangeChange={onRangeChange} />,
    );
    expect(bars()).toHaveLength(1);
    fireEvent.pointerUp(bars()[0]);
    expect(onRangeChange).toHaveBeenCalledWith({ from: T0 + 1000, to: T0 + 2000 });
  });

  it("draws a slot for a bar with no width, such as a skipped hour, without repeating a key", () => {
    // Spring forward: the skipped hour's bar starts where the next one does.
    const hour = 3_600_000;
    const bar = (start: number, end: number, other: number) => ({ start, end, danger: 0, warning: 0, info: 0, other });
    stub.volume = { step: hour, untimed: 0, buckets: [bar(0, hour, 1), bar(hour, hour, 0), bar(hour, 2 * hour, 1)] };
    const errors = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const { bars } = draw();
    expect(bars()).toHaveLength(3);
    expect(errors).not.toHaveBeenCalled();
    errors.mockRestore();
  });

  describe("keyboard focus", () => {
    const group = () => screen.getByRole("group", { name: /log volume over time/i });

    it("shows where focus is on the group, rather than switching the outline off", () => {
      draw();
      expect(group().classList.contains("outline-none")).toBe(false);
      expect(group().classList.contains("volume-bars")).toBe(true);
    });

    it("has a focus-visible outline for the group in the stylesheet", () => {
      // jsdom draws no CSS; what can be held is that the rule the class names exists, in the accent.
      const css = readFileSync(join(__dirname, "../../styles/next.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
      const rule = css.match(/\.volume-bars:focus-visible\s*\{([^}]*)\}/)?.[1];
      expect(rule).toContain("outline: 2px solid var(--accent)");
    });

    it("rings the focused bar outside the dimmed bars, at a constant width", async () => {
      const { container, bars } = draw({ from: T0, to: T0 + 1000 });
      group().focus();
      await userEvent.keyboard("{ArrowRight}{ArrowRight}");
      expect(bars()[1].getAttribute("opacity")).toBe("0.35");
      const ring = container.querySelector('[data-slot="bucket-focus"]');
      expect(ring).toBeTruthy();
      expect(ring?.getAttribute("x")).toBe("12");
      expect(ring?.closest('[opacity="0.35"]')).toBeNull();
      expect(ring?.getAttribute("vector-effect")).toBe("non-scaling-stroke");
    });

    it("leaves a modified arrow to the browser", () => {
      const { container } = draw();
      group().focus();
      expect(fireEvent.keyDown(group(), { key: "ArrowRight", altKey: true })).toBe(true);
      expect(container.querySelector('[data-slot="bucket-focus"]')).toBeNull();
      expect(fireEvent.keyDown(group(), { key: "ArrowRight" })).toBe(false);
    });

    it("announces the bar the arrows land on, in the words of its tooltip", async () => {
      const { container } = draw();
      group().focus();
      await userEvent.keyboard("{ArrowRight}");
      const live = container.querySelector('[aria-live="polite"]');
      const title = container.querySelector('[data-slot="bucket"] title')?.textContent;
      expect(title).toMatch(/ · 1 line · 1 error$/);
      expect(live?.textContent).toBe(title);
    });

    it("doesn't rescan the lines when only the focus moves", async () => {
      draw({ from: T0, to: T0 + 1000 });
      const scans = stub.inRangeCalls;
      expect(scans).toBeGreaterThan(0);
      group().focus();
      await userEvent.keyboard("{ArrowRight}{ArrowRight}");
      expect(stub.inRangeCalls).toBe(scans);
    });
  });

  describe("dragging", () => {
    it("ignores a button that isn't the primary one", () => {
      const { bars, onRangeChange } = draw();
      fireEvent.pointerDown(bars()[1], { button: 2 });
      fireEvent.pointerUp(bars()[1], { button: 2 });
      expect(onRangeChange).not.toHaveBeenCalled();
    });

    it("drops a drag the system cancels, so leaving the chart commits nothing", () => {
      const { bars, container, onRangeChange } = draw();
      fireEvent.pointerDown(bars()[1]);
      fireEvent.pointerCancel(bars()[1]);
      fireEvent.pointerLeave(container.querySelector("svg") as SVGElement);
      expect(onRangeChange).not.toHaveBeenCalled();
    });

    it("drops a drag when the window loses focus", () => {
      const { bars, container, onRangeChange } = draw();
      fireEvent.pointerDown(bars()[1]);
      fireEvent.blur(window);
      fireEvent.pointerLeave(container.querySelector("svg") as SVGElement);
      expect(onRangeChange).not.toHaveBeenCalled();
    });
  });

  describe("the bars", () => {
    const one = (...texts: string[]) =>
      render(<VolumeHistogram lines={texts.map((t) => ql(t, T0))} range={undefined} onRangeChange={vi.fn()} />);
    const segments = (container: HTMLElement) =>
      Array.from(container.querySelectorAll<SVGRectElement>('[data-slot="bucket"] rect[data-slot="segment"]'));
    const num = (el: Element, attr: string) => Number(el.getAttribute(attr));

    it("is 96 pixels tall", () => {
      const { container } = one("info a");
      expect(container.querySelector("svg")?.style.height).toBe("96px");
      expect(container.querySelector("svg")?.getAttribute("viewBox")).toMatch(/ 96$/);
    });

    it("stacks a bar up from the baseline: error, warn, info, then the rest", () => {
      const { container } = one("debug d", "info c", "warn b", "error a");
      const bottomUp = segments(container).sort((a, b) => num(b, "y") - num(a, "y"));
      expect(bottomUp.map((r) => r.getAttribute("data-group"))).toEqual(["danger", "warning", "info", "other"]);
      // The errors sit on the baseline, and each segment starts where the one below ends.
      expect(num(bottomUp[0], "y") + num(bottomUp[0], "height")).toBeCloseTo(96);
      for (let i = 1; i < bottomUp.length; i += 1) {
        expect(num(bottomUp[i], "y") + num(bottomUp[i], "height")).toBeCloseTo(num(bottomUp[i - 1], "y"));
      }
    });

    it("colours error, warn and info from core's health tones, and mutes the rest", () => {
      const { container } = one("debug d", "info c", "warn b", "error a");
      const fill = (group: string) => segments(container).find((r) => r.getAttribute("data-group") === group);
      expect(fill("danger")?.getAttribute("fill")).toBe(toneColor(statusTone("danger")));
      expect(fill("warning")?.getAttribute("fill")).toBe(toneColor(statusTone("warning")));
      expect(fill("info")?.getAttribute("fill")).toBe(toneColor(statusTone("info")));
      expect(fill("other")?.getAttribute("fill")).toBe(toneColor(statusTone("neutral")));
      // Translucent, so a stream of unlevelled lines doesn't drown the ones that have a level.
      expect(num(fill("other") as Element, "fill-opacity")).toBeGreaterThan(0);
      expect(num(fill("other") as Element, "fill-opacity")).toBeLessThan(1);
      expect(fill("info")?.getAttribute("fill-opacity")).toBeNull();
    });
  });

  it("keeps a lone error visible beside a spike in the same bar", () => {
    const spike = [...Array.from({ length: 500 }, () => ql("info x", T0)), ql("error y", T0)];
    const { container } = render(<VolumeHistogram lines={spike} range={undefined} onRangeChange={vi.fn()} />);
    const rects = Array.from(container.querySelectorAll('[data-slot="bucket"] rect[fill^="var(--"]'));
    const danger = rects.filter((r) => r.getAttribute("fill") === toneColor(statusTone("danger")));
    expect(danger).toHaveLength(1);
    expect(Number(danger[0].getAttribute("height"))).toBeGreaterThanOrEqual(1);
    // The stack still fits the chart: nothing is pushed past the top.
    for (const r of rects) expect(Number(r.getAttribute("y"))).toBeGreaterThanOrEqual(-1e-9);
  });

  describe("dates", () => {
    const day = (n: number) => new Date(2026, 9, n).getTime();
    const caption = (container: HTMLElement) =>
      container.querySelector('[data-slot="volume-caption"]')?.textContent ?? "";
    const titles = (container: HTMLElement) =>
      Array.from(container.querySelectorAll('[data-slot="bucket"] title')).map((t) => t.textContent);

    it("names a bar of a day or more by its date as well as its time", () => {
      const bar = (start: number, end: number) => ({ start, end, danger: 0, warning: 0, info: 0, other: 1 });
      stub.volume = {
        step: 86_400_000,
        untimed: 0,
        buckets: [bar(day(10), day(11)), bar(day(11), day(12)), bar(day(12), day(13))],
      };
      const { container } = render(<VolumeHistogram lines={LINES} range={undefined} onRangeChange={vi.fn()} />);
      expect(caption(container)).toContain("2026-10-10 00:00–2026-10-13 00:00");
      expect(titles(container)[0]).toBe("2026-10-10 00:00–2026-10-11 00:00 · 1 line");
    });

    it("dates the bars once the span crosses a day, whatever the bar width", () => {
      const lines = [ql("info a", day(10) + 9 * 3_600_000), ql("info b", day(13) + 9 * 3_600_000)];
      const { container } = render(<VolumeHistogram lines={lines} range={undefined} onRangeChange={vi.fn()} />);
      expect(caption(container)).toMatch(/ · 2026-10-10 \d\d:\d\d–2026-10-1\d \d\d:\d\d · /);
      expect(titles(container)[0]).toMatch(/^2026-10-10 \d\d:\d\d–2026-10-10 \d\d:\d\d · 1 line$/);
    });

    it("keeps the seconds of bars narrower than a minute when it dates them", () => {
      const lines = [
        ql("info a", new Date(2026, 9, 12, 23, 59, 58).getTime()),
        ql("info b", new Date(2026, 9, 13, 0, 0, 1).getTime()),
      ];
      const { container } = render(<VolumeHistogram lines={lines} range={undefined} onRangeChange={vi.fn()} />);
      expect(caption(container)).toContain("2026-10-12 23:59:58–2026-10-13 00:00:02 · 1s bars");
    });

    it("leaves the date off a span within one day", () => {
      const { container } = render(<VolumeHistogram lines={LINES} range={undefined} onRangeChange={vi.fn()} />);
      expect(caption(container)).not.toMatch(/\d{4}-\d\d-\d\d/);
    });
  });

  it("draws nothing when no line has a time", () => {
    const { container } = render(<VolumeHistogram lines={[ql("no time")]} range={undefined} onRangeChange={vi.fn()} />);
    expect(container.innerHTML).toBe("");
  });
});
