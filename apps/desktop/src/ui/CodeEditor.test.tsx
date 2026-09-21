import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/react";
import React from "react";
import { EditorView } from "@codemirror/view";
import { openSearchPanel } from "@codemirror/search";
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

  it("copies what is in the editor NOW, not the text it was mounted with", async () => {
    // `onChange` is optional, so an editable editor is free to hold a document
    // the caller has never been told about — and a Copy that reads the `value`
    // prop would hand over the text from mount while the reader looks at what
    // they have typed. Read at the click instead. (#656 review)
    const writeText = vi.fn(async () => {});
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    const { container, getByRole, findByText } = render(
      <CodeEditor value="kind: Pod" copy ariaLabel="web manifest" />,
    );

    // Typed into, the way CodeMirror delivers it — not by replacing `value`,
    // which is the path that already works.
    const view = EditorView.findFromDOM(container.querySelector(".cm-editor")!)!;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: "kind: Service" } });

    getByRole("button", { name: /copy/i }).click();

    await findByText("Copied");
    expect(writeText).toHaveBeenCalledWith("kind: Service");
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

/**
 * Every CSS rule CodeMirror has mounted, in the order a browser applies them.
 *
 * Style modules write one rule per line into a `<style>` in the head, base
 * themes first. That order is the point here: a base theme and this editor's
 * theme reach the same elements at the same specificity, so the rule written
 * last is the one that wins.
 */
function mountedRules(): string[] {
  return [...document.head.querySelectorAll("style")]
    .flatMap((tag) => (tag.textContent ?? "").split("\n"))
    .filter((line) => line.includes("{"));
}

/** The rule that wins for `selector`, given that ties go to the last one. */
function winningRule(selector: string): string | undefined {
  return mountedRules().filter((rule) => rule.includes(selector + " {")).at(-1);
}

/**
 * Ctrl-F used to open CodeMirror's own panel, coloured from a light/dark fork
 * this editor never chose: on every dark theme its buttons came out pale grey
 * under near-white ink. (#652)
 *
 * The widget that replaced it is the kit's, and what it DOES is covered where
 * it lives, in `packages/ui-kit/src/searchPanel.test.ts`. What is only true
 * here is the wiring: that this editor is the thing that installs it, and that
 * the colours it hands over are the classic design's `--fl-*` set. Drop the
 * `search()` call and `openSearchPanel` quietly installs CodeMirror's own
 * configuration instead — the default panel, the unreadable buttons, and no
 * failure anywhere.
 */
describe("CodeEditor — find", () => {
  it("opens the kit's widget, not CodeMirror's panel", () => {
    const { container } = render(<CodeEditor value="a: 1" />);
    const view = EditorView.findFromDOM(container.querySelector(".cm-editor")!)!;
    openSearchPanel(view);
    expect(container.querySelector(".cm-sl-find")).not.toBeNull();
    expect(container.querySelector(".cm-panel.cm-search")).toBeNull();
    // The Find field has to carry this, or a second Ctrl-F does not land in it.
    expect(container.querySelector(".cm-sl-find [main-field]")).not.toBeNull();
  });

  it("floats the widget rather than shoving the document down", () => {
    render(<CodeEditor value="a: 1" />);
    expect(winningRule(".cm-panels.cm-panels-top:has(.cm-sl-find)")).toContain("height: 0");
    expect(winningRule(".cm-sl-find")).toContain("position: absolute");
  });

  it.each([".cm-sl-find", ".cm-sl-input", ".cm-panels"])("dresses %s from the classic tokens", (part) => {
    render(<CodeEditor value="a: 1" />);
    const rule = winningRule(part);
    expect(rule, `no rule for ${part}`).toBeDefined();
    // `var(--fl-`, not just `var(--`: the widget is shared with the new design,
    // and handing it that design's tokens here would leave every colour unset.
    expect(rule).toContain("background-color: var(--fl-");
  });

  it("still dresses the go-to-line dialog, which is CodeMirror's", () => {
    // Mod-Alt-g is in the same keymap and still opens a `.cm-textfield` and a
    // `.cm-button`; replacing the search panel did not replace those.
    render(<CodeEditor value="a: 1" />);
    expect(winningRule(".cm-button")).toContain("background-color: var(--fl-");
    expect(winningRule(".cm-textfield")).toContain("background-color: var(--fl-");
  });
});
