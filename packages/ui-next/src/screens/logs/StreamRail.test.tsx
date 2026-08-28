import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { LogLine } from "@srelens/core";
import { StreamRail, STREAM_RAIL_WIDTH, type StreamPod } from "./StreamRail";

/**
 * The design's three pods (§15), with the tones its Sources rows carry — two
 * failing, one on the old revision that never does.
 */
const PODS: StreamPod[] = [
  { name: "x2mzp", revision: "rev 119", checked: true, tone: "danger" },
  { name: "q7v4t", revision: "rev 119", checked: true, tone: "danger" },
  { name: "mk3wl", revision: "rev 118", checked: true, tone: "success" },
];

/**
 * A line as the rail wants it: the pod that wrote it, and the MESSAGE — the
 * stream's RFC3339 prefix already off. A line still carrying its stamp opens
 * with a digit, which the tally reads as a value and stops on, so a stamped
 * buffer tallies to nothing at all.
 */
const line = (source: string, text: string): LogLine => ({ source, text });

/** Enough repetition for the tally's two-occurrence bar, at three frequencies. */
const BUFFER: LogLine[] = [
  line("x2mzp", "error pool timeout waited=30.1s in_use=5"),
  line("x2mzp", "error pool timeout waited=30.2s in_use=6"),
  line("q7v4t", "error pool timeout waited=30.3s in_use=7"),
  line("q7v4t", "warn pool saturated, queueing request depth=18"),
  line("q7v4t", "warn pool saturated, queueing request depth=19"),
  line("mk3wl", "info liveness deadline extended for the drain"),
  line("mk3wl", "info liveness deadline extended for the shutdown"),
];

function rail(props: Partial<Parameters<typeof StreamRail>[0]> = {}) {
  const onTogglePod = vi.fn();
  const { container } = render(
    <StreamRail pods={PODS} lines={BUFFER} onTogglePod={onTogglePod} {...props} />,
  );
  return { container, onTogglePod };
}

/** Every Sources row, as `[pod name, revision]`. */
const podRows = (container: HTMLElement) =>
  Array.from(container.querySelectorAll('[data-slot="pod"]')).map(
    (row) =>
      [
        row.querySelector(".status")?.textContent ?? "",
        row.querySelector(".path")?.textContent ?? "",
      ] as const,
  );

/** Every Top terms row, as `[term, count]`, in the order drawn. */
const termRows = (container: HTMLElement) =>
  Array.from(container.querySelectorAll('[data-slot="term"]')).map(
    (row) =>
      [
        row.querySelector(".status")?.textContent ?? "",
        row.querySelector(".path")?.textContent ?? "",
      ] as const,
  );

/** The tone the dot on a row was given, as core named it. */
const toneOf = (row: Element) => row.querySelector(".status")?.getAttribute("data-kind");

const headings = () => screen.queryAllByRole("heading").map((h) => h.textContent ?? "");

/** The box named after a pod — the accessible name is the pod's own row text. */
const box = (pod: string) => screen.getByRole("checkbox", { name: pod }) as HTMLInputElement;

describe("StreamRail", () => {
  it("pins the design's rail width so the screen has one place to read it from", () => {
    expect(STREAM_RAIL_WIDTH).toBe(272);
  });

  describe("Sources", () => {
    it("draws a row per pod, with its name and its revision", () => {
      const { container } = rail();

      expect(podRows(container)).toEqual([
        ["x2mzp", "rev 119"],
        ["q7v4t", "rev 119"],
        ["mk3wl", "rev 118"],
      ]);
    });

    it("names each checkbox after its pod, and reflects whether it is in view", () => {
      rail({
        pods: [
          { name: "x2mzp", revision: "rev 119", checked: true, tone: "danger" },
          { name: "mk3wl", revision: "rev 118", checked: false, tone: "success" },
        ],
      });

      expect(box("x2mzp").checked).toBe(true);
      expect(box("mk3wl").checked).toBe(false);
    });

    it("drives the pod filter — the design's boxes are defaultChecked and drive nothing", async () => {
      const user = userEvent.setup();
      const { onTogglePod } = rail();

      await user.click(screen.getByRole("checkbox", { name: "q7v4t" }));
      expect(onTogglePod).toHaveBeenCalledWith("q7v4t", false);
    });

    it("puts a pod back in view when its box is ticked again", async () => {
      const user = userEvent.setup();
      const { onTogglePod } = rail({
        pods: [{ name: "q7v4t", revision: "rev 119", checked: false, tone: "danger" }],
      });

      await user.click(screen.getByRole("checkbox", { name: "q7v4t" }));
      expect(onTogglePod).toHaveBeenCalledWith("q7v4t", true);
    });

    it("takes each pod's tone as given rather than deciding one here", () => {
      const { container } = rail();

      expect(
        Array.from(container.querySelectorAll('[data-slot="pod"]')).map(toneOf),
      ).toEqual(["danger", "danger", "success"]);
    });

    it("omits a revision it was given none of, rather than drawing the row blank", () => {
      const { container } = rail({ pods: [{ name: "x2mzp", checked: true, tone: "danger" }] });

      expect(podRows(container)).toEqual([["x2mzp", ""]]);
    });
  });

  describe("the error badge", () => {
    it("counts error, fatal and panic lines in view, and nothing milder", () => {
      const { container } = rail({
        lines: [
          line("a", "error pool timeout"),
          line("a", "fatal liveness deadline exceeded, terminating"),
          line("a", "panic: send on closed channel"),
          line("a", "warn pool saturated"),
          line("a", "info starting checkout-api"),
          line("a", "GET /healthz 200 1ms"),
        ],
      });

      expect(container.querySelector(".badge")?.textContent).toBe("3 errors");
    });

    it("counts only what is in view, so unchecking a pod moves the number", () => {
      const { container } = rail({
        lines: BUFFER.filter((l) => l.source !== "x2mzp"),
        pods: PODS.map((p) => (p.name === "x2mzp" ? { ...p, checked: false } : p)),
      });

      // Three danger lines in the buffer, two of them x2mzp's.
      expect(container.querySelector(".badge")?.textContent).toBe("1 error");
    });

    it("says nothing at all rather than badging a zero", () => {
      const { container } = rail({ lines: [line("mk3wl", "info all quiet"), line("mk3wl", "info still quiet")] });

      expect(container.querySelector(".badge")).toBeNull();
      expect(headings()).toContain("Sources");
    });
  });

  describe("Top terms", () => {
    it("ranks what the stream keeps saying, most frequent first", () => {
      const { container } = rail();

      expect(termRows(container)).toEqual([
        ["pool timeout", "3"],
        ["pool saturated", "2"],
        ["liveness deadline", "2"],
      ]);
    });

    it("tones a term by core's verdict on the lines behind it", () => {
      const { container } = rail();
      const rows = Array.from(container.querySelectorAll('[data-slot="term"]'));

      expect(rows.map(toneOf)).toEqual(["danger", "warning", "info"]);
    });

    it("tallies the lines in view, so the terms cannot disagree with the badge", () => {
      const { container } = rail({ lines: BUFFER.filter((l) => l.source === "q7v4t") });

      // q7v4t's three lines: one `pool timeout`, two `pool saturated`. The
      // singleton never recurred, so the tally drops it.
      expect(termRows(container)).toEqual([["pool saturated", "2"]]);
    });
  });

  describe("nothing to say", () => {
    it("renders neither section on an empty buffer rather than two empty boxes", () => {
      const { container } = rail({ lines: [] });

      expect(headings()).toEqual([]);
      expect(container.textContent).toBe("");
    });

    it("keeps Sources when the empty view is a box the reader ticked off", async () => {
      // Unchecking the last pod empties the view. Hiding the section here
      // would take the only control that can undo it away with the lines.
      const user = userEvent.setup();
      const { container, onTogglePod } = rail({
        lines: [],
        pods: [{ name: "x2mzp", revision: "rev 119", checked: false, tone: "danger" }],
      });

      expect(headings()).toEqual(["Sources"]);
      await user.click(screen.getByRole("checkbox", { name: "x2mzp" }));
      expect(onTogglePod).toHaveBeenCalledWith("x2mzp", true);
      expect(container.querySelectorAll('[data-slot="term"]')).toHaveLength(0);
    });

    it("drops Top terms when nothing has recurred, rather than heading an empty list", () => {
      const { container } = rail({ lines: [line("x2mzp", "info starting checkout-api build=4f2a1c")] });

      expect(headings()).toEqual(["Sources"]);
      expect(container.querySelectorAll('[data-slot="term"]')).toHaveLength(0);
    });

    it("draws no Sources section for a stream with no pods to name", () => {
      const { container } = rail({ pods: [] });

      expect(headings()).toEqual(["Top terms"]);
      expect(container.querySelectorAll('[data-slot="pod"]')).toHaveLength(0);
    });
  });

  it("puts no log text in a title attribute", () => {
    // The rule PairList and KV were stripped for: a `title` is a value nobody
    // asked to have shown, and a log line is exactly the kind of string that
    // carries a secret through one.
    const { container } = rail();

    expect(container.querySelectorAll("[title]")).toHaveLength(0);
  });
});
