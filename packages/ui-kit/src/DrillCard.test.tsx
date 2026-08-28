import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DrillCard, type DrillStep } from "./DrillCard";

const STEPS: DrillStep[] = [
  { id: "signal", label: "Signal", content: <p>5xx rate rising</p> },
  { id: "diagnose", label: "Diagnose", content: <p>Pool exhausted</p> },
  { id: "act", label: "Act", content: <p>Roll back to 118</p> },
];

function setup(props: Partial<Parameters<typeof DrillCard>[0]> = {}) {
  const onActiveChange = vi.fn();
  const view = render(
    <DrillCard steps={STEPS} active="signal" onActiveChange={onActiveChange} {...props} />,
  );
  return { onActiveChange, ...view };
}

const step = (name: string) => screen.getByRole("tab", { name });

describe("DrillCard", () => {
  it("renders a step per move, in order", () => {
    setup();
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual([
      "01Signal",
      "02Diagnose",
      "03Act",
    ]);
  });

  it("shows only the step it is on", () => {
    setup();
    expect(screen.getByText("5xx rate rising")).toBeDefined();
    expect(screen.queryByText("Pool exhausted")).toBeNull();
  });

  it("marks the step it is on", () => {
    setup({ active: "diagnose" });
    expect(step("Diagnose").getAttribute("aria-selected")).toBe("true");
    expect(step("Signal").getAttribute("aria-selected")).toBe("false");
  });

  it("emits the step id on click", async () => {
    const { onActiveChange } = setup();
    await userEvent.click(step("Act"));
    expect(onActiveChange).toHaveBeenCalledWith("act");
  });

  it("keeps the numbers out of the spoken name", () => {
    // "zero one Signal" is not what the step is called; the tablist already
    // says which of three this is.
    setup();
    expect(screen.getByRole("tab", { name: "Signal" })).toBeDefined();
  });

  it("numbers past nine without mangling the digits", () => {
    // The mock wrote `0{i + 1}`, which turns step ten into "010".
    const many = Array.from({ length: 10 }, (_, i) => ({
      id: `s${i}`,
      label: `Step ${i}`,
      content: <p>body {i}</p>,
    }));
    setup({ steps: many, active: "s0" });
    expect(screen.getByRole("tab", { name: "Step 9" }).textContent).toBe("10Step 9");
  });

  it("gives every button it owns an explicit type", () => {
    // A bare <button> inside a form submits it.
    setup();
    for (const tab of screen.getAllByRole("tab")) {
      expect(tab.getAttribute("type")).toBe("button");
    }
  });
});

describe("DrillCard panel", () => {
  it("labels the panel with the step it belongs to", () => {
    setup();
    expect(screen.getByRole("tabpanel", { name: "Signal" })).toBeDefined();
  });

  it("points the active step at the panel it controls", () => {
    setup();
    const panel = screen.getByRole("tabpanel");
    expect(step("Signal").getAttribute("aria-controls")).toBe(panel.getAttribute("id"));
  });

  it("does not point the other steps at a panel that is not there", () => {
    // Only the active step's panel is rendered, so an `aria-controls` on the
    // rest would name an element that does not exist.
    setup();
    expect(step("Act").getAttribute("aria-controls")).toBeNull();
  });

  it("is reachable by keyboard, because it scrolls", () => {
    setup();
    expect(screen.getByRole("tabpanel").getAttribute("tabindex")).toBe("0");
  });

  it("renders the step's own actions", () => {
    setup({
      steps: [{ ...STEPS[0], actions: <button type="button">Inspect workload</button> }],
      active: "signal",
    });
    expect(screen.getByRole("button", { name: "Inspect workload" })).toBeDefined();
  });

  it("buys no space for actions that resolved to nothing", () => {
    const { container } = setup({
      steps: [{ ...STEPS[0], actions: false }],
      active: "signal",
    });
    expect(container.querySelector("[data-slot='actions']")).toBeNull();
  });
});

describe("DrillCard head", () => {
  it("renders the title as a heading", () => {
    setup({ title: "Incident drill" });
    expect(screen.getByRole("heading", { level: 2, name: "Incident drill" })).toBeDefined();
  });

  it("renders what the app puts beside the title", () => {
    setup({ title: "Incident drill", headerAction: <span>Live</span> });
    expect(screen.getByText("Live")).toBeDefined();
  });

  it("omits the header entirely when there is nothing to put in it", () => {
    const { container } = setup();
    expect(container.querySelector(".card-head")).toBeNull();
  });

  it("omits the header when both slots resolved to false", () => {
    const { container } = setup({ title: false, headerAction: false });
    expect(container.querySelector(".card-head")).toBeNull();
  });
});

describe("DrillCard flush", () => {
  it("drops the card chrome so it can fill a pane", () => {
    const { container } = setup({ flush: true });
    expect(container.querySelector(".card")).toBeNull();
  });

  it("keeps the card chrome otherwise", () => {
    const { container } = setup();
    expect(container.querySelector(".card")).not.toBeNull();
  });
});

describe("DrillCard with an active step it does not have", () => {
  it("falls back to the first step rather than showing a blank card", () => {
    // A step id can outlive the steps it came from — a saved drill reopened
    // against a shorter list.
    setup({ active: "nowhere" });
    expect(screen.getByText("5xx rate rising")).toBeDefined();
    expect(step("Signal").getAttribute("aria-selected")).toBe("true");
  });
});

describe("DrillCard with no steps", () => {
  it("says so", () => {
    setup({ steps: [] });
    expect(screen.getByText("Nothing to drill into")).toBeDefined();
  });

  it("takes its own wording", () => {
    setup({ steps: [], emptyLabel: "No investigation yet" });
    expect(screen.getByText("No investigation yet")).toBeDefined();
  });

  it("renders no rail and no panel", () => {
    setup({ steps: [] });
    expect(screen.queryByRole("tablist")).toBeNull();
    expect(screen.queryByRole("tabpanel")).toBeNull();
  });
});

/**
 * The keyboard contract the tab roles promise. A tablist where every step is a
 * Tab stop and the arrows do nothing is worse than three plain buttons, because
 * the roles told assistive technology to expect otherwise. Matches `Tabs`.
 */
describe("DrillCard keyboard behaviour", () => {
  it("names the rail", () => {
    setup({ railLabel: "Investigation steps" });
    expect(screen.getByRole("tablist", { name: "Investigation steps" })).toBeDefined();
  });

  it("is a single tab stop, landing on the step it is on", () => {
    setup({ active: "diagnose" });
    expect(step("Diagnose").getAttribute("tabindex")).toBe("0");
    expect(step("Signal").getAttribute("tabindex")).toBe("-1");
    expect(step("Act").getAttribute("tabindex")).toBe("-1");
  });

  it("moves right and left with the arrow keys", async () => {
    const { onActiveChange } = setup();
    step("Signal").focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(onActiveChange).toHaveBeenLastCalledWith("diagnose");
    onActiveChange.mockClear();
    // Back to Signal, not on to Act: navigation follows the focused step, not
    // the `active` prop, which a controlled parent may not have committed yet.
    await userEvent.keyboard("{ArrowLeft}");
    expect(onActiveChange).toHaveBeenLastCalledWith("signal");
  });

  it("wraps at both ends", async () => {
    const { onActiveChange, rerender } = setup({ active: "act" });
    step("Act").focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(onActiveChange).toHaveBeenLastCalledWith("signal");

    onActiveChange.mockClear();
    rerender(<DrillCard steps={STEPS} active="signal" onActiveChange={onActiveChange} />);
    step("Signal").focus();
    await userEvent.keyboard("{ArrowLeft}");
    expect(onActiveChange).toHaveBeenLastCalledWith("act");
  });

  it("jumps to the first and last with Home and End", async () => {
    const { onActiveChange } = setup({ active: "diagnose" });
    step("Diagnose").focus();
    await userEvent.keyboard("{End}");
    expect(onActiveChange).toHaveBeenLastCalledWith("act");
    onActiveChange.mockClear();
    await userEvent.keyboard("{Home}");
    expect(onActiveChange).toHaveBeenLastCalledWith("signal");
  });

  it("leaves other keys alone", async () => {
    const { onActiveChange } = setup();
    step("Signal").focus();
    await userEvent.keyboard("{ArrowDown}");
    await userEvent.keyboard("a");
    expect(onActiveChange).not.toHaveBeenCalled();
  });
});
