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
