import { describe, it, expect } from "vitest";
import { render } from "@testing-library/react";
import { ListCount } from "./ListCount";

/** What a sighted reader sees, and what a screen reader is told. */
function draw(props: Parameters<typeof ListCount>[0]) {
  const { container } = render(<ListCount {...props} />);
  const slot = container.querySelector('[data-slot="list-count"]')!;
  return {
    slot,
    visible: slot.querySelector('[aria-hidden="true"]')!.textContent,
    spoken: slot.querySelector(".sr-only")!.textContent,
  };
}

describe("ListCount", () => {
  it("says the number with the word when nothing is narrowed", () => {
    expect(draw({ total: 426, shown: 426, noun: "pods" }).visible).toBe("426 items");
  });

  it("says filtered over full, bare, in the same slot while the filter hides rows", () => {
    const { visible, slot } = draw({ total: 426, shown: 3, noun: "pods" });
    expect(visible).toBe("3 / 426");
    // One element in one of two states, not a total plus a second line.
    expect(slot.querySelectorAll('[aria-hidden="true"]')).toHaveLength(1);
  });

  it("goes back to the number and the word when the filter matches every row again", () => {
    expect(draw({ total: 10, shown: 10, noun: "pods" }).visible).toBe("10 items");
  });

  it("shows a filter that matches nothing as nought of the total, not as an empty list", () => {
    expect(draw({ total: 426, shown: 0, noun: "pods" }).visible).toBe("0 / 426");
  });

  it("says 0 items for a list that answered with none", () => {
    expect(draw({ total: 0, shown: 0, noun: "pods" }).visible).toBe("0 items");
  });

  it("says 1 item, not 1 items", () => {
    expect(draw({ total: 1, shown: 1, noun: "pods" }).visible).toBe("1 item");
  });

  it("groups thousands the way the rest of the app writes them", () => {
    expect(draw({ total: 5416, shown: 5416, noun: "pods" }).visible).toBe("5 416 items");
    expect(draw({ total: 5416, shown: 1200, noun: "pods" }).visible).toBe("1 200 / 5 416");
  });

  it("marks a total the list was cut off at as a floor, in both states", () => {
    // The backend stopped at its row cap: there are at least this many.
    expect(draw({ total: 5000, shown: 5000, truncated: true, noun: "pods" }).visible).toBe("5 000+ items");
    expect(draw({ total: 5000, shown: 12, truncated: true, noun: "pods" }).visible).toBe("12 / 5 000+");
    expect(draw({ total: 1, shown: 1, truncated: true, noun: "pods" }).visible).toBe("1+ items");
  });

  it("tells a screen reader the same fact in words, naming what is counted", () => {
    expect(draw({ total: 426, shown: 426, noun: "pods" }).spoken).toBe("426 pods");
    expect(draw({ total: 426, shown: 3, noun: "pods" }).spoken).toBe("3 of 426 pods shown");
    expect(draw({ total: 5000, shown: 12, truncated: true, noun: "pods" }).spoken).toBe(
      "12 of more than 5 000 pods shown",
    );
    expect(draw({ total: 5000, shown: 5000, truncated: true, noun: "pods" }).spoken).toBe("More than 5 000 pods");
  });

  it("is a polite live region, so narrowing the table is announced without interrupting", () => {
    const { slot } = draw({ total: 426, shown: 3, noun: "pods" });
    expect(slot.getAttribute("role")).toBe("status");
    expect(slot.getAttribute("aria-live")).toBe("polite");
  });
});
