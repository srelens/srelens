import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Progress } from "./Progress";

const fill = (container: HTMLElement) =>
  container.querySelector<HTMLElement>('[data-slot="progress-fill"]');

describe("Progress", () => {
  it("reports how far along it is, not only draws it", () => {
    render(<Progress value={30} ariaLabel="Rollout" />);
    const bar = screen.getByRole("progressbar", { name: "Rollout" });
    expect(bar.getAttribute("aria-valuenow")).toBe("30");
    expect(bar.getAttribute("aria-valuemin")).toBe("0");
    expect(bar.getAttribute("aria-valuemax")).toBe("100");
  });

  it("can be named by another element instead of a literal label", () => {
    render(
      <>
        <span id="rollout-label">Rollout</span>
        <Progress value={10} ariaLabelledBy="rollout-label" />
      </>,
    );
    expect(screen.getByRole("progressbar", { name: "Rollout" })).toBeDefined();
  });

  it("clamps the bar past 100 but keeps the true figure", () => {
    // A bar that runs past its track reads as a rendering fault rather than a
    // reading, and hiding the real number would be worse. (#317, #320)
    const { container } = render(<Progress value={150} ariaLabel="Rollout" label="Rollout" />);
    expect(fill(container)?.style.width).toBe("100%");
    expect(screen.getByText("150%")).toBeDefined();
  });

  it("keeps aria-valuenow inside the range it declares", () => {
    // A value outside its own min/max is invalid: assistive technology may
    // clamp it silently or skip the element. aria-valuetext is free text, so
    // the real figure still reaches a screen reader. (#317)
    render(<Progress value={150} ariaLabel="Rollout" />);
    const bar = screen.getByRole("progressbar", { name: "Rollout" });
    expect(bar.getAttribute("aria-valuenow")).toBe("100");
    expect(bar.getAttribute("aria-valuetext")).toBe("150%");
  });

  it("does not render backwards for a negative value", () => {
    const { container } = render(<Progress value={-5} ariaLabel="Rollout" />);
    expect(fill(container)?.style.width).toBe("0%");
    const bar = screen.getByRole("progressbar", { name: "Rollout" });
    expect(bar.getAttribute("aria-valuenow")).toBe("0");
    expect(bar.getAttribute("aria-valuetext")).toBe("-5%");
  });

  it("announces the plain value when it is inside the range", () => {
    render(<Progress value={42} ariaLabel="Rollout" />);
    const bar = screen.getByRole("progressbar", { name: "Rollout" });
    expect(bar.getAttribute("aria-valuenow")).toBe("42");
    expect(bar.getAttribute("aria-valuetext")).toBe("42%");
  });

  it("rounds the figure it prints and the one it announces", () => {
    // These come out of ratios: 1 of 3 pods updated is 33.33333333333333.
    const { container } = render(
      <Progress value={(1 / 3) * 100} ariaLabel="Rollout" label="Rollout" />,
    );
    expect(container.textContent).toContain("33%");
    expect(container.textContent).not.toContain("33.3");
    expect(screen.getByRole("progressbar").getAttribute("aria-valuetext")).toBe("33%");
  });

  it("keeps the unrounded value for the bar itself", () => {
    const { container } = render(<Progress value={(1 / 3) * 100} ariaLabel="Rollout" />);
    expect(fill(container)?.style.width).toBe(`${(1 / 3) * 100}%`);
  });

  it("renders the label beside the figure", () => {
    const { container } = render(<Progress value={40} ariaLabel="Rollout" label="Rollout" />);
    const head = container.querySelector('[data-slot="progress-head"]');
    expect(head?.textContent).toContain("Rollout");
    expect(head?.textContent).toContain("40%");
  });

  it("omits the label row entirely, not just its text", () => {
    // The row holds a line box and a bottom margin; an empty one makes a bare
    // bar sit lower than its neighbours in the same column.
    const { container } = render(<Progress value={40} ariaLabel="Rollout" />);
    expect(container.querySelector('[data-slot="progress-head"]')).toBeNull();
    expect(container.textContent).toBe("");
  });

  it("treats a conditional label that resolved to false as absent", () => {
    const { container } = render(<Progress value={40} ariaLabel="Rollout" label={false} />);
    expect(container.querySelector('[data-slot="progress-head"]')).toBeNull();
  });

  it("shows the percentage exactly once when it is labelled", () => {
    const { container } = render(<Progress value={40} ariaLabel="Rollout" label="Rollout" />);
    expect(container.textContent?.match(/40%/g)).toHaveLength(1);
  });

  it("is nothing but the bar when it has no label row", () => {
    // An unadorned progress bar stays the element its callers lay out, rather
    // than sitting inside a wrapper div that is there for a row that is not.
    // Meter settled this the same way. (#318)
    const { container } = render(<Progress value={40} ariaLabel="Rollout" />);
    expect(container.firstElementChild?.getAttribute("role")).toBe("progressbar");
  });

  it("forwards className onto the outermost element when there is a label row", () => {
    // The caller's spacing belongs outside the caption, not between it and the
    // bar. That wrapper carries no classes of its own, so there is nothing here
    // for the caller's to replace.
    const { container } = render(
      <Progress value={40} ariaLabel="Rollout" label="Rollout" className="extra" />,
    );
    const root = container.firstElementChild as HTMLElement;
    expect(root.classList.contains("extra")).toBe(true);
    expect(root.querySelector('[data-slot="progress-head"]')).not.toBeNull();
  });

  it("forwards className onto the bar when there is no label row", () => {
    // The root is the bar in that case; the caller's spacing has to land on
    // whatever the outermost element turns out to be.
    const { container } = render(<Progress value={40} ariaLabel="Rollout" className="extra" />);
    const root = container.firstElementChild as HTMLElement;
    expect(root.getAttribute("role")).toBe("progressbar");
    expect(root.classList.contains("extra")).toBe(true);
    expect(root.className.trim()).not.toBe("extra");
  });
});
