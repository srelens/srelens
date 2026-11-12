import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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

  it("passes the input's own attributes through, for a typed field", () => {
    // An app's typed setting bounds its value and names itself from a legend
    // (#1001, step 7c): these went on a bare <input> before.
    render(
      <TextInput
        value=""
        onValueChange={() => {}}
        type="url"
        maxLength={64}
        spellCheck={false}
        autoComplete="off"
        aria-required
        aria-labelledby="title"
      />,
    );
    const input = screen.getByRole("textbox") as HTMLInputElement;
    expect(input.type).toBe("url");
    expect(input.maxLength).toBe(64);
    expect(input.getAttribute("spellcheck")).toBe("false");
    expect(input.getAttribute("autocomplete")).toBe("off");
    expect(input.getAttribute("aria-required")).toBe("true");
    expect(input.getAttribute("aria-labelledby")).toBe("title");
  });

  it("bounds a number", () => {
    render(<TextInput aria-label="Port" value="" onValueChange={() => {}} type="number" inputMode="numeric" min={1} max={65535} step={1} />);
    const input = screen.getByRole("spinbutton", { name: "Port" }) as HTMLInputElement;
    expect([input.min, input.max, input.step, input.inputMode]).toEqual(["1", "65535", "1", "numeric"]);
  });

  it("tells the caller when it gains and loses focus", () => {
    const onFocus = vi.fn();
    const onBlur = vi.fn();
    render(<TextInput aria-label="Rename" value="" onValueChange={() => {}} onFocus={onFocus} onBlur={onBlur} />);
    fireEvent.focus(screen.getByRole("textbox"));
    fireEvent.blur(screen.getByRole("textbox"));
    expect([onFocus.mock.calls.length, onBlur.mock.calls.length]).toEqual([1, 1]);
  });

  it("keeps an Enter it hands to onEnter out of the form around it", async () => {
    // A secret saved with Enter must not also submit the settings form it sits in.
    const onEnter = vi.fn();
    const onSubmit = vi.fn((event: { preventDefault(): void }) => event.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <TextInput aria-label="Secret" value="" onValueChange={() => {}} onEnter={onEnter} />
        <button type="submit">Save</button>
      </form>,
    );
    await userEvent.type(screen.getByRole("textbox"), "{Enter}");
    expect(onEnter).toHaveBeenCalledTimes(1);
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("keeps an Escape it hands to onEscape from going further", () => {
    // A rename field closes on Escape and hands focus back to its row; the
    // rest of the keystroke must not then act on the row.
    render(<TextInput aria-label="Rename" value="" onValueChange={() => {}} onEscape={() => {}} />);
    expect(fireEvent.keyDown(screen.getByRole("textbox"), { key: "Escape" })).toBe(false);
  });

  it("leaves an Enter or Escape that an input method is composing to the input method", () => {
    // Mid-composition, Enter picks a candidate and Escape drops one: neither is about the field.
    const onEnter = vi.fn();
    const onEscape = vi.fn();
    render(<TextInput aria-label="Name" value="" onValueChange={() => {}} onEnter={onEnter} onEscape={onEscape} />);
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter", isComposing: true });
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Escape", isComposing: true });
    expect([onEnter.mock.calls.length, onEscape.mock.calls.length]).toEqual([0, 0]);
  });

  it("leaves Enter to the form when nothing else takes it", async () => {
    const onSubmit = vi.fn((event: { preventDefault(): void }) => event.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <TextInput aria-label="Name" value="" onValueChange={() => {}} />
        <button type="submit">Save</button>
      </form>,
    );
    await userEvent.type(screen.getByRole("textbox"), "{Enter}");
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });
});


it("keeps a visible control boundary and does not suppress keyboard focus", () => {
  render(<TextInput value="" onValueChange={() => {}} aria-label="Name" />);
  const control = screen.getByRole("textbox");
  expect(control.className).not.toContain("outline-none");
  expect(control.style.borderColor).toBe("var(--control-line)");
});

it("uses readable theme ink for placeholder text instead of half-transparent browser ink", () => {
  render(<TextInput value="" onValueChange={() => {}} placeholder="Find a resource" />);
  expect(screen.getByRole("textbox").className).toContain("placeholder:text-faint");
});
