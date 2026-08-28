import { useEffect, useRef } from "react";
import { Annotation, EditorState, StateEffect } from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  drawSelection,
  dropCursor,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import {
  syntaxHighlighting,
  HighlightStyle,
  indentOnInput,
  bracketMatching,
  foldGutter,
  foldKeymap,
} from "@codemirror/language";
import { highlightSelectionMatches, searchKeymap } from "@codemirror/search";
import { yaml } from "@codemirror/lang-yaml";
import { linter, lintGutter, type Diagnostic } from "@codemirror/lint";
import { autocompletion, completionKeymap, type CompletionSource } from "@codemirror/autocomplete";
import { parseAllDocuments } from "yaml";
import { tags as t } from "@lezer/highlight";

/**
 * Parse YAML (one or more `---`-separated documents) and return syntax
 * errors/warnings across ALL documents as diagnostics. The `yaml` package
 * reports absolute offsets into the full source, so ranges from later
 * documents still land correctly without any per-document adjustment.
 * Pure + tested.
 */
export function yamlDiagnostics(text: string): Diagnostic[] {
  const len = text.length;
  if (!text.trim()) return [];
  const clamp = (n: number) => Math.max(0, Math.min(n, len));
  try {
    const docs = parseAllDocuments(text, { prettyErrors: false });
    const asDiag = (issue: { pos?: [number, number, number?]; message: string }, severity: "error" | "warning"): Diagnostic => {
      const [from, to] = issue.pos ?? [0, 1];
      return { from: clamp(from), to: Math.max(clamp(to), clamp(from) + 1), severity, message: issue.message };
    };
    const diagnostics: Diagnostic[] = [];
    for (const doc of docs) {
      diagnostics.push(...doc.errors.map((e) => asDiag(e, "error")));
      diagnostics.push(...doc.warnings.map((w) => asDiag(w, "warning")));
    }
    return diagnostics;
  } catch (e) {
    return [{ from: 0, to: len, severity: "error", message: String(e) }];
  }
}

/** Split a dotted field path ("spec.template.spec.containers[0].image") into
 *  getIn segments (["spec","template",…,"containers",0,"image"]). */
function fieldSegments(path: string): (string | number)[] {
  const segs: (string | number)[] = [];
  for (const part of path.split(".")) {
    const name = part.replace(/\[\d+\]/g, "");
    if (name) segs.push(name);
    for (const m of part.matchAll(/\[(\d+)\]/g)) segs.push(Number(m[1]));
  }
  return segs;
}

/** Field paths mentioned in a validation message (unknown field / invalid value). */
function extractFieldPaths(message: string): string[] {
  const paths = new Set<string>();
  for (const m of message.matchAll(/unknown field "([^"]+)"/g)) paths.add(m[1]);
  for (const m of message.matchAll(
    /\b([a-zA-Z_][\w-]*(?:\.[\w-]+|\[\d+\])*)\s*:\s*(?:Invalid value|Required value|Unsupported value|Forbidden|Duplicate value|Too long)/g,
  )) {
    paths.add(m[1]);
  }
  return [...paths];
}

/**
 * Marks a transaction as this component syncing the document to `value`,
 * rather than the user typing. Both change the document, and only one of them
 * is an edit. (#326 review)
 */
const sync = Annotation.define<boolean>();

/**
 * Asks the linter to look again at a document that has not changed.
 *
 * `forceLinting` will not do it: it acts only when a run is already pending,
 * so once the initial lint has settled it is a no-op. `needsRefresh` is the
 * hook meant for this — the linter re-runs when a transaction carries this.
 * (#326 review)
 */
const revalidate = StateEffect.define<null>();

/**
 * Map document-indexed validation messages onto editor ranges. Positions each
 * message at the offending field when it can be located in the YAML, else at
 * the top of the document — so the error is always shown. Pure + tested.
 *
 * Called `k8sDiagnostics` in the classic component, which named the caller
 * rather than the work: whoever produced the messages, this locates a field
 * path inside a multi-document YAML file. The kit does not carry the product's
 * vocabulary — the same call made for NavIcon's resource map. (#318)
 */
export function documentDiagnostics(text: string, errors: Array<{ docIndex: number; message: string }>): Diagnostic[] {
  if (!errors.length || !text.trim()) return [];
  const len = text.length;
  const clamp = (n: number) => Math.max(0, Math.min(n, len));
  let docs: ReturnType<typeof parseAllDocuments> = [];
  try {
    // Match the backend's split (which skips empty documents) so docIndex aligns.
    // Cast: `.filter()` widens the yaml package's generic Document union in a
    // way `docs`'s ReturnType<typeof parseAllDocuments> annotation doesn't
    // structurally match, even though every filtered element is still one of
    // the same Document instances `parseAllDocuments` produced.
    docs = parseAllDocuments(text).filter((d) => d.contents != null) as typeof docs;
  } catch {
    docs = [];
  }
  const diagnostics: Diagnostic[] = [];
  for (const { docIndex, message } of errors) {
    const doc = docs[docIndex];
    const ranges = doc
      ? extractFieldPaths(message)
          .map((p) => {
            const node = doc.getIn(fieldSegments(p), true) as { range?: [number, number] } | undefined;
            return node?.range ? ([node.range[0], node.range[1]] as [number, number]) : null;
          })
          .filter((r): r is [number, number] => !!r)
      : [];
    if (ranges.length) {
      for (const [from, to] of ranges) {
        diagnostics.push({ from: clamp(from), to: Math.max(clamp(to), clamp(from) + 1), severity: "error", message });
      }
    } else {
      // Fall back to the START of the target document (not the whole-text top).
      const docStart = (doc?.contents as { range?: [number, number] } | undefined)?.range?.[0] ?? 0;
      const nl = text.indexOf("\n", docStart);
      const to = nl === -1 ? len : nl;
      diagnostics.push({ from: clamp(docStart), to: Math.max(clamp(to), clamp(docStart) + 1), severity: "error", message });
    }
  }
  return diagnostics;
}

/**
 * Syntax colours, sourced from CSS tokens so the editor tracks the app theme
 * (light/dark) automatically. Keys are on-brand teal, the most scannable cue in
 * a manifest.
 */
const highlightStyle = HighlightStyle.define([
  { tag: [t.definition(t.propertyName), t.propertyName, t.labelName], color: "var(--accent)" },
  { tag: [t.string, t.special(t.string)], color: "var(--ink-soft)" },
  { tag: [t.number, t.integer, t.float], color: "var(--info)" },
  { tag: [t.bool, t.null, t.keyword, t.atom], color: "var(--info)" },
  { tag: [t.comment, t.lineComment, t.blockComment], color: "var(--ink-faint)", fontStyle: "italic" },
  { tag: [t.meta, t.punctuation, t.separator], color: "var(--ink-muted)" },
]);

/** Editor chrome, themed from CSS tokens (so light/dark just works). */
function editorTheme(minHeight: number, maxHeight: number, fill: boolean) {
  return EditorView.theme({
    "&": {
      color: "var(--ink)",
      backgroundColor: "var(--surface)",
      fontSize: "12px",
      border: "1px solid var(--rule)",
      borderRadius: "var(--radius-tile)",
      // `fill` makes the editor take its container's full height (scrolling
      // internally); otherwise it grows with content up to `maxHeight`.
      ...(fill ? { height: "100%" } : { maxHeight: `${maxHeight}px` }),
    },
    "&.cm-focused": { outline: "none", borderColor: "var(--accent)" },
    ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.55", overflow: "auto" },
    ".cm-content": { minHeight: fill ? "0" : `${minHeight}px`, caretColor: "var(--accent)" },
    ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent)" },
    ".cm-gutters": {
      backgroundColor: "var(--surface-sunk)",
      color: "var(--ink-muted)",
      border: "none",
      borderRight: "1px solid var(--rule)",
    },
    ".cm-activeLineGutter": { backgroundColor: "var(--field)", color: "var(--ink)" },
    ".cm-activeLine": { backgroundColor: "var(--field)" },
    ".cm-foldPlaceholder": {
      backgroundColor: "var(--field)",
      border: "none",
      color: "var(--ink-muted)",
    },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": {
      backgroundColor: "color-mix(in srgb, var(--accent) 25%, transparent)",
    },
    ".cm-selectionMatch": { backgroundColor: "color-mix(in srgb, var(--accent) 18%, transparent)" },
    ".cm-matchingBracket, &.cm-focused .cm-matchingBracket": {
      backgroundColor: "color-mix(in srgb, var(--accent) 22%, transparent)",
      outline: "1px solid var(--accent)",
    },
    ".cm-panels": { backgroundColor: "var(--surface-sunk)", color: "var(--ink)" },
    ".cm-searchMatch": { backgroundColor: "color-mix(in srgb, var(--warn) 30%, transparent)" },
    ".cm-tooltip": { maxWidth: "480px" },
    ".cm-tooltip.cm-tooltip-lint": {
      backgroundColor: "var(--surface-sunk)",
      border: "1px solid var(--rule)",
      borderRadius: "var(--radius-tile)",
      boxShadow: "0 4px 16px color-mix(in srgb, var(--canvas-deep) 60%, transparent)",
      color: "var(--ink)",
      maxWidth: "480px",
    },
    ".cm-diagnostic": {
      padding: "6px 10px",
      whiteSpace: "normal",
      maxHeight: "240px",
      overflowY: "auto",
      fontSize: "12px",
      lineHeight: "1.45",
    },
    ".cm-diagnostic-error": { borderLeft: "3px solid var(--sev)" },
    ".cm-tooltip.cm-tooltip-autocomplete": {
      backgroundColor: "var(--surface-sunk)",
      border: "1px solid var(--rule)",
      borderRadius: "var(--radius-tile)",
      boxShadow: "0 4px 16px color-mix(in srgb, var(--canvas-deep) 60%, transparent)",
    },
    ".cm-tooltip-autocomplete > ul > li": {
      padding: "2px 8px",
      fontFamily: "var(--font-mono)",
      fontSize: "12px",
      color: "var(--ink)",
    },
    ".cm-tooltip-autocomplete > ul > li[aria-selected]": {
      backgroundColor: "var(--accent)",
      color: "var(--accent-ink)",
    },
    ".cm-completionDetail": { color: "var(--ink-muted)", fontStyle: "italic", marginLeft: "6px" },
    ".cm-lint-marker": { width: "0.9em", height: "0.9em" },
  });
}

export interface CodeEditorProps {
  value: string;
  onChange?: (value: string) => void;
  /** YAML language + syntax highlighting (default true). */
  language?: "yaml" | "none";
  readOnly?: boolean;
  ariaLabel?: string;
  minHeight?: number;
  maxHeight?: number;
  /** Fill the parent's height (scroll internally) instead of growing to content. */
  fill?: boolean;
  /**
   * Validation the caller performs, given the YAML, resolving to messages
   * against a document index (empty = valid). When set, the editor lints with
   * it in addition to YAML syntax — syntax first and locally, so a document
   * that does not parse never costs a round trip.
   *
   * Structural on purpose: what validates, and where, is the caller's. In this
   * app that is a server-side dry-run against the API server.
   */
  schemaValidate?: (yaml: string) => Promise<Array<{ docIndex: number; message: string }>>;
  /**
   * Autocomplete, supplied by the caller.
   *
   * The classic editor resolved Kubernetes schemas itself. The kit cannot: it
   * may not reach the service layer, and a design system has no business
   * knowing what an apiVersion is. This is CodeMirror's own completion source
   * type, so the caller keeps that knowledge and hands over only the result.
   * (#318)
   */
  completions?: CompletionSource;
}

/**
 * A real code editor (CodeMirror 6): line numbers, YAML syntax highlighting,
 * fold gutter, bracket matching, undo/redo, and find (Cmd/Ctrl-F). Mounted
 * imperatively and kept in sync with `value`; `onChange` fires on user edits.
 */
export function CodeEditor({
  value,
  onChange,
  language = "yaml",
  readOnly = false,
  ariaLabel,
  minHeight = 320,
  maxHeight = 520,
  fill = false,
  schemaValidate,
  completions,
}: CodeEditorProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  // Keep the latest onChange/validate without re-creating the editor on every render.
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const validateRef = useRef(schemaValidate);
  validateRef.current = schemaValidate;
  const completionsRef = useRef(completions);
  completionsRef.current = completions;

  useEffect(() => {
    const parent = parentRef.current;
    if (!parent) return;

    const extensions = [
      lineNumbers(),
      highlightActiveLineGutter(),
      highlightActiveLine(),
      foldGutter(),
      history(),
      drawSelection(),
      dropCursor(),
      indentOnInput(),
      bracketMatching(),
      highlightSelectionMatches(),
      keymap.of([...completionKeymap, ...defaultKeymap, ...historyKeymap, ...searchKeymap, ...foldKeymap, indentWithTab]),
      editorTheme(minHeight, maxHeight, fill),
      syntaxHighlighting(highlightStyle),
      EditorView.editable.of(!readOnly),
      EditorState.readOnly.of(readOnly),
      EditorView.updateListener.of((u) => {
        if (!u.docChanged) return;
        // A reset or a reload replaces the document from outside. That is the
        // caller telling us, not the user typing, and reporting it back as an
        // edit marks their own form dirty. (#326 review)
        if (u.transactions.some((t) => t.annotation(sync))) return;
        onChangeRef.current?.(u.state.doc.toString());
      }),
    ];
    if (language === "yaml") {
      // Lint YAML syntax first (local, instant); if it parses, validate against
      // the caller's validator (debounced via the linter delay) for deeper errors.
      const yamlLinter = linter(
        async (view) => {
          const text = view.state.doc.toString();
          const syntax = yamlDiagnostics(text);
          if (syntax.length) return syntax;
          const validate = validateRef.current;
          if (!validate || !text.trim()) return [];
          try {
            return documentDiagnostics(text, await validate(text));
          } catch {
            return [];
          }
        },
        {
          delay: 500,
          // What is valid depends on which cluster is answering, so a new
          // validator has to be asked about the document already on screen.
          needsRefresh: (update) =>
            update.transactions.some((t) => t.effects.some((e) => e.is(revalidate))),
        },
      );
      extensions.push(yaml(), yamlLinter, lintGutter());

      // Whatever the caller knows how to complete. `override` rather than
      // `addTo`, so nothing else offers suggestions behind their back.
      //
      // Registered unconditionally and read through the ref on each call: the
      // editor is only rebuilt when a structural option changes, and a caller
      // that resolves its completion source from the cluster hands it over
      // after mount. Gating on its presence here would mean it never arrived.
      extensions.push(
        autocompletion({ override: [(ctx) => completionsRef.current?.(ctx) ?? null] }),
      );
    }
    if (ariaLabel) extensions.push(EditorView.contentAttributes.of({ "aria-label": ariaLabel }));

    const view = new EditorView({
      state: EditorState.create({ doc: value, extensions }),
      parent,
    });
    viewRef.current = view;
    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // Re-create only when structural options change, not on every value/onChange.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [readOnly, language, ariaLabel, minHeight, maxHeight, fill]);

  // A new validator has to be asked about the document already on screen.
  // Swapping it changes what is true — a different cluster, a different set of
  // CRDs — but the linter is scheduled by edits, so without this the previous
  // validator's diagnostics stay up until someone types. (#326 review)
  useEffect(() => {
    const view = viewRef.current;
    if (view && language === "yaml") view.dispatch({ effects: revalidate.of(null) });
  }, [schemaValidate, language]);

  // Push external value changes into the editor (e.g. after Reset or reload).
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (value !== current) {
      view.dispatch({
        changes: { from: 0, to: current.length, insert: value },
        annotations: sync.of(true),
      });
    }
  }, [value]);

  return <div ref={parentRef} className="h-full w-full [&_.cm-editor]:h-full [&_.cm-scroller]:overflow-auto" />;
}
