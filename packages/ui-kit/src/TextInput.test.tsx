import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { TextInput } from "./TextInput";

/** The first three are the classic component's tests, carried over. (#318) */
describe("TextInput", () => {
  it("renders the value and emits value-first changes", () => {
    const onValueChange = vi.fn();
    render(<TextInput value="abc" onValueChange={onValueChange} />);
    const input = screen.getByDisplayValue("abc");
    fireEvent.change(input, { target: { value: "abcd" } });
    expect(onValueChange).toHaveBeenCalledWith("abcd");
  });

  it("calls onEnter when Enter is pressed", () => {
    const onEnter = vi.fn();
    render(<TextInput value="" onValueChange={() => {}} onEnter={onEnter} />);
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" });
    expect(onEnter).toHaveBeenCalledTimes(1);
  });

  it("does not throw on Enter without an onEnter handler", () => {
    render(<TextInput value="" onValueChange={() => {}} />);
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" });
    // no assertion needed; absence of a thrown error is the contract
  });

  it("calls onEscape when Escape is pressed", () => {
    const onEscape = vi.fn();
    render(<TextInput value="" onValueChange={() => {}} onEscape={onEscape} />);
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Escape" });
    expect(onEscape).toHaveBeenCalledTimes(1);
  });

  it("marks itself invalid for assistive technology, not just in colour", () => {
    // The mock signals an invalid field by reddening its border, which a screen
    // reader cannot see and a colour-blind user may not either.
    render(<TextInput value="" onValueChange={() => {}} invalid aria-label="Name" />);
    expect(screen.getByRole("textbox").getAttribute("aria-invalid")).toBe("true");
  });

  it("does not claim to be valid when nothing said it was", () => {
    // aria-invalid="false" on every field is noise; the attribute belongs only
    // on a field that has actually been judged.
    render(<TextInput value="" onValueChange={() => {}} aria-label="Name" />);
    expect(screen.getByRole("textbox").getAttribute("aria-invalid")).toBeNull();
  });
});
