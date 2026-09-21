import { describe, it, expect, vi } from "vitest";
import { render, waitFor } from "@testing-library/react";
import { CodeEditor } from "./CodeEditor";

describe("CodeEditor", () => {
  it("mounts a CodeMirror editor showing the initial value", () => {
    const { container } = render(<CodeEditor value="kind: Pod" ariaLabel="Manifest YAML" />);
    expect(container.querySelector(".cm-editor")).not.toBeNull();
    expect(container.querySelector(".cm-content")?.textContent).toContain("kind: Pod");
    // The label goes on the editable content, which is what a screen reader
    // lands on — not on the wrapper.
    expect(container.querySelector('[aria-label="Manifest YAML"]')).not.toBeNull();
  });

  it("does not call onChange while read-only", () => {
    const onChange = vi.fn();
    const { container } = render(<CodeEditor value="a: 1" readOnly onChange={onChange} />);
    expect(container.querySelector(".cm-editor")).not.toBeNull();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("pushes an external value change into the editor", () => {
    // Reset and reload replace the document from outside; the editor is
    // mounted imperatively, so this is the one direction that needs wiring.
    const { container, rerender } = render(<CodeEditor value="a: 1" />);
    rerender(<CodeEditor value="b: 2" />);
    expect(container.querySelector(".cm-content")?.textContent).toContain("b: 2");
  });

  it("takes completions as an injected source, knowing nothing of what they mean", () => {
    // The classic editor resolved Kubernetes schemas itself, importing four
    // helpers and a type from @srelens/core. The kit may not: `tokens-only`
    // forbids the service layer, and a design system has no business knowing
    // what an apiVersion is. The caller supplies a CodeMirror completion
    // source and keeps that knowledge. (#318)
    const completions = vi.fn(() => null);
    const { container } = render(<CodeEditor value="a: 1" completions={completions} />);
    expect(container.querySelector(".cm-editor")).not.toBeNull();
  });

  it("accepts the sizing options without recreating itself into a broken state", () => {
    // What `fill`, `minHeight` and `maxHeight` actually do is not assertable
    // here: CodeMirror compiles a theme into a generated stylesheet with
    // hashed class names rather than inline styles, and jsdom applies no CSS.
    // Said plainly rather than asserting on a generated class name, which
    // would pin CodeMirror's internals and still prove nothing about layout.
    const { container } = render(<CodeEditor value="a: 1" fill minHeight={100} maxHeight={400} />);
    expect(container.querySelector(".cm-editor")).not.toBeNull();
  });
  it("does not report a prop-driven value change as a user edit", () => {
    // Reset and reload replace the document from outside. The dispatch that
    // does it changes the doc, so an unconditional listener reports it as
    // typing — marking a form dirty, or invalidating a preview, on a change
    // the caller made itself. (#326 review)
    const onChange = vi.fn();
    const { rerender } = render(<CodeEditor value="a: 1" onChange={onChange} />);
    rerender(<CodeEditor value="b: 2" onChange={onChange} />);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("re-lints when the validator changes, without waiting for an edit", async () => {
    // Switching cluster context swaps the validator while the same document
    // stays mounted. Updating the ref alone schedules nothing, so the previous
    // validator's diagnostics sit there until someone types. (#326 review)
    const first = vi.fn(async () => []);
    const second = vi.fn(async () => []);
    const { rerender } = render(<CodeEditor value="a: 1" schemaValidate={first} />);
    // Let the document settle on the first validator. Without this the swap
    // happens before the debounced first lint ever runs, and the test passes
    // whether or not anything re-lints.
    await vi.waitFor(() => expect(first).toHaveBeenCalled(), { timeout: 3000 });

    rerender(<CodeEditor value="a: 1" schemaValidate={second} />);
    await vi.waitFor(() => expect(second).toHaveBeenCalled(), { timeout: 3000 });
  });
});

describe("CodeEditor — what it tells the caller", () => {
  it("reports the cursor on mount, and again when the document is replaced under it", () => {
    // A sidebar that says what is valid at the cursor needs the position; the
    // editor is mounted imperatively, so this is the one way it gets out.
    const onCursorChange = vi.fn();
    const { rerender } = render(<CodeEditor value="a: 1" onCursorChange={onCursorChange} />);
    expect(onCursorChange).toHaveBeenCalledWith(0);
    rerender(<CodeEditor value="b: 22" onCursorChange={onCursorChange} />);
    expect(onCursorChange).toHaveBeenCalledTimes(2);
  });

  it("reports each lint pass's findings, with the line each is on", async () => {
    // The gutter shows a marker; a list beside the editor has to say "line 2"
    // and what is wrong there. A duplicate key is a syntax-level error the
    // yaml package places at the second key.
    const onDiagnostics = vi.fn();
    render(<CodeEditor value={"a: 1\na: 2\n"} onDiagnostics={onDiagnostics} />);
    await waitFor(() => expect(onDiagnostics).toHaveBeenCalled(), { timeout: 3000 });
    const last = onDiagnostics.mock.calls.at(-1)![0] as Array<{ line: number; severity: string; message: string }>;
    expect(last.length).toBeGreaterThan(0);
    expect(last[0].line).toBe(2);
    expect(last[0].severity).toBe("error");
  });

  it("reports an empty pass too, so the caller can say the document is clean", async () => {
    const onDiagnostics = vi.fn();
    render(<CodeEditor value={"a: 1\n"} onDiagnostics={onDiagnostics} />);
    await waitFor(() => expect(onDiagnostics).toHaveBeenCalledWith([]), { timeout: 3000 });
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

  it("leaves an editable document to CodeMirror's own tab stop", () => {
    const { container } = render(<CodeEditor value="kind: Pod" />);
    expect(container.querySelector(".cm-content")?.hasAttribute("tabindex")).toBe(false);
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
    expect(container.querySelector(".cm-selectionBackground, .cm-selectionLayer")).not.toBeNull();
  });

  it("draws a focus indicator on the read-only pane it made reachable", () => {
    // `tabindex="0"` puts the pane in the tab order; `kit.css` clears the
    // outline from every focused `div`, and this content is one — so without
    // a rule of its own a keyboard reader arriving here is given nothing at
    // all to say where they are. Read off the stylesheet CodeMirror actually
    // injected rather than off the source: jsdom applies no CSS and resolves
    // no `:focus-visible`, so the rule's presence is what is observable.
    // (#656 review)
    render(<CodeEditor value="kind: Pod" readOnly ariaLabel="web manifest" />);
    const sheets = [...document.querySelectorAll("style")].map((s) => s.textContent ?? "").join("");
    const at = sheets.indexOf(".cm-content[tabindex]:focus-visible");
    expect(at, "no focus indicator for a focusable read-only pane").toBeGreaterThan(-1);
    expect(sheets.slice(at, sheets.indexOf("}", at))).toContain("outline");
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
});
