import { afterEach, beforeEach, describe, it, expect, vi } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  logLineHealth,
  logLineLevel,
  logLineWords,
  parseLogFields,
  parseLogQuery,
  type QueryClause,
  type QueryLine,
  type Volume,
} from "@srelens/core";
import { statusTone, toneColor } from "@srelens/ui-kit";
import { isSectionOpen, loadSectionFolds, SECTION_FOLDS_KEY } from "../../lib/sectionFolds";
import { VolumeHistogram } from "./VolumeHistogram";
import { clockTime } from "./time";

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
// The fold is remembered between launches, so a test that shuts the chart must not leave it shut.
beforeEach(() => loadSectionFolds(emptyStorage()));

function emptyStorage() {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
  };
}

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

/** For a render that isn't about the legend. */
const QUIET = { clauses: [] as readonly QueryClause[], onToggleLevels: () => undefined };

const clausesOf = (query: string): readonly QueryClause[] => {
  const parsed = parseLogQuery(query);
  return parsed.ok ? parsed.clauses : [];
};

function draw(range?: { from: number; to: number }, clauses: readonly QueryClause[] = []) {
  const onRangeChange = vi.fn();
  const onToggleLevels = vi.fn();
  const view = render(
    <VolumeHistogram
      lines={LINES}
      range={range}
      onRangeChange={onRangeChange}
      clauses={clauses}
      onToggleLevels={onToggleLevels}
    />,
  );
  const bars = () => Array.from(view.container.querySelectorAll('[data-slot="bucket"]'));
  const caption = () => view.container.querySelector('[data-slot="volume-caption"]')?.textContent ?? "";
  return { ...view, onRangeChange, onToggleLevels, bars, caption };
}

/** One bucket holding `texts`, all at the same instant. */
const one = (...texts: string[]) =>
  render(<VolumeHistogram {...QUIET} lines={texts.map((t) => ql(t, T0))} range={undefined} onRangeChange={vi.fn()} />);
const segments = (container: HTMLElement) =>
  Array.from(container.querySelectorAll<SVGRectElement>('[data-slot="bucket"] rect[data-slot="segment"]'));
const num = (el: Element, attr: string) => Number(el.getAttribute(attr));
const texts = (container: HTMLElement, slot: string) =>
  Array.from(container.querySelectorAll(`[data-slot="${slot}"]`)).map((e) => e.textContent);

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
      <VolumeHistogram {...QUIET} lines={LINES} range={{ from: T0 + 1000, to: T0 + 2000 }} onRangeChange={onRangeChange} />,
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
      <VolumeHistogram {...QUIET} lines={lines} range={{ from: T0, to: T0 + 1000 }} onRangeChange={vi.fn()} />,
    );
    const caption = container.querySelector('[data-slot="volume-caption"]')?.textContent ?? "";
    expect(caption).toMatch(/^2 lines · .* · 1 without a time · 1 outside the window$/);
  });

  it("still picks a window when the stream coarsens its bars mid-drag", () => {
    // A live tail re-buckets under the pointer: the bar pressed may be gone by the release.
    const onRangeChange = vi.fn();
    const view = render(<VolumeHistogram {...QUIET} lines={LINES} range={undefined} onRangeChange={onRangeChange} />);
    const bars = () => view.container.querySelectorAll('[data-slot="bucket"]');
    fireEvent.pointerDown(bars()[3]);
    view.rerender(
      <VolumeHistogram {...QUIET} lines={[ql("info b", T0 + 1000)]} range={undefined} onRangeChange={onRangeChange} />,
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

    it("draws the focus wash behind the bars, the ring over them, and clips neither", async () => {
      const { container, bars } = draw();
      group().focus();
      await userEvent.keyboard("{ArrowRight}");
      const wash = container.querySelector('[data-slot="bucket-focus-wash"]') as Element;
      const ring = container.querySelector('[data-slot="bucket-focus"]') as Element;
      expect(wash.compareDocumentPosition(bars()[0]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      expect(ring.compareDocumentPosition(bars()[3]) & Node.DOCUMENT_POSITION_PRECEDING).toBeTruthy();
      // A ring on the first bar sits half outside the viewBox.
      expect(container.querySelector("svg")?.style.overflow).toBe("visible");
    });

    it("leaves a modified arrow to the browser", () => {
      const { container } = draw();
      group().focus();
      expect(fireEvent.keyDown(group(), { key: "ArrowRight", altKey: true })).toBe(true);
      expect(container.querySelector('[data-slot="bucket-focus"]')).toBeNull();
      expect(fireEvent.keyDown(group(), { key: "ArrowRight" })).toBe(false);
    });

    const live = (container: HTMLElement) => container.querySelector('[aria-live="polite"]')?.textContent;

    it("announces the bar the arrows land on, in the words of its tooltip", async () => {
      const { container } = draw();
      group().focus();
      await userEvent.keyboard("{ArrowRight}");
      expect(live(container)).toBe(`${clockTime(T0)}–${clockTime(T0 + 1000)} · 1 line · error 1`);
    });

    it("says nothing when the stream adds lines to the bar the arrows are on", async () => {
      const { container, rerender } = draw();
      group().focus();
      await userEvent.keyboard("{ArrowRight}");
      const said = live(container);
      expect(said).toContain("1 line");
      rerender(
        <VolumeHistogram
          {...QUIET}
          lines={[...LINES, ql("error again", T0 + 100), ql("warn too", T0 + 200)]}
          range={undefined}
          onRangeChange={vi.fn()}
        />,
      );
      // The bar now holds three lines, and the announcement still describes the bar the reader moved onto.
      expect(live(container)).toBe(said);
    });

    it("says nothing once focus has left", async () => {
      const { container } = draw();
      group().focus();
      await userEvent.keyboard("{ArrowRight}");
      fireEvent.blur(group());
      expect(live(container)).toBe("");
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

  describe("the y axis", () => {
    const hundreds = (n: number): Volume => ({
      step: 1000,
      untimed: 0,
      buckets: [{ start: T0, end: T0 + 1000, danger: 0, warning: 0, info: n, other: 0 }],
    });

    it("scales the bars to a round top rather than to the tallest bar", () => {
      const { container } = one("error a", "error b", "error c");
      // Three lines under a top of five.
      expect(num(segments(container)[0], "height")).toBeCloseTo((3 / 5) * 96);
    });

    it("labels the round top and its half", () => {
      const { container } = one("info a", "info b", "info c", "info d", "info e", "info f");
      expect(texts(container, "y-label")).toEqual(["10", "5"]);
    });

    it("groups the digits of a long label like every other count", () => {
      stub.volume = hundreds(1200);
      const { container } = one("info a");
      expect(texts(container, "y-label")).toEqual(["2 000", "1 000"]);
    });

    it("draws gridlines at zero, half and the top, at a constant width", () => {
      const { container } = one("info a");
      const lines = Array.from(container.querySelectorAll('[data-slot="gridline"]'));
      expect(lines.map((l) => num(l, "y1"))).toEqual([96, 48, 0]);
      for (const l of lines) expect(l.getAttribute("vector-effect")).toBe("non-scaling-stroke");
    });
  });

  describe("the time axis", () => {
    const minute = 60_000;
    const across = (from: number, bars: number): Volume => ({
      step: minute,
      untimed: 0,
      buckets: Array.from({ length: bars }, (_, i) => ({
        start: from + i * minute,
        end: from + (i + 1) * minute,
        danger: 0,
        warning: 0,
        info: 1,
        other: 0,
      })),
    });
    const ticks = (container: HTMLElement) =>
      Array.from(container.querySelectorAll<HTMLElement>('[data-slot="x-tick"]')).map((t) => [
        t.textContent,
        t.style.left,
      ]);

    it("puts round local times under the bars, where they fall along the span", () => {
      stub.volume = across(new Date(2026, 9, 12, 10).getTime(), 10);
      const { container } = one("info a");
      expect(ticks(container)).toEqual([
        ["10:00", "0%"],
        ["10:02", "20%"],
        ["10:04", "40%"],
        ["10:06", "60%"],
        ["10:08", "80%"],
        ["10:10", "100%"],
      ]);
    });

    it("dates the tick that starts a new day", () => {
      stub.volume = across(new Date(2026, 9, 12, 23).getTime(), 120);
      const { container } = one("info a");
      expect(ticks(container).map(([label]) => label)).toEqual(["23:00", "23:30", "10-13 00:00", "00:30", "01:00"]);
    });
  });

  describe("the legend", () => {
    const entries = (container: HTMLElement) => texts(container, "legend-item");
    const words = [ql("error a", T0), ql("fatal b", T0 + 1000), ql("FATAL c", T0 + 2000), ql("info d", T0 + 3000)];
    const withWords = (clauses: readonly QueryClause[]) => {
      const onToggleLevels = vi.fn();
      const view = render(
        <VolumeHistogram
          lines={words}
          range={undefined}
          onRangeChange={vi.fn()}
          clauses={clauses}
          onToggleLevels={onToggleLevels}
        />,
      );
      return { ...view, onToggleLevels };
    };

    it("counts each group's lines, and offers only the groups that are there", () => {
      const { container } = draw();
      // The line with no time is still a line of the query, so it is counted.
      expect(entries(container)).toEqual(["error 1", "info 2", "other 1"]);
    });

    it("colours each entry's dot as its bars are coloured", () => {
      const { container } = draw();
      const dots = Array.from(container.querySelectorAll('[data-slot="legend-item"] [data-slot="dot"] circle'));
      expect(dots.map((d) => d.getAttribute("fill"))).toEqual([
        toneColor(statusTone("danger")),
        toneColor(statusTone("info")),
        toneColor(statusTone("neutral")),
      ]);
    });

    it("makes error, warn and info buttons, and leaves other as plain text", () => {
      draw();
      expect(screen.getByRole("button", { name: "error 1" })).toBeTruthy();
      expect(screen.getByRole("button", { name: "info 2" })).toBeTruthy();
      expect(screen.queryByRole("button", { name: /^other/ })).toBeNull();
      expect(screen.getByText("other")).toBeTruthy();
    });

    it("asks for every level word of the group that the lines carry", async () => {
      const { onToggleLevels } = withWords([]);
      await userEvent.click(screen.getByRole("button", { name: "error 3" }));
      expect(onToggleLevels).toHaveBeenCalledWith(["error", "fatal"], true);
    });

    it("is pressed when all of the group's words are in the query, and then asks to take them out", async () => {
      const { onToggleLevels } = withWords(clausesOf("level:error level:fatal"));
      const button = screen.getByRole("button", { name: "error 3" });
      expect(button.getAttribute("aria-pressed")).toBe("true");
      await userEvent.click(button);
      expect(onToggleLevels).toHaveBeenCalledWith(["error", "fatal"], false);
    });

    it("is not pressed while a word of the group is missing, and then asks for all of them", async () => {
      const { onToggleLevels } = withWords(clausesOf("level:error"));
      const button = screen.getByRole("button", { name: "error 3" });
      expect(button.getAttribute("aria-pressed")).toBe("false");
      await userEvent.click(button);
      expect(onToggleLevels).toHaveBeenCalledWith(["error", "fatal"], true);
    });

    it("reads an excluded word as not included", () => {
      withWords(clausesOf("-level:error level:fatal"));
      expect(screen.getByRole("button", { name: "error 3" }).getAttribute("aria-pressed")).toBe("false");
    });
  });

  describe("the tooltip", () => {
    const tooltip = () => screen.queryByRole("tooltip");

    it("is not there until a bar is pointed at", () => {
      draw();
      expect(tooltip()).toBeNull();
    });

    it("names the bar's time, its lines and each group's count, as soon as the pointer enters it", () => {
      const { bars } = draw();
      fireEvent.pointerEnter(bars()[0]);
      const tip = tooltip() as HTMLElement;
      expect(tip.querySelector('[data-slot="tooltip-range"]')?.textContent).toBe(
        `${clockTime(T0)}–${clockTime(T0 + 1000)}`,
      );
      expect(tip.querySelector('[data-slot="tooltip-total"]')?.textContent).toBe("1 line");
      const groups = Array.from(tip.querySelectorAll('[data-slot="tooltip-group"]'));
      expect(groups.map((g) => g.textContent)).toEqual(["error 1"]);
      expect(groups[0].querySelector("circle")?.getAttribute("fill")).toBe(toneColor(statusTone("danger")));
    });

    it("counts a bar with several groups in the legend's order, and pluralises", () => {
      const lines = [ql("info a", T0), ql("warn b", T0), ql("debug c", T0), ql("error d", T0), ql("error e", T0)];
      const { container } = render(<VolumeHistogram {...QUIET} lines={lines} range={undefined} onRangeChange={vi.fn()} />);
      fireEvent.pointerEnter(container.querySelector('[data-slot="bucket"]') as Element);
      expect(texts(container, "tooltip-total")).toEqual(["5 lines"]);
      expect(texts(container, "tooltip-group")).toEqual(["error 2", "warn 1", "info 1", "other 1"]);
    });

    it("follows the pointer from bar to bar, and goes when it leaves the chart", () => {
      const { bars, container } = draw();
      fireEvent.pointerEnter(bars()[0]);
      const first = (tooltip() as HTMLElement).style.left;
      fireEvent.pointerEnter(bars()[3]);
      expect(Number.parseFloat((tooltip() as HTMLElement).style.left)).toBeGreaterThan(Number.parseFloat(first));
      fireEvent.pointerLeave(container.querySelector("svg") as SVGElement);
      expect(tooltip()).toBeNull();
    });

    it("covers the gap between bars with the bar before it, so crossing one doesn't flicker", () => {
      const { container } = draw();
      const hit = container.querySelector('[data-slot="bucket"] rect[fill="transparent"]');
      // BAR is 10 and GAP 2 in the viewBox.
      expect(hit?.getAttribute("width")).toBe("12");
    });

    it("shows for the bar the keyboard is on, and goes with the focus", async () => {
      draw();
      screen.getByRole("group", { name: /log volume over time/i }).focus();
      await userEvent.keyboard("{ArrowRight}");
      expect(tooltip()?.querySelector('[data-slot="tooltip-total"]')?.textContent).toBe("1 line");
      fireEvent.blur(screen.getByRole("group", { name: /log volume over time/i }));
      expect(tooltip()).toBeNull();
    });

    it("hands the tooltip to the arrow keys, though the pointer is still resting on a bar", async () => {
      const { bars } = draw();
      fireEvent.pointerEnter(bars()[3]);
      expect(tooltip()?.querySelector('[data-slot="tooltip-range"]')?.textContent).toBe(
        `${clockTime(T0 + 3000)}–${clockTime(T0 + 4000)}`,
      );
      screen.getByRole("group", { name: /log volume over time/i }).focus();
      await userEvent.keyboard("{ArrowRight}");
      expect(tooltip()?.querySelector('[data-slot="tooltip-range"]')?.textContent).toBe(
        `${clockTime(T0)}–${clockTime(T0 + 1000)}`,
      );
    });

    it("replaces the slow native tooltip rather than doubling it", () => {
      const { container } = draw();
      expect(container.querySelector("title")).toBeNull();
      expect(container.querySelector("[title]")).toBeNull();
    });
  });

  describe("the brush", () => {
    const brush = (container: HTMLElement) => container.querySelector('[data-slot="brush"]');

    it("is only there while a drag is in progress", () => {
      const { bars, container } = draw();
      expect(brush(container)).toBeNull();
      fireEvent.pointerDown(bars()[1]);
      expect(brush(container)).not.toBeNull();
      fireEvent.pointerUp(bars()[1]);
      expect(brush(container)).toBeNull();
    });

    it("spans the dragged bars, whichever way the drag went, in a translucent accent", () => {
      const { bars, container } = draw();
      fireEvent.pointerDown(bars()[3]);
      fireEvent.pointerEnter(bars()[1]);
      const rect = brush(container) as Element;
      // From the left edge of bar 1 to the right edge of bar 3, at 12 units a slot.
      expect(rect.getAttribute("x")).toBe("12");
      expect(rect.getAttribute("width")).toBe("34");
      expect(rect.getAttribute("fill")).toBe(toneColor("accent"));
      expect(Number(rect.getAttribute("fill-opacity"))).toBeGreaterThan(0);
      expect(Number(rect.getAttribute("fill-opacity"))).toBeLessThan(0.5);
    });

    it("goes when the system cancels the drag", () => {
      const { bars, container } = draw();
      fireEvent.pointerDown(bars()[1]);
      fireEvent.pointerCancel(bars()[1]);
      expect(brush(container)).toBeNull();
    });
  });

  describe("the fold", () => {
    const toggle = (name: string) => screen.getByRole("button", { name });

    it("is open to start with, and offers to hide the chart", () => {
      draw();
      expect(toggle("Hide the volume chart").getAttribute("aria-expanded")).toBe("true");
      expect(screen.getByRole("group", { name: /log volume over time/i })).toBeTruthy();
    });

    it("hides everything but the caption row, remembers that, and offers to show it again", async () => {
      const { container, caption } = draw();
      await userEvent.click(toggle("Hide the volume chart"));
      expect(screen.queryByRole("group", { name: /log volume over time/i })).toBeNull();
      expect(container.querySelector("svg")).toBeNull();
      expect(screen.queryByRole("button", { name: "error 1" })).toBeNull();
      expect(container.querySelector('[data-slot="y-label"]')).toBeNull();
      // The caption row stays, with the toggle at its end.
      expect(caption()).toMatch(/^3 lines · /);
      expect(toggle("Show the volume chart").getAttribute("aria-expanded")).toBe("false");
      expect(isSectionOpen("logs", "volume", true)).toBe(false);
      await userEvent.click(toggle("Show the volume chart"));
      expect(container.querySelector("svg")).not.toBeNull();
      expect(isSectionOpen("logs", "volume", true)).toBe(true);
    });

    it("starts shut for a reader who shut it last time", () => {
      const storage = emptyStorage();
      storage.setItem(SECTION_FOLDS_KEY, JSON.stringify({ logs: ["!volume"] }));
      loadSectionFolds(storage);
      const { container } = draw();
      expect(container.querySelector("svg")).toBeNull();
      expect(toggle("Show the volume chart")).toBeTruthy();
    });

    it("lets go of the bar the arrows were on, so opening it again shows no stale ring", async () => {
      const { container } = draw();
      screen.getByRole("group", { name: /log volume over time/i }).focus();
      await userEvent.keyboard("{ArrowRight}");
      await userEvent.click(toggle("Hide the volume chart"));
      await userEvent.click(toggle("Show the volume chart"));
      expect(container.querySelector('[data-slot="bucket-focus"]')).toBeNull();
      expect(container.querySelector('[aria-live="polite"]')?.textContent).toBe("");
    });

    it("draws nothing at all, toggle included, when no line has a time", () => {
      const { container } = render(<VolumeHistogram {...QUIET} lines={[ql("no time")]} range={undefined} onRangeChange={vi.fn()} />);
      expect(container.innerHTML).toBe("");
    });
  });

  it("keeps a lone error visible beside a spike in the same bar", () => {
    const spike = [...Array.from({ length: 500 }, () => ql("info x", T0)), ql("error y", T0)];
    const { container } = render(<VolumeHistogram {...QUIET} lines={spike} range={undefined} onRangeChange={vi.fn()} />);
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
    /** What the tooltip says of the first bar. */
    const tip = (container: HTMLElement) => {
      fireEvent.pointerEnter(container.querySelector('[data-slot="bucket"]') as Element);
      const text = (slot: string) => container.querySelector(`[data-slot="${slot}"]`)?.textContent;
      return [text("tooltip-range"), text("tooltip-total")];
    };

    it("names a bar of a day or more by its date as well as its time", () => {
      const bar = (start: number, end: number) => ({ start, end, danger: 0, warning: 0, info: 0, other: 1 });
      stub.volume = {
        step: 86_400_000,
        untimed: 0,
        buckets: [bar(day(10), day(11)), bar(day(11), day(12)), bar(day(12), day(13))],
      };
      const { container } = render(<VolumeHistogram {...QUIET} lines={LINES} range={undefined} onRangeChange={vi.fn()} />);
      expect(caption(container)).toContain("2026-10-10 00:00–2026-10-13 00:00");
      expect(tip(container)).toEqual(["2026-10-10 00:00–2026-10-11 00:00", "1 line"]);
    });

    it("dates the bars once the span crosses a day, whatever the bar width", () => {
      const lines = [ql("info a", day(10) + 9 * 3_600_000), ql("info b", day(13) + 9 * 3_600_000)];
      const { container } = render(<VolumeHistogram {...QUIET} lines={lines} range={undefined} onRangeChange={vi.fn()} />);
      expect(caption(container)).toMatch(/ · 2026-10-10 \d\d:\d\d–2026-10-1\d \d\d:\d\d · /);
      const [range, total] = tip(container);
      expect(range).toMatch(/^2026-10-10 [0-9]{2}:[0-9]{2}–2026-10-10 [0-9]{2}:[0-9]{2}$/);
      expect(total).toBe("1 line");
    });

    it("keeps the seconds of bars narrower than a minute when it dates them", () => {
      const lines = [
        ql("info a", new Date(2026, 9, 12, 23, 59, 58).getTime()),
        ql("info b", new Date(2026, 9, 13, 0, 0, 1).getTime()),
      ];
      const { container } = render(<VolumeHistogram {...QUIET} lines={lines} range={undefined} onRangeChange={vi.fn()} />);
      expect(caption(container)).toContain("2026-10-12 23:59:58–2026-10-13 00:00:02 · 1s bars");
    });

    it("leaves the date off a span within one day", () => {
      const { container } = render(<VolumeHistogram {...QUIET} lines={LINES} range={undefined} onRangeChange={vi.fn()} />);
      expect(caption(container)).not.toMatch(/\d{4}-\d\d-\d\d/);
    });
  });

  it("draws nothing when no line has a time", () => {
    const { container } = render(<VolumeHistogram {...QUIET} lines={[ql("no time")]} range={undefined} onRangeChange={vi.fn()} />);
    expect(container.innerHTML).toBe("");
  });
});
