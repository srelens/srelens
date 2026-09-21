import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/react";
import React from "react";
import { CodeEditor } from "./CodeEditor";

describe("CodeEditor", () => {
  it("mounts a CodeMirror editor showing the initial value", () => {
    const { container } = render(<CodeEditor value="kind: Pod" ariaLabel="Manifest YAML" />);
    const cm = container.querySelector(".cm-editor");
    expect(cm).not.toBeNull();
    expect(container.querySelector(".cm-content")?.textContent).toContain("kind: Pod");
    // aria-label is applied to the editable content for screen readers.
    expect(container.querySelector('[aria-label="Manifest YAML"]')).not.toBeNull();
  });

  it("does not call onChange while editable is disabled (read-only)", () => {
    const onChange = vi.fn();
    const { container } = render(<CodeEditor value="a: 1" readOnly onChange={onChange} />);
    expect(container.querySelector(".cm-editor")).not.toBeNull();
    expect(onChange).not.toHaveBeenCalled();
  });
});

describe("CodeEditor — taking the document away", () => {
  /** ⌘A as the browser delivers it with focus on the page, not the editor. */
  function pressSelectAll(): KeyboardEvent {
    const event = new KeyboardEvent("keydown", {
      key: "a",
      metaKey: true,
      bubbles: true,
      cancelable: true,
    });
    document.body.dispatchEvent(event);
    return event;
  }

  it("makes a read-only document a tab stop", () => {
    // A read-only view is `contenteditable="false"`, which the browser will
    // not focus and will not put a caret in — so the pane was unreachable by
    // keyboard, and any selection made in it was never the document's own
    // selection, which is what ⌘C copies. (#656)
    const { container } = render(<CodeEditor value="kind: Pod" readOnly ariaLabel="web manifest" />);
    expect(container.querySelector(".cm-content")?.getAttribute("tabindex")).toBe("0");
  });

  it("draws a focus indicator on the pane it made reachable", () => {
    // A tab stop with no caret and no ring is a keyboard reader with no idea
    // where they are. The stylesheet's own `:where(…, [tabindex])` ring does
    // not reach here — `:where()` carries no specificity and CodeMirror's base
    // theme sets `.cm-content { outline: none }` above it — so the editor
    // declares its own. Read off the stylesheet CodeMirror actually injected:
    // jsdom applies no CSS and resolves no `:focus-visible`, so the rule's
    // presence is what is observable. (#656 review)
    render(<CodeEditor value="kind: Pod" readOnly ariaLabel="web manifest" />);
    const sheets = [...document.querySelectorAll("style")].map((s) => s.textContent ?? "").join("");
    const at = sheets.indexOf(".cm-content[tabindex]:focus-visible");
    expect(at, "no focus indicator for a focusable read-only pane").toBeGreaterThan(-1);
    expect(sheets.slice(at, sheets.indexOf("}", at))).toContain("outline");
  });

  it("answers ⌘A pressed on the page by selecting the manifest", () => {
    // The complaint in #656: ⌘A on the manifest view selected every label and
    // table row AROUND the YAML and left the YAML itself out, because that is
    // what the browser's select-all does to a `contenteditable`.
    const { container } = render(
      <CodeEditor value={"kind: Pod\nmetadata:\n  name: web\n"} readOnly ariaLabel="web manifest" />,
    );
    const content = container.querySelector(".cm-content");
    expect(document.activeElement).not.toBe(content);

    const event = pressSelectAll();

    expect(event.defaultPrevented).toBe(true);
    // Focus is the half that makes the selection the clipboard's: CodeMirror
    // writes the DOM selection only for a view that has focus.
    expect(document.activeElement).toBe(content);
  });

  it("renders no Copy control unless the caller asks for one", () => {
    const { queryByRole } = render(<CodeEditor value="kind: Pod" ariaLabel="web manifest" />);
    expect(queryByRole("button", { name: /copy/i })).toBeNull();
  });

  it("puts the whole document on the clipboard", async () => {
    const writeText = vi.fn(async () => {});
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    const yaml = "kind: Pod\nmetadata:\n  name: web\n";
    const { getByRole, findByText } = render(
      <CodeEditor value={yaml} readOnly copy ariaLabel="web manifest" />,
    );

    getByRole("button", { name: /copy/i }).click();

    await findByText("Copied");
    expect(writeText).toHaveBeenCalledWith(yaml);
    vi.unstubAllGlobals();
  });

  it("says so when the clipboard refuses, rather than repainting nothing", async () => {
    // "Copied" over an empty clipboard is the outcome that actually misleads;
    // silence is the one that leaves the reader believing they have the text.
    // `navigator.clipboard` is absent on a non-secure origin and can be
    // refused outright, so this is a path readers reach. (#656 review)
    const writeText = vi.fn(async () => {
      throw new Error("denied");
    });
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    const { getByRole, findByText, queryByText } = render(
      <CodeEditor value="kind: Pod" readOnly copy ariaLabel="web manifest" />,
    );

    getByRole("button", { name: /copy/i }).click();

    await findByText("Copy failed");
    expect(queryByText("Copied")).toBeNull();
    vi.unstubAllGlobals();
  });
});
