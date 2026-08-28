import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Meter } from "./Meter";

describe("Meter", () => {
  it("reports its value to assistive technology, not only visually", () => {
    render(<Meter value={30} ariaLabel="Memory" />);
    const meter = screen.getByRole("meter", { name: "Memory" });
    expect(meter.getAttribute("aria-valuenow")).toBe("30");
    expect(meter.getAttribute("aria-valuemax")).toBe("100");
  });

  it("picks a tone from the value, so a column of meters reads as a heat map", () => {
    const { container: ok } = render(<Meter value={10} ariaLabel="a" />);
    expect(ok.querySelector<HTMLElement>(".h-full")?.style.background).toContain("--ok");
    const { container: warn } = render(<Meter value={70} ariaLabel="b" />);
    expect(warn.querySelector<HTMLElement>(".h-full")?.style.background).toContain("--warn");
    const { container: sev } = render(<Meter value={95} ariaLabel="c" />);
    expect(sev.querySelector<HTMLElement>(".h-full")?.style.background).toContain("--sev");
  });

  it("keeps aria-valuenow inside the range it declares", () => {
    // 150 with a max of 100 is an invalid meter value; assistive technology may
    // clamp it or ignore the element. The real figure goes in aria-valuetext,
    // which is free text. (#317 review)
    render(<Meter value={150} ariaLabel="CPU" />);
    const meter = screen.getByRole("meter", { name: "CPU" });
    expect(meter.getAttribute("aria-valuenow")).toBe("100");
    expect(meter.getAttribute("aria-valuetext")).toBe("150%");
  });

  it("announces the plain value when it is inside the range", () => {
    render(<Meter value={42} ariaLabel="CPU" />);
    const meter = screen.getByRole("meter", { name: "CPU" });
    expect(meter.getAttribute("aria-valuenow")).toBe("42");
    expect(meter.getAttribute("aria-valuetext")).toBe("42%");
  });

  it("can be named by another element instead of a literal label", () => {
    render(
      <>
        <span id="mem-label">Memory</span>
        <Meter value={10} ariaLabelledBy="mem-label" />
      </>,
    );
    expect(screen.getByRole("meter", { name: "Memory" })).toBeDefined();
  });

  it("clamps the bar past 100 but keeps the real number", () => {
    // A pod over its limit genuinely reports more than 100%. The bar must not
    // run past its track — that reads as a rendering fault rather than a
    // reading — but hiding the true figure would be worse.
    const { container } = render(<Meter value={150} ariaLabel="CPU" />);
    expect(container.querySelector<HTMLElement>(".h-full")?.style.width).toBe("100%");
    expect(screen.getByText("150%")).toBeDefined();
  });

  it("does not render backwards for a negative value", () => {
    const { container } = render(<Meter value={-5} ariaLabel="CPU" />);
    expect(container.querySelector<HTMLElement>(".h-full")?.style.width).toBe("0%");
  });
});
