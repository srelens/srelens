import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Field } from "./Field";
import { TextInput } from "./TextInput";

describe("Field", () => {
  it("labels the control it wraps", () => {
    // Implicit association: the classic component rendered the label as a bare
    // sibling with no htmlFor, so nothing tied the two together and clicking
    // the label did nothing. Wrapping is what the new design's markup does.
    render(
      <Field label="Namespace">
        <TextInput value="" onValueChange={() => {}} />
      </Field>,
    );
    expect(screen.getByLabelText("Namespace")).toBeDefined();
  });

  it("shows a hint under the control", () => {
    render(
      <Field label="Name" hint="Lowercase letters only">
        <input />
      </Field>,
    );
    const control = screen.getByRole("textbox", { name: "Name" });
    const hint = screen.getByText("Lowercase letters only");
    expect(control.getAttribute("aria-describedby")).toBe(hint.id);
  });

  it("shows the error instead of the hint, never both", () => {
    // Two lines of small print under one control, one of them stale advice,
    // reads as a rendering fault.
    render(
      <Field label="Name" hint="Lowercase letters only" error="Already taken">
        <input />
      </Field>,
    );
    const control = screen.getByRole("textbox", { name: "Name" });
    const error = screen.getByText("Already taken");
    expect(control.getAttribute("aria-describedby")).toBe(error.id);
    expect(screen.queryByText("Lowercase letters only")).toBeNull();
  });

  it("keeps a control's existing description when it adds the field note", () => {
    render(
      <Field label="Name" hint="Lowercase letters only">
        <input aria-describedby="server-rule" />
      </Field>,
    );
    const control = screen.getByRole("textbox", { name: "Name" });
    const hint = screen.getByText("Lowercase letters only");
    expect(control.getAttribute("aria-describedby")?.split(/\s+/)).toEqual([
      "server-rule",
      hint.id,
    ]);
  });

  it("keeps an action outside the label element", () => {
    // A <button> is a labelable element, so nesting it inside the <label> makes
    // a label click activate it and swallows its name into the label's name
    // computation. Carried over from the classic component, which documented
    // this and got it right.
    render(
      <Field label="Manifest" action={<button>Preview</button>}>
        <input />
      </Field>,
    );
    const action = screen.getByRole("button", { name: "Preview" });
    expect(action.closest("label")).toBeNull();
    expect(screen.getByLabelText("Manifest")).toBe(screen.getByRole("textbox"));
  });

  it("forwards className", () => {
    const { container } = render(
      <Field label="X" className="extra">
        <input />
      </Field>,
    );
    expect(container.querySelector(".extra")).not.toBeNull();
  });
});
