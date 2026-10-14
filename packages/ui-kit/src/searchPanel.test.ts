import { describe, it, expect, afterEach } from "vitest";
import { EditorState } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { SearchQuery, getSearchQuery, openSearchPanel, search, searchKeymap } from "@codemirror/search";
import { codeSearchPanel, matchCount, searchPanelStyles } from "./searchPanel";

const SAMPLE = "one two one\nthree one two\n";

let view: EditorView | null = null;

afterEach(() => {
  view?.destroy();
  view = null;
});

/** An editor wired the way `CodeEditor` wires one, with the panel already up. */
function openPanel(doc = SAMPLE, readOnly = false): HTMLElement {
  view = new EditorView({
    state: EditorState.create({
      doc,
      extensions: [
        EditorState.readOnly.of(readOnly),
        keymap.of(searchKeymap),
        search({ top: true, createPanel: codeSearchPanel }),
      ],
    }),
    parent: document.body,
  });
  openSearchPanel(view);
  const panel = view.dom.querySelector<HTMLElement>(".cm-sl-find");
  if (!panel) throw new Error("the find widget did not open");
  return panel;
}

function field(panel: HTMLElement, label: string): HTMLInputElement {
  const input = panel.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`);
  if (!input) throw new Error(`no ${label} field`);
  return input;
}

/** Type into a field the way a reader does: value, then the input event. */
function type(input: HTMLInputElement, text: string) {
  input.value = text;
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

function button(panel: HTMLElement, label: string): HTMLButtonElement {
  const found = panel.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
  if (!found) throw new Error(`no ${label} button`);
  return found;
}

describe("matchCount", () => {
  it("counts every match, and says which one the cursor is on", () => {
    const state = EditorState.create({ doc: "one two one two one", selection: { anchor: 8, head: 11 } });
    expect(matchCount(state, new SearchQuery({ search: "one" }))).toEqual({
      total: 3,
      current: 2,
      capped: false,
    });
  });

  it("reports no current match when the selection is not on one", () => {
    // What the reader sees on opening the panel: the count is known, the
    // position in it is not, so the widget says "3 results" and not "0 of 3".
    const state = EditorState.create({ doc: "one two one two one" });
    expect(matchCount(state, new SearchQuery({ search: "one" }))).toMatchObject({ total: 3, current: 0 });
  });

  it("counts nothing for a regexp that does not parse", () => {
    const state = EditorState.create({ doc: "a(b" });
    expect(matchCount(state, new SearchQuery({ search: "(", regexp: true }))).toEqual({
      total: 0,
      current: 0,
      capped: false,
    });
  });

  it("stops counting rather than scan a document for a number nobody reads", () => {
    const state = EditorState.create({ doc: "a".repeat(5000) });
    const counted = matchCount(state, new SearchQuery({ search: "a" }));
    expect(counted.capped).toBe(true);
    expect(counted.total).toBeLessThan(5000);
  });
});

/**
 * The widget that replaced CodeMirror's search panel. It exists because the
 * default one is coloured from a light/dark fork this editor never chose, and
 * came out unreadable on every dark theme (#652) — but what it has to be
 * judged on now is whether it finds things.
 */
describe("the find widget", () => {
  it("opens in place of CodeMirror's own panel", () => {
    const panel = openPanel();
    expect(panel).not.toBeNull();
    expect(view!.dom.querySelector(".cm-panel.cm-search")).toBeNull();
    // `openSearchPanel` focuses whatever carries this attribute when the panel
    // is already open, so Ctrl-F twice has to land back in the Find field.
    expect(field(panel, "Find").getAttribute("main-field")).toBe("true");
  });

  it("searches as the reader types, and counts what it found", () => {
    const panel = openPanel();
    type(field(panel, "Find"), "one");
    expect(getSearchQuery(view!.state).search).toBe("one");
    expect(panel.querySelector(".cm-sl-count")!.textContent).toBe("3 results");
  });

  it("says so, and stops offering to step, when there is nothing to find", () => {
    const panel = openPanel();
    type(field(panel, "Find"), "nowhere");
    expect(panel.querySelector(".cm-sl-count")!.textContent).toBe("No results");
    expect(panel.getAttribute("data-empty")).toBe("true");
    expect(button(panel, "Next match").disabled).toBe(true);
  });

  it("names a bad pattern rather than call it a miss", () => {
    // "No results" for an unparseable regexp reads as "not in this file",
    // which sends the reader looking for the text instead of the typo.
    const panel = openPanel();
    button(panel, "Use regular expression").click();
    type(field(panel, "Find"), "one(");
    expect(panel.querySelector(".cm-sl-count")!.textContent).toBe("Bad pattern");
  });

  it("carries each option into the query", () => {
    const panel = openPanel();
    type(field(panel, "Find"), "one");
    for (const [label, flag] of [
      ["Match case", "caseSensitive"],
      ["Match whole word", "wholeWord"],
      ["Use regular expression", "regexp"],
    ] as const) {
      const toggle = button(panel, label);
      toggle.click();
      expect(toggle.getAttribute("aria-pressed"), label).toBe("true");
      expect(getSearchQuery(view!.state)[flag], label).toBe(true);
    }
  });

  it("keeps the replace row folded away until it is asked for", () => {
    const panel = openPanel();
    const disclosure = button(panel, "Show replace");
    expect(panel.getAttribute("data-replace")).toBe("closed");
    disclosure.click();
    expect(panel.getAttribute("data-replace")).toBe("open");
    expect(disclosure.getAttribute("aria-expanded")).toBe("true");
  });

  it("offers no replace at all in a read-only editor", () => {
    const panel = openPanel(SAMPLE, true);
    expect(panel.querySelector(".cm-sl-row-replace")).toBeNull();
    expect(panel.querySelector(".cm-sl-expand")).toBeNull();
  });

  it("steps through the matches, and follows the cursor onto one", () => {
    const panel = openPanel();
    type(field(panel, "Find"), "one");
    button(panel, "Next match").click();
    expect(panel.querySelector(".cm-sl-count")!.textContent).toBe("1 of 3");
    button(panel, "Next match").click();
    expect(panel.querySelector(".cm-sl-count")!.textContent).toBe("2 of 3");
  });

  it("replaces the match it is on, and then all of them", () => {
    const panel = openPanel();
    type(field(panel, "Find"), "one");
    button(panel, "Show replace").click();
    type(field(panel, "Replace"), "ONE");
    const [replaceOne, replaceAll] = panel.querySelectorAll<HTMLButtonElement>(".cm-sl-btn");
    button(panel, "Next match").click();
    replaceOne.click();
    expect(view!.state.doc.toString()).toContain("ONE");
    replaceAll.click();
    expect(view!.state.doc.toString()).not.toContain("one");
  });
});

describe("searchPanelStyles", () => {
  it("names nothing but the tokens it was handed", () => {
    // The whole point of #652: the widget's colours come from the design that
    // mounts it, so a theme the kit has never heard of still gets a readable
    // find box. A literal colour here is the bug coming back.
    const rules = searchPanelStyles({
      surface: "var(--surface-raised)",
      fieldBg: "var(--surface-sunk)",
      ink: "var(--ink)",
      inkMuted: "var(--ink-muted)",
      rule: "var(--rule-strong)",
      hover: "var(--field)",
      accent: "var(--accent)",
      accentWash: "var(--accent-wash)",
      danger: "var(--sev)",
      font: "var(--font-sans)",
      shadow: "0 8px 24px var(--shade)",
      radius: "var(--radius-tile)",
    });
    const colourish = /^(color|background-?color|border-?color|box-shadow|outline)$/i;
    for (const [selector, declarations] of Object.entries(rules)) {
      for (const [property, value] of Object.entries(declarations)) {
        if (!colourish.test(property)) continue;
        if (value === "transparent" || value === "none") continue;
        expect(value, `${selector} { ${property} }`).toContain("var(--");
      }
    }
  });
});
