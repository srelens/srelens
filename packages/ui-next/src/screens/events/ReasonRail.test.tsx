import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { eventVerdict } from "@srelens/core";
import type { EventRow } from "../../lib/kinds/events";
import { ReasonRail } from "./ReasonRail";

/**
 * One event. `count` is how many times THAT event fired — deliberately a
 * parameter here, because the whole point of the rail is that it does not add
 * these up.
 */
function event(reason: string, type: string, count: number, key = `${reason}.${count}`): EventRow {
  return {
    name: `shop/${key}`,
    namespace: "shop",
    type,
    reason,
    object: "Pod/web-1",
    message: `${reason} happened`,
    count,
    age: "12s",
  };
}

/**
 * Six events, three reasons, chosen so the two possible readings of "count"
 * disagree about BOTH the numbers and the order:
 *
 * | reason    | events | summed `count` |
 * |-----------|--------|----------------|
 * | Unhealthy | 3      | 3              |
 * | Scheduled | 2      | 2              |
 * | BackOff   | 1      | 400            |
 *
 * By events the order is Unhealthy, Scheduled, BackOff; by summed `count` it
 * is BackOff, Unhealthy, Scheduled. A fixture of `count: 1` rows cannot tell
 * the two apart, which is exactly how the wrong one ships.
 */
const ROWS: EventRow[] = [
  event("BackOff", "Warning", 400),
  event("Unhealthy", "Warning", 1, "u1"),
  event("Scheduled", "Normal", 1, "s1"),
  event("Unhealthy", "Warning", 1, "u2"),
  event("Scheduled", "Normal", 1, "s2"),
  event("Unhealthy", "Warning", 1, "u3"),
];

function rail(rows: EventRow[] = ROWS, onPick = vi.fn()) {
  const { container } = render(<ReasonRail rows={rows} onPick={onPick} />);
  return { container, onPick };
}

/**
 * Every rail row, as the two things it shows — the labelled dot and the count
 * beside it — in the order it was drawn.
 */
const shown = () =>
  screen
    .getAllByRole("button")
    .map(
      (el) =>
        [el.querySelector(".status")?.textContent ?? "", el.querySelector(".path")?.textContent ?? ""] as const,
    );

/** The tone the dot on the row called `name` was drawn in, as `StatusPill`
 *  records it. Found through the row's accessible name, which is the same
 *  handle a reader has. */
const toneOf = (name: string) =>
  screen.getByRole("button", { name }).querySelector(".status")?.getAttribute("data-kind");

describe("ReasonRail", () => {
  it("counts the events carrying a reason, not the times one of them fired", () => {
    const { container } = rail();

    // One event that fired 400 times is ONE thing going wrong. Summing `count`
    // would put it at the top and bury the three distinct Unhealthy reports
    // under it.
    expect(shown()).toEqual([
      ["Unhealthy", "3"],
      ["Scheduled", "2"],
      ["BackOff", "1"],
    ]);
    expect(container.textContent).not.toContain("400");
  });

  it("names every row in words, so the dot is never the only signal", () => {
    rail();
    // The design draws these rows with a status dot whose label is EMPTY — a
    // bare coloured dot, silent to a screen reader and meaningless to a reader
    // who cannot see the colour. Every row here is named by the words already
    // in it, so there is no second string to drift from what is drawn.
    const named = ["Unhealthy 3", "Scheduled 2", "BackOff 1"].map((name) =>
      screen.getByRole("button", { name }),
    );
    // Every button in the rail is one of the three that answered to a name.
    expect(new Set(named).size).toBe(screen.getAllByRole("button").length);
  });

  it("tones a row from the type of the first event carrying that reason", () => {
    rail();

    // Asserted against core's own verdict rather than against the word
    // "danger": the tone rule lives in one place and this reads it there.
    expect(toneOf("Unhealthy 3")).toBe(eventVerdict("Warning").health);
    expect(toneOf("Scheduled 2")).toBe(eventVerdict("Normal").health);
    expect(toneOf("Unhealthy 3")).not.toBe(toneOf("Scheduled 2"));
  });

  it("reads the first event of a reason, not the last, for the tone", () => {
    // Kubernetes reuses a reason across both types — `Killing` is Normal on a
    // rollout and a Warning on an eviction. The design's rule is the first.
    rail([event("Killing", "Normal", 1, "k1"), event("Killing", "Warning", 1, "k2")]);
    expect(toneOf("Killing 2")).toBe(eventVerdict("Normal").health);
  });

  it("hands the reason back when a row is clicked", async () => {
    const user = userEvent.setup();
    const { onPick } = rail();

    await user.click(screen.getByRole("button", { name: "Scheduled 2" }));
    // The reason alone — the screen turns it into a query; the rail does not
    // know what a query is.
    expect(onPick.mock.calls).toEqual([["Scheduled"]]);
  });

  it("renders nothing at all when nothing is on screen", () => {
    // Not an empty box, not a heading with nothing under it: §8 leaves the
    // rail blank when the filter has emptied the table.
    const { container } = rail([]);
    expect(container.innerHTML).toBe("");
  });
});
