import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { FormEvent } from "react";
import { Switch } from "./Switch";

describe("Switch", () => {
  it("renders the label and the hint, and reports its state", () => {
    render(<Switch on label="Live updates" hint="Streams changes as they happen" onChange={() => {}} />);
    const control = screen.getByRole("switch", { name: "Live updates" });
    expect(control.getAttribute("aria-checked")).toBe("true");
    expect(screen.getByText("Streams changes as they happen")).toBeDefined();
  });

  it("reports the state it is moving to", async () => {
    const onChange = vi.fn();
    render(<Switch on={false} label="Live updates" onChange={onChange} />);
    await userEvent.click(screen.getByRole("switch", { name: "Live updates" }));
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("toggles from the keyboard", async () => {
    const onChange = vi.fn();
    render(<Switch on label="Live updates" onChange={onChange} />);
    await userEvent.tab();
    expect(document.activeElement).toBe(screen.getByRole("switch", { name: "Live updates" }));
    await userEvent.keyboard(" ");
    expect(onChange).toHaveBeenLastCalledWith(false);
    await userEvent.keyboard("{Enter}");
    expect(onChange).toHaveBeenLastCalledWith(false);
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it("is toggled by clicking its label text", async () => {
    const onChange = vi.fn();
    render(<Switch on={false} label="Live updates" onChange={onChange} />);
    await userEvent.click(screen.getByText("Live updates"));
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("takes a name from the caller when there is no visible label", () => {
    render(<Switch on={false} ariaLabel="Live updates" onChange={() => {}} />);
    expect(screen.getByRole("switch", { name: "Live updates" })).toBeDefined();
  });

  it("does not repeat a visible label in aria-label", () => {
    // The mock set both, so a labelled switch was named twice and an unlabelled
    // one — `aria-label={undefined}` — was not named at all. (#320)
    render(<Switch on={false} label="Live updates" ariaLabel="Toggle live updates" onChange={() => {}} />);
    expect(screen.getByRole("switch", { name: "Live updates" }).getAttribute("aria-label")).toBeNull();
  });

  it("describes itself with the hint rather than folding it into the name", () => {
    render(<Switch on label="Live updates" hint="Streams changes as they happen" onChange={() => {}} />);
    const control = screen.getByRole("switch", { name: "Live updates" });
    const describedBy = control.getAttribute("aria-describedby");
    expect(describedBy).not.toBeNull();
    expect(document.getElementById(describedBy!)?.textContent).toBe("Streams changes as they happen");
  });

  it("omits the hint cleanly when the slot resolved to false", () => {
    const { container } = render(<Switch on={false} label="Live updates" hint={false} onChange={() => {}} />);
    expect(container.querySelector(".text-muted")).toBeNull();
    expect(screen.getByRole("switch", { name: "Live updates" }).getAttribute("aria-describedby")).toBeNull();
  });

  it("does not submit the form it is sitting in", async () => {
    // A bare <button> inside a form is a submit button, so toggling a setting
    // would send the form. This one is `type="button"`. (#320)
    const onSubmit = vi.fn((e: FormEvent) => e.preventDefault());
    const onChange = vi.fn();
    render(
      <form onSubmit={onSubmit}>
        <Switch on={false} label="Live updates" onChange={onChange} />
      </form>,
    );
    const control = screen.getByRole("switch", { name: "Live updates" });
    expect(control.getAttribute("type")).toBe("button");
    await userEvent.click(control);
    expect(onChange).toHaveBeenCalledWith(true);
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("blocks the change when disabled", async () => {
    const onChange = vi.fn();
    render(<Switch on={false} label="Live updates" onChange={onChange} disabled />);
    const control = screen.getByRole("switch", { name: "Live updates" }) as HTMLButtonElement;
    expect(control.disabled).toBe(true);
    await userEvent.click(control);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("colours the thumb from a token in both states", () => {
    // `bg-white` is a fixed colour on a themed control: on the dark themes the
    // thumb is then the one part of the switch that ignores the theme. (#320)
    const { container, rerender } = render(<Switch on={false} ariaLabel="Live updates" onChange={() => {}} />);
    const thumb = () => container.querySelector("span") as HTMLElement;
    expect(thumb().className).not.toMatch(/bg-white|text-black/);
    expect(thumb().style.background).toMatch(/^var\(--/);
    rerender(<Switch on ariaLabel="Live updates" onChange={() => {}} />);
    expect(thumb().style.background).toMatch(/^var\(--/);
  });

  it("marks a destructive setting with the severity colour", () => {
    render(<Switch on danger ariaLabel="Delete on drop" onChange={() => {}} />);
    const control = screen.getByRole("switch", { name: "Delete on drop" });
    expect((control as HTMLElement).style.background).toBe("var(--sev)");
  });

  it("forwards className onto the labelled row", () => {
    const { container } = render(<Switch on={false} label="Live updates" className="extra" onChange={() => {}} />);
    expect(container.querySelector(".extra")).not.toBeNull();
  });

  it("forwards className onto the control when there is no label", () => {
    render(<Switch on={false} ariaLabel="Live updates" className="extra" onChange={() => {}} />);
    expect(screen.getByRole("switch", { name: "Live updates" }).className).toContain("extra");
  });
});
