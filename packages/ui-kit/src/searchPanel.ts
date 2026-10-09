import { EditorView, runScopeHandlers, type Panel, type ViewUpdate } from "@codemirror/view";
import type { EditorState } from "@codemirror/state";
import {
  SearchQuery,
  closeSearchPanel,
  findNext,
  findPrevious,
  getSearchQuery,
  replaceAll,
  replaceNext,
  selectMatches,
  setSearchQuery,
} from "@codemirror/search";

/**
 * A find-and-replace widget, in place of CodeMirror's own search panel.
 *
 * CodeMirror's panel is a full-width bar of native checkboxes and text
 * buttons, coloured from ITS base theme — which forks on `EditorView.darkTheme`
 * and so came out light under every one of the app's themes: pale grey buttons
 * under near-white ink, unreadable on a dark ground. (#652)
 *
 * Rather than re-dress that bar, this replaces it, the way the CodeMirror
 * maintainers suggest replacing it: a `createPanel` of our own, modelled on the
 * structure of theirs. What that buys beyond the colours is the shape readers
 * already know from VS Code — a compact card floating at the top right instead
 * of a bar that shoves the document down the moment Ctrl-F is pressed, the
 * options as toggles inside the field instead of three captioned checkboxes, a
 * live match count, and a replace row that stays folded away until asked for.
 *
 * Structure and behaviour live here; the colours do not. Every value comes
 * from `SearchPanelTokens`, which each editor fills from its own design — the
 * kit's `--ink`/`--surface` set and the classic design's `--fl-*` set name the
 * same things differently, and neither belongs in here.
 */

const SVG_NS = "http://www.w3.org/2000/svg";

/**
 * Stop counting matches here.
 *
 * A count is something a reader glances at; past a few hundred it has stopped
 * being information, and a full scan of a large document on every keystroke is
 * real work spent producing a number nobody reads.
 */
const MAX_COUNT = 999;

/** Icon paths, on a 16x16 grid, stroked in `currentColor`. */
const PATH = {
  chevronRight: ["M6.5 4 L10.5 8 L6.5 12"],
  chevronDown: ["M4 6.5 L8 10.5 L12 6.5"],
  arrowUp: ["M8 12.5 V4", "M4.5 7.5 L8 4 L11.5 7.5"],
  arrowDown: ["M8 3.5 V12", "M4.5 8.5 L8 12 L11.5 8.5"],
  close: ["M4.5 4.5 L11.5 11.5", "M11.5 4.5 L4.5 11.5"],
  selectAll: ["M3 4.5 H13", "M3 8 H13", "M3 11.5 H13"],
};

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, string> = {},
  children: (Node | string)[] = [],
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attrs)) node.setAttribute(name, value);
  node.append(...children);
  return node;
}

function icon(paths: string[]): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("width", "14");
  svg.setAttribute("height", "14");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "1.4");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  svg.setAttribute("aria-hidden", "true");
  for (const d of paths) {
    const path = document.createElementNS(SVG_NS, "path");
    path.setAttribute("d", d);
    svg.append(path);
  }
  return svg;
}

/**
 * How many matches the document holds, and which one the cursor is on.
 *
 * `current` is 0 when the selection is not sitting on a match — on opening the
 * panel, or after an edit moved the text under it — which is the difference
 * between "3 of 12" and "12 results". Pure + tested.
 */
export function matchCount(
  state: EditorState,
  query: SearchQuery,
): { total: number; current: number; capped: boolean } {
  if (!query.valid) return { total: 0, current: 0, capped: false };
  const sel = state.selection.main;
  const cursor = query.getCursor(state);
  let total = 0;
  let current = 0;
  for (;;) {
    const step = cursor.next();
    if (step.done) return { total, current, capped: false };
    total++;
    if (step.value.from === sel.from && step.value.to === sel.to) current = total;
    if (total >= MAX_COUNT) return { total, current, capped: true };
  }
}

class CodeSearchPanel implements Panel {
  readonly dom: HTMLElement;
  readonly top = true;

  private readonly view: EditorView;
  private query: SearchQuery;
  private readonly findField: HTMLInputElement;
  private readonly replaceField: HTMLInputElement;
  private readonly caseToggle: HTMLButtonElement;
  private readonly wordToggle: HTMLButtonElement;
  private readonly regexpToggle: HTMLButtonElement;
  private readonly count: HTMLElement;
  private readonly expand: HTMLButtonElement | null;
  /** Everything that only makes sense when there is a match to act on. */
  private readonly steppers: HTMLButtonElement[];

  constructor(view: EditorView) {
    this.view = view;
    this.query = getSearchQuery(view.state);
    const phrase = (text: string) => view.state.phrase(text);

    this.findField = el("input", {
      class: "cm-sl-field",
      // What `openSearchPanel` focuses when the panel is already open.
      "main-field": "true",
      placeholder: phrase("Find"),
      "aria-label": phrase("Find"),
      // The panel lives inside the editor's DOM; without this the field joins
      // whatever form the editor is mounted in, and Enter submits it.
      form: "",
    });
    this.replaceField = el("input", {
      class: "cm-sl-field",
      placeholder: phrase("Replace"),
      "aria-label": phrase("Replace"),
      form: "",
    });

    // The three glyphs VS Code uses, which readers already read as these three
    // options: "Aa" for case, "ab" for whole word, ".*" for regular expression.
    const toggle = (glyph: string, title: string) =>
      el(
        "button",
        { class: "cm-sl-toggle", type: "button", "aria-pressed": "false", title, "aria-label": title },
        [glyph],
      );
    this.caseToggle = toggle("Aa", phrase("Match case"));
    this.wordToggle = toggle("ab", phrase("Match whole word"));
    this.regexpToggle = toggle(".*", phrase("Use regular expression"));
    const options = [this.caseToggle, this.wordToggle, this.regexpToggle];

    const iconButton = (paths: string[], title: string, onClick: () => void) => {
      const button = el("button", { class: "cm-sl-icon", type: "button", title, "aria-label": title }, [icon(paths)]);
      button.addEventListener("click", onClick);
      return button;
    };
    const previous = iconButton(PATH.arrowUp, phrase("Previous match"), () => findPrevious(view));
    const next = iconButton(PATH.arrowDown, phrase("Next match"), () => findNext(view));
    const all = iconButton(PATH.selectAll, phrase("Select all matches"), () => selectMatches(view));
    const close = iconButton(PATH.close, phrase("Close"), () => closeSearchPanel(view));
    this.steppers = [previous, next, all];

    this.count = el("output", { class: "cm-sl-count", "aria-live": "polite" });

    const rows: Node[] = [
      el("div", { class: "cm-sl-row" }, [
        el("div", { class: "cm-sl-input cm-sl-input-find" }, [
          this.findField,
          el("div", { class: "cm-sl-toggles" }, options),
        ]),
        this.count,
        previous,
        next,
        all,
        close,
      ]),
    ];

    // A read-only editor has nothing to replace, so it gets neither the row
    // nor the disclosure that opens it.
    if (view.state.readOnly) {
      this.expand = null;
    } else {
      const replaceOne = el("button", { class: "cm-sl-btn", type: "button" }, [phrase("Replace")]);
      replaceOne.addEventListener("click", () => replaceNext(view));
      const replaceEvery = el("button", { class: "cm-sl-btn", type: "button" }, [phrase("All")]);
      replaceEvery.addEventListener("click", () => replaceAll(view));
      this.steppers.push(replaceOne, replaceEvery);
      rows.push(
        el("div", { class: "cm-sl-row cm-sl-row-replace" }, [
          el("div", { class: "cm-sl-input" }, [this.replaceField]),
          replaceOne,
          replaceEvery,
        ]),
      );
      this.expand = el("button", { class: "cm-sl-expand", type: "button", "aria-expanded": "false" }, [
        icon(PATH.chevronRight),
      ]);
      this.expand.addEventListener("click", () => this.setExpanded(!this.isExpanded()));
    }

    this.dom = el(
      "div",
      { class: "cm-sl-find", role: "search", "aria-label": phrase("Find and replace"), "data-replace": "closed" },
      [...(this.expand ? [this.expand] : []), el("div", { class: "cm-sl-body" }, rows)],
    );

    this.dom.addEventListener("keydown", (e) => this.keydown(e));
    // Search as you type, the way every find widget the reader has used does.
    this.findField.addEventListener("input", () => this.commit());
    this.replaceField.addEventListener("input", () => this.commit());
    for (const option of options) {
      option.addEventListener("click", () => {
        option.setAttribute("aria-pressed", option.getAttribute("aria-pressed") === "true" ? "false" : "true");
        this.commit();
        // The reader is mid-search; the toggle is a detour, not a destination.
        this.findField.focus();
      });
    }

    this.setQuery(this.query);
    // Unconditional, because this is also where the disclosure gets its label:
    // a query carried over from the last time the panel was open may already
    // have a replacement in it, and folding the row away would hide what is
    // about to happen.
    this.setExpanded(Boolean(this.query.replace));
  }

  mount() {
    this.findField.select();
  }

  update(update: ViewUpdate) {
    for (const tr of update.transactions) {
      for (const effect of tr.effects) {
        if (effect.is(setSearchQuery) && !effect.value.eq(this.query)) this.setQuery(effect.value);
      }
    }
    // The count answers two questions — how many there are, and which one you
    // are on — so a plain cursor move changes it as much as an edit does.
    if (update.docChanged || update.selectionSet) this.renderCount();
  }

  /**
   * Keys the widget answers itself, and keys it hands back to the editor.
   *
   * Escape, F3 and Mod-g belong to the search keymap under the `search-panel`
   * scope; running them through the scope handler is how they keep working
   * while focus is in a field rather than in the document.
   */
  private keydown(e: KeyboardEvent) {
    if (runScopeHandlers(this.view, e, "search-panel")) {
      e.preventDefault();
      return;
    }
    if (e.key !== "Enter") return;
    if (e.target === this.findField) {
      e.preventDefault();
      (e.shiftKey ? findPrevious : findNext)(this.view);
    } else if (e.target === this.replaceField) {
      e.preventDefault();
      // Mod-Enter replaces every match, the way Ctrl-Alt-Enter does in the
      // editor readers are coming from.
      (e.ctrlKey || e.metaKey ? replaceAll : replaceNext)(this.view);
    }
  }

  private isExpanded(): boolean {
    return this.dom.getAttribute("data-replace") === "open";
  }

  private setExpanded(open: boolean) {
    if (!this.expand) return;
    const title = this.view.state.phrase(open ? "Hide replace" : "Show replace");
    this.expand.setAttribute("aria-expanded", String(open));
    this.expand.setAttribute("title", title);
    this.expand.setAttribute("aria-label", title);
    this.expand.replaceChildren(icon(open ? PATH.chevronDown : PATH.chevronRight));
    this.dom.setAttribute("data-replace", open ? "open" : "closed");
  }

  /** Read the controls, and tell the editor if anything moved. */
  private commit() {
    const query = new SearchQuery({
      search: this.findField.value,
      caseSensitive: this.caseToggle.getAttribute("aria-pressed") === "true",
      wholeWord: this.wordToggle.getAttribute("aria-pressed") === "true",
      regexp: this.regexpToggle.getAttribute("aria-pressed") === "true",
      replace: this.replaceField.value,
    });
    if (!query.eq(this.query)) {
      this.query = query;
      this.view.dispatch({ effects: setSearchQuery.of(query) });
    }
    this.renderCount();
  }

  /** Put a query the editor already holds onto the controls. */
  private setQuery(query: SearchQuery) {
    this.query = query;
    this.findField.value = query.search;
    this.replaceField.value = query.replace;
    this.caseToggle.setAttribute("aria-pressed", String(query.caseSensitive));
    this.wordToggle.setAttribute("aria-pressed", String(query.wholeWord));
    this.regexpToggle.setAttribute("aria-pressed", String(query.regexp));
    this.renderCount();
  }

  private renderCount() {
    const phrase = (text: string) => this.view.state.phrase(text);
    const say = (text: string, empty: boolean, stepping: boolean) => {
      this.count.textContent = text;
      if (empty) this.dom.setAttribute("data-empty", "true");
      else this.dom.removeAttribute("data-empty");
      for (const button of this.steppers) button.disabled = !stepping;
    };
    if (!this.query.search) return say("", false, false);
    // The only way a non-empty query is invalid is a regexp that does not
    // parse, and saying so beats a bare "no results", which a reader takes to
    // mean the text is not in the file.
    if (!this.query.valid) return say(phrase("Bad pattern"), true, false);
    const { total, current, capped } = matchCount(this.view.state, this.query);
    if (!total) return say(phrase("No results"), true, false);
    const of = capped ? `${MAX_COUNT}+` : String(total);
    say(current ? `${current} ${phrase("of")} ${of}` : `${of} ${phrase("results")}`, false, true);
  }
}

/**
 * The panel constructor to hand to `search({ createPanel })`.
 *
 * `search()` has to be among the editor's extensions for this to be used at
 * all: `openSearchPanel` installs CodeMirror's own configuration when it finds
 * none, and that configuration brings the default panel with it.
 */
export function codeSearchPanel(view: EditorView): Panel {
  return new CodeSearchPanel(view);
}

/** The colours the widget needs, named by role rather than by design. */
export interface SearchPanelTokens {
  /** The widget's own ground — it floats above the document. */
  surface: string;
  /** The ground of a text field, one step back from `surface`. */
  fieldBg: string;
  /** Text the reader is meant to read. */
  ink: string;
  /** Placeholders, counts, and an icon at rest. */
  inkMuted: string;
  /** Hairlines: the widget's edge, a field's edge. */
  rule: string;
  /** The wash under a hovered control. */
  hover: string;
  /** Focus rings, and an option that is on. */
  accent: string;
  /** The ground under an option that is on. */
  accentWash: string;
  /** No results, and a regexp that does not parse. */
  danger: string;
  /** The UI font stack — the widget is chrome, not code. */
  font: string;
  /** What lifts the widget off the document. */
  shadow: string;
  /** The widget's corner radius. */
  radius: string;
}

/**
 * The widget's appearance, for spreading into an `EditorView.theme` spec.
 *
 * Also re-dresses the panel container CodeMirror wraps around it, which by
 * default is an opaque full-width bar with a hairline of its own.
 */
export function searchPanelStyles(t: SearchPanelTokens): Record<string, Record<string, string>> {
  return {
    // A bar at the top pushes the document down the moment Ctrl-F is pressed,
    // which moves the line the reader was looking at. Collapsing the container
    // to nothing and floating the widget inside it leaves the text where it is.
    // Guarded by `:has` so a future panel that IS a bar still gets to be one.
    ".cm-panels.cm-panels-top:has(.cm-sl-find)": {
      height: "0",
      border: "none",
      backgroundColor: "transparent",
      overflow: "visible",
    },
    ".cm-sl-find": {
      position: "absolute",
      top: "4px",
      // Clear of the vertical scrollbar the document scrolls under.
      right: "16px",
      maxWidth: "calc(100% - 32px)",
      boxSizing: "border-box",
      display: "flex",
      alignItems: "stretch",
      gap: "2px",
      padding: "4px",
      backgroundColor: t.surface,
      border: "1px solid " + t.rule,
      borderRadius: t.radius,
      boxShadow: t.shadow,
      color: t.ink,
      fontFamily: t.font,
      fontSize: "12px",
      lineHeight: "1",
    },
    ".cm-sl-body": { display: "flex", flexDirection: "column", gap: "4px", minWidth: "0" },
    ".cm-sl-row": { display: "flex", alignItems: "center", gap: "3px", minWidth: "0" },
    '.cm-sl-find[data-replace="closed"] .cm-sl-row-replace': { display: "none" },
    ".cm-sl-input": {
      display: "flex",
      alignItems: "center",
      gap: "2px",
      // One width for both fields rather than "whatever is left in the row":
      // the find row carries a count and four buttons and the replace row two,
      // so a field that fills its row leaves the two ragged against each other.
      flex: "0 1 auto",
      boxSizing: "border-box",
      width: "236px",
      minWidth: "0",
      height: "24px",
      padding: "0 2px 0 6px",
      backgroundColor: t.fieldBg,
      border: "1px solid " + t.rule,
      borderRadius: "4px",
    },
    ".cm-sl-input:focus-within": { borderColor: t.accent, boxShadow: "0 0 0 1px " + t.accent },
    '.cm-sl-find[data-empty="true"] .cm-sl-input-find': { borderColor: t.danger },
    ".cm-sl-field": {
      flex: "1 1 auto",
      width: "auto",
      minWidth: "0",
      padding: "0",
      border: "none",
      outline: "none",
      backgroundColor: "transparent",
      color: t.ink,
      fontFamily: "inherit",
      fontSize: "12px",
    },
    ".cm-sl-field::placeholder": { color: t.inkMuted },
    ".cm-sl-toggles": { display: "flex", alignItems: "center", gap: "1px", flex: "none" },
    ".cm-sl-toggle": {
      display: "flex",
      alignItems: "center",
      justifyContent: "center",
      flex: "none",
      width: "20px",
      height: "20px",
      padding: "0",
      border: "1px solid transparent",
      borderRadius: "4px",
      backgroundColor: "transparent",
      color: t.inkMuted,
      fontFamily: "inherit",
      fontSize: "11px",
      fontWeight: "600",
      lineHeight: "1",
      cursor: "pointer",
    },
    ".cm-sl-toggle:hover": { backgroundColor: t.hover, color: t.ink },
    '.cm-sl-toggle[aria-pressed="true"]': {
      backgroundColor: t.accentWash,
      borderColor: t.accent,
      color: t.accent,
    },
    ".cm-sl-icon, .cm-sl-expand": {
      display: "flex",
      alignItems: "center",
      justifyContent: "center",
      flex: "none",
      width: "24px",
      padding: "0",
      border: "none",
      borderRadius: "4px",
      backgroundColor: "transparent",
      color: t.inkMuted,
      cursor: "pointer",
    },
    ".cm-sl-icon": { height: "24px" },
    ".cm-sl-icon:hover, .cm-sl-expand:hover": { backgroundColor: t.hover, color: t.ink },
    ".cm-sl-icon:disabled, .cm-sl-icon:disabled:hover": {
      opacity: "0.35",
      cursor: "default",
      backgroundColor: "transparent",
      color: t.inkMuted,
    },
    ".cm-sl-count": {
      flex: "none",
      minWidth: "64px",
      padding: "0 4px",
      textAlign: "right",
      color: t.inkMuted,
      fontSize: "11px",
      fontVariantNumeric: "tabular-nums",
      whiteSpace: "nowrap",
    },
    '.cm-sl-find[data-empty="true"] .cm-sl-count': { color: t.danger },
    ".cm-sl-btn": {
      flex: "none",
      height: "24px",
      padding: "0 9px",
      border: "1px solid " + t.rule,
      borderRadius: "4px",
      backgroundColor: "transparent",
      color: t.ink,
      fontFamily: "inherit",
      fontSize: "11px",
      fontWeight: "500",
      whiteSpace: "nowrap",
      cursor: "pointer",
    },
    ".cm-sl-btn:hover": { backgroundColor: t.hover },
    ".cm-sl-btn:disabled, .cm-sl-btn:disabled:hover": {
      opacity: "0.35",
      cursor: "default",
      backgroundColor: "transparent",
    },
    ".cm-sl-find button:focus-visible": { outline: "1px solid " + t.accent, outlineOffset: "1px" },
  };
}
