import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Checkbox } from "./Checkbox";

describe("Checkbox", () => {
  it("renders the label and reflects the checked state", () => {
    render(<Checkbox checked onChange={() => {}} label="Select all" />);
    const box = screen.getByRole("checkbox", { name: "Select all" }) as HTMLInputElement;
    expect(box.checked).toBe(true);
  });

  it("reports the state it is moving to", async () => {
    const onChange = vi.fn();
    render(<Checkbox checked={false} onChange={onChange} label="Select all" />);
    await userEvent.click(screen.getByRole("checkbox", { name: "Select all" }));
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("toggles from the keyboard", async () => {
    // Space is the native activation key, and the whole reason this is an
    // `<input>` rather than a styled div. (#320)
    const onChange = vi.fn();
    render(<Checkbox checked={false} onChange={onChange} label="Select all" />);
    await userEvent.tab();
    expect(document.activeElement).toBe(screen.getByRole("checkbox", { name: "Select all" }));
    await userEvent.keyboard(" ");
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("is selected by clicking its label text", async () => {
    const onChange = vi.fn();
    render(<Checkbox checked={false} onChange={onChange} label="Select all" />);
    await userEvent.click(screen.getByText("Select all"));
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("takes a name from the caller when there is no visible label", () => {
    render(<Checkbox checked={false} onChange={() => {}} ariaLabel="Select every row" />);
    expect(screen.getByRole("checkbox", { name: "Select every row" })).toBeDefined();
  });

  it("does not repeat a visible label in aria-label", () => {
    // Two names for one control: the visible one is the name, and a duplicate
    // aria-label is at best noise and at worst a different wording. (#320)
    render(<Checkbox checked={false} onChange={() => {}} label="Select all" ariaLabel="Select every row" />);
    const box = screen.getByRole("checkbox", { name: "Select all" });
    expect(box.getAttribute("aria-label")).toBeNull();
  });

  it("omits the label wrapper entirely when the slot resolved to false", () => {
    // Not `label != null`: `label={showLabels && "Select all"}` is the ordinary
    // way to make it conditional, and an empty wrapper still takes its gap.
    const { container } = render(<Checkbox checked={false} onChange={() => {}} label={false} />);
    expect(container.querySelector("label")).toBeNull();
    expect(screen.getByRole("checkbox")).toBeDefined();
  });

  it("blocks the change when disabled", async () => {
    const onChange = vi.fn();
    render(<Checkbox checked={false} onChange={onChange} label="Select all" disabled />);
    const box = screen.getByRole("checkbox", { name: "Select all" }) as HTMLInputElement;
    expect(box.disabled).toBe(true);
    await userEvent.click(box);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("shows the third state for a partial selection", () => {
    render(<Checkbox checked={false} onChange={() => {}} label="Select all" indeterminate />);
    const box = screen.getByRole("checkbox", { name: "Select all" }) as HTMLInputElement;
    expect(box.indeterminate).toBe(true);
  });

  it("clears the third state when the selection is no longer partial", () => {
    const { rerender } = render(<Checkbox checked={false} onChange={() => {}} label="Select all" indeterminate />);
    rerender(<Checkbox checked onChange={() => {}} label="Select all" />);
    const box = screen.getByRole("checkbox", { name: "Select all" }) as HTMLInputElement;
    expect(box.indeterminate).toBe(false);
  });

  it("keeps the third state after a click that cleared it in the DOM", async () => {
    // The browser clears `indeterminate` the moment the user clicks the box, so
    // a component that only writes the property when the prop changes loses the
    // dash on the first click and never gets it back. (#320)
    const onChange = vi.fn();
    const props = { checked: false, onChange, label: "Select all", indeterminate: true };
    const { rerender } = render(<Checkbox {...props} />);
    const box = screen.getByRole("checkbox", { name: "Select all" }) as HTMLInputElement;
    await userEvent.click(box);
    rerender(<Checkbox {...props} />);
    expect(box.indeterminate).toBe(true);
  });

  it("forwards className onto the labelled row", () => {
    const { container } = render(<Checkbox checked={false} onChange={() => {}} label="Select all" className="extra" />);
    expect(container.querySelector("label.extra")).not.toBeNull();
  });

  it("forwards className onto the box when there is no label", () => {
    const { container } = render(<Checkbox checked={false} onChange={() => {}} ariaLabel="Select all" className="extra" />);
    expect(container.querySelector("input.extra")).not.toBeNull();
  });
});
