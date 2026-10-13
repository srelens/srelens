import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  logLineHealth,
  logLineLevel,
  logLineWords,
  parseLogFields,
  parseLogQuery,
  type LogLine,
  type QueryLine,
} from "@srelens/core";
import { toneWash } from "@srelens/ui-kit";
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

/** A QueryLine as the Logs screen builds one. */
function ql(text: string): QueryLine {
  const { message, fields } = parseLogFields(text);
  const health = logLineHealth(text);
  return { text, haystack: `${text} ${health}`.toLowerCase(), words: logLineWords(message), fields, level: logLineLevel(text) ?? "", health };
}

const FACET_LINES = [
  '{"level":"info","msg":"a","system":"camlog"}',
  '{"level":"info","msg":"b","system":"camlog"}',
  '{"level":"warn","msg":"c","system":"schwarz"}',
].map(ql);

function rail(props: Partial<Parameters<typeof StreamRail>[0]> = {}) {
  const onTogglePod = vi.fn();
  const onToggle = vi.fn();
  const { container } = render(
    <StreamRail
      pods={PODS}
      lines={BUFFER}
      onTogglePod={onTogglePod}
      query={{ lines: [], clauses: [], fields: [], onToggle }}
      {...props}
    />,
  );
  return { container, onTogglePod, onToggle };
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

  it("draws Sources and Top terms as flush section bands", () => {
    const { container } = rail();
    const sections = [...container.querySelectorAll("section")];
    expect(sections.length).toBe(2);
    expect(sections.every(section => section.hasAttribute("data-band"))).toBe(true);
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

    it("counts a line by the severity it arrives with, rather than reading its text again", () => {
      const { container } = rail({
        lines: [
          { ...line("a", "disk gone"), health: "danger" },
          { ...line("a", "disk still gone"), health: "danger" },
          { ...line("a", "error is only a word in this one"), health: "neutral" },
        ],
      });

      expect(container.querySelector(".badge")?.textContent).toBe("2 errors");
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

    it("tallies the message a line arrives with, and tones it by the severity it arrives with", () => {
      // A logfmt line opens with its own time, which ends a term before any word of the message.
      const logfmt = (n: number) => ({
        ...line("a", `time=2026-10-12T10:33:0${n}Z level=error msg="Write failed: timeout" system=x`),
        message: "Write failed: timeout",
        health: "danger" as const,
      });
      const { container } = rail({ lines: [1, 2, 3].map(logfmt) });

      expect(termRows(container)).toEqual([["Write failed timeout", "3"]]);
      expect(Array.from(container.querySelectorAll('[data-slot="term"]')).map(toneOf)).toEqual(["danger"]);
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

describe("the facets (#898)", () => {
  const facetRail = (clauses = "", lines: readonly QueryLine[] = FACET_LINES) => {
    const parsed = parseLogQuery(clauses);
    const onToggle = vi.fn();
    const view = render(
      <StreamRail
        pods={PODS}
        lines={BUFFER}
        onTogglePod={vi.fn()}
        query={{ lines, clauses: parsed.ok ? parsed.clauses : [], fields: ["system"], onToggle }}
      />,
    );
    return { ...view, onToggle };
  };

  /** Six systems, `s1` the busiest and `s6` the quietest, so `s6` ranks past the collapsed five. */
  const SIX_SYSTEMS = ["s1", "s1", "s1", "s1", "s1", "s1", "s2", "s2", "s2", "s2", "s2", "s3", "s3", "s3", "s3", "s4", "s4", "s4", "s5", "s5", "s6"].map(
    (system) => ql(`{"level":"info","msg":"m","system":"${system}"}`),
  );
  const facetLabels = () => Array.from(document.querySelectorAll('[data-slot="facet"] .status')).map((e) => e.textContent);

  it("lists the levels worst first, and each discovered field's values, with counts", () => {
    facetRail();
    expect(screen.getByRole("button", { name: "level = warn, 1 line" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "level = info, 2 lines" })).toBeTruthy();
    const levels = Array.from(document.querySelectorAll('[data-slot="level"] .status')).map((e) => e.textContent);
    expect(levels).toEqual(["warn", "info"]);
    expect(screen.getByRole("button", { name: "system = camlog, 2 lines" })).toBeTruthy();
  });

  it("filters a value in with a click and out with its exclude button", async () => {
    const { onToggle } = facetRail();
    await userEvent.click(screen.getByRole("button", { name: "system = camlog, 2 lines" }));
    expect(onToggle).toHaveBeenLastCalledWith({ field: "system", value: "camlog" }, "include");
    await userEvent.click(screen.getByRole("button", { name: "Exclude system = camlog" }));
    expect(onToggle).toHaveBeenLastCalledWith({ field: "system", value: "camlog" }, "exclude");
  });

  it("presses what the query already filters on, and keeps an excluded value reachable at zero", () => {
    facetRail("level:info -system:schwarz");
    expect(screen.getByRole("button", { name: "level = info, 2 lines" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByRole("button", { name: "Exclude system = schwarz" }).getAttribute("aria-pressed")).toBe("true");
  });

  it("filters on a Top term as a phrase", async () => {
    const { onToggle } = rail();
    await userEvent.click(screen.getByRole("button", { name: /^pool timeout, 3 lines$/ }));
    expect(onToggle).toHaveBeenLastCalledWith({ phrase: "pool timeout" }, "include");
  });

  it("draws a value the query names twice, and no line has, as one row", () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      facetRail("system:zzz -system:zzz system:ZZZ");
      expect(error.mock.calls.some((call) => String(call[0]).includes("same key"))).toBe(false);
      expect(screen.getAllByRole("button", { name: "system = zzz, 0 lines" })).toHaveLength(1);
    } finally {
      error.mockRestore();
    }
  });

  it("keeps a value the query names at zero lines in the collapsed list", () => {
    facetRail("system:zzz", SIX_SYSTEMS);
    expect(facetLabels()).toEqual(["zzz", "s1", "s2", "s3", "s4"]);
    expect(screen.getByRole("button", { name: "system = zzz, 0 lines" })).toBeTruthy();
  });

  it("keeps an included value that ranks past the collapsed five in the list", () => {
    facetRail("system:s6", SIX_SYSTEMS);
    expect(facetLabels()).toEqual(["s6", "s1", "s2", "s3", "s4"]);
    expect(screen.getByRole("button", { name: "system = s6, 1 line" }).getAttribute("aria-pressed")).toBe("true");
  });

  it("names the field on Show all, and shows the rest when it is pressed", async () => {
    facetRail("", SIX_SYSTEMS);
    expect(facetLabels()).toEqual(["s1", "s2", "s3", "s4", "s5"]);
    await userEvent.click(screen.getByRole("button", { name: "Show all 6 system values" }));
    expect(facetLabels()).toEqual(["s1", "s2", "s3", "s4", "s5", "s6"]);
    await userEvent.click(screen.getByRole("button", { name: "Show fewer system values" }));
    expect(facetLabels()).toEqual(["s1", "s2", "s3", "s4", "s5"]);
  });

  it("keeps the levels worst first when one is filtered on, since that list is never cut", () => {
    facetRail("level:info");
    const levels = Array.from(document.querySelectorAll('[data-slot="level"] .status')).map((e) => e.textContent);
    expect(levels).toEqual(["warn", "info"]);
  });

  it("tints the rows the query filters in, and only those", () => {
    facetRail("level:info");
    const row = (level: string) =>
      Array.from(document.querySelectorAll<HTMLElement>('[data-slot="level"]')).find(
        (r) => r.querySelector(".status")?.textContent === level,
      )!;
    expect(row("info").style.background).toBe(toneWash("accent"));
    expect(row("warn").getAttribute("style")).toBeNull();
  });
});
