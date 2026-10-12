import { afterEach, describe, it, expect, vi } from "vitest";
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
import { VolumeHistogram } from "./VolumeHistogram";

// Real bars come from the reader's time zone, and a bar with no width needs a
// DST change in it. A test that must pass in every zone hands the component one.
const stub = vi.hoisted(() => ({ volume: undefined as Volume | undefined }));
vi.mock("@srelens/core", async (importOriginal) => {
  const core = await importOriginal<typeof import("@srelens/core")>();
  return {
    ...core,
    volumeBuckets: (...args: Parameters<typeof core.volumeBuckets>) => stub.volume ?? core.volumeBuckets(...args),
  };
});
afterEach(() => {
  stub.volume = undefined;
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
    const bar = (start: number, end: number, other: number) => ({ start, end, danger: 0, warning: 0, other });
    stub.volume = { step: hour, untimed: 0, buckets: [bar(0, hour, 1), bar(hour, hour, 0), bar(hour, 2 * hour, 1)] };
    const errors = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const { bars } = draw();
    expect(bars()).toHaveLength(3);
    expect(errors).not.toHaveBeenCalled();
    errors.mockRestore();
  });

  it("draws nothing when no line has a time", () => {
    const { container } = render(<VolumeHistogram lines={[ql("no time")]} range={undefined} onRangeChange={vi.fn()} />);
    expect(container.innerHTML).toBe("");
  });
});
