import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { TextArea } from "./TextArea";
import { TextInput } from "./TextInput";
import { Field } from "./Field";

/** Four screens styled their own multi-line box, three different ways (#1001, step 7c). */
describe("TextArea", () => {
  it("renders the value and emits value-first changes", () => {
    const onValueChange = vi.fn();
    render(<TextArea aria-label="Manifest" value="a: 1" onValueChange={onValueChange} />);
    fireEvent.change(screen.getByRole("textbox", { name: "Manifest" }), { target: { value: "a: 2" } });
    expect(onValueChange).toHaveBeenCalledWith("a: 2");
  });

  it("draws the box a TextInput draws", () => {
    render(
      <>
        <TextInput aria-label="One line" value="" onValueChange={() => {}} />
        <TextArea aria-label="Many lines" value="" onValueChange={() => {}} />
      </>,
    );
    const line = screen.getByRole("textbox", { name: "One line" });
    const area = screen.getByRole("textbox", { name: "Many lines" });
    expect(area.style.background).toBe(line.style.background);
    expect(area.style.borderColor).toBe(line.style.borderColor);
    for (const each of ["w-full", "rounded-md", "border", "px-2"]) expect(area.className.split(" ")).toContain(each);
  });

  it("sets what it holds in a fixed-width face, since it holds machine text", () => {
    // A manifest, a kubeconfig, namespace patterns, a skill's Markdown.
    render(<TextArea aria-label="Manifest" value="" onValueChange={() => {}} />);
    expect(screen.getByRole("textbox").className.split(" ")).toContain("font-mono");
  });

  it("marks itself invalid in colour and to assistive tech, and only when judged", () => {
    const { rerender } = render(<TextArea aria-label="Members" value="" onValueChange={() => {}} />);
    expect(screen.getByRole("textbox").getAttribute("aria-invalid")).toBeNull();
    rerender(<TextArea aria-label="Members" value="" onValueChange={() => {}} invalid />);
    expect(screen.getByRole("textbox").getAttribute("aria-invalid")).toBe("true");
    expect(screen.getByRole("textbox").style.borderColor).toBe("var(--sev)");
  });

  it("passes the textarea's own attributes through", () => {
    render(<TextArea aria-label="Kubeconfig" value="" onValueChange={() => {}} rows={12} spellCheck={false} disabled />);
    const area = screen.getByRole("textbox") as HTMLTextAreaElement;
    expect(area.rows).toBe(12);
    expect(area.getAttribute("spellcheck")).toBe("false");
    expect(area.disabled).toBe(true);
  });

  it("is labelled and described by the Field around it", () => {
    render(
      <Field label="Instructions" hint="What the agent should do.">
        <TextArea value="" onValueChange={() => {}} />
      </Field>,
    );
    const area = screen.getByRole("textbox", { name: "Instructions" });
    expect(area.getAttribute("aria-describedby")).toBeTruthy();
    expect(document.getElementById(area.getAttribute("aria-describedby")!)?.textContent).toBe("What the agent should do.");
  });
});
