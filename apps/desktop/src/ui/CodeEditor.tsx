import React, { useEffect, useRef } from "react";
import { EditorState } from "@codemirror/state";
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
import { autocompletion, completionKeymap, type CompletionContext, type CompletionResult } from "@codemirror/autocomplete";
import { parseAllDocuments } from "yaml";
import { tags as t } from "@lezer/highlight";
import type { SchemaBundle } from "../lib/schema";
import { extractApiVersionKind, pathAtCursor, fieldCompletions, valueCompletions } from "../lib/schemaComplete";

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

/** Split a k8s field path ("spec.template.spec.containers[0].image") into
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

/** Field paths mentioned in a k8s validation message (unknown field / invalid value). */
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
 * Map Kubernetes validation messages (from server-side dry-run) onto editor
 * ranges. Positions each message at the offending field when it can be located
 * in the YAML, else at the top of the document — so the error is always shown.
 * Pure + tested.
 */
export function k8sDiagnostics(text: string, errors: Array<{ docIndex: number; message: string }>): Diagnostic[] {
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
  { tag: [t.definition(t.propertyName), t.propertyName, t.labelName], color: "var(--fl-syntax-key)" },
  { tag: [t.string, t.special(t.string)], color: "var(--fl-syntax-string)" },
  { tag: [t.number, t.integer, t.float], color: "var(--fl-syntax-number)" },
  { tag: [t.bool, t.null, t.keyword, t.atom], color: "var(--fl-syntax-bool)" },
  { tag: [t.comment, t.lineComment, t.blockComment], color: "var(--fl-syntax-comment)", fontStyle: "italic" },
  { tag: [t.meta, t.punctuation, t.separator], color: "var(--fl-color-text-muted)" },
]);

/** Editor chrome, themed from CSS tokens (so light/dark just works). */
function editorTheme(minHeight: number, maxHeight: number, fill: boolean) {
  return EditorView.theme({
    "&": {
      color: "var(--fl-color-text)",
      backgroundColor: "var(--fl-color-bg)",
      fontSize: "12px",
      border: "1px solid var(--fl-color-border)",
      borderRadius: "var(--fl-radius-md)",
      // `fill` makes the editor take its container's full height (scrolling
      // internally); otherwise it grows with content up to `maxHeight`.
      ...(fill ? { height: "100%" } : { maxHeight: `${maxHeight}px` }),
    },
    "&.cm-focused": { outline: "none", borderColor: "var(--fl-color-accent)" },
    ".cm-scroller": { fontFamily: "var(--fl-font-mono)", lineHeight: "1.55", overflow: "auto" },
    ".cm-content": { minHeight: fill ? "0" : `${minHeight}px`, caretColor: "var(--fl-color-accent)" },
    ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--fl-color-accent)" },
    ".cm-gutters": {
      backgroundColor: "var(--fl-color-surface)",
      color: "var(--fl-color-text-muted)",
      border: "none",
      borderRight: "1px solid var(--fl-color-border-faint)",
    },
    ".cm-activeLineGutter": { backgroundColor: "var(--fl-color-surface-alt)", color: "var(--fl-color-text)" },
    ".cm-activeLine": { backgroundColor: "rgba(127, 140, 150, 0.08)" },
    ".cm-foldPlaceholder": {
      backgroundColor: "var(--fl-color-surface-alt)",
      border: "none",
      color: "var(--fl-color-text-muted)",
    },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": {
      backgroundColor: "rgba(0, 167, 160, 0.25)",
    },
    ".cm-selectionMatch": { backgroundColor: "rgba(0, 167, 160, 0.18)" },
    ".cm-matchingBracket, &.cm-focused .cm-matchingBracket": {
      backgroundColor: "rgba(0, 167, 160, 0.22)",
      outline: "1px solid var(--fl-color-accent)",
    },
    ".cm-panels": { backgroundColor: "var(--fl-color-surface)", color: "var(--fl-color-text)" },
    ".cm-searchMatch": { backgroundColor: "rgba(210, 153, 34, 0.3)" },
    ".cm-tooltip": { maxWidth: "480px" },
    ".cm-tooltip.cm-tooltip-lint": {
      backgroundColor: "var(--fl-color-surface)",
      border: "1px solid var(--fl-color-border)",
      borderRadius: "var(--fl-radius-md)",
      boxShadow: "0 4px 16px rgba(0, 0, 0, 0.18)",
      color: "var(--fl-color-text)",
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
    ".cm-diagnostic-error": { borderLeft: "3px solid var(--fl-color-danger)" },
    ".cm-tooltip.cm-tooltip-autocomplete": {
      backgroundColor: "var(--fl-color-surface)",
      border: "1px solid var(--fl-color-border)",
      borderRadius: "var(--fl-radius-md)",
      boxShadow: "0 4px 16px rgba(0, 0, 0, 0.18)",
    },
    ".cm-tooltip-autocomplete > ul > li": {
      padding: "2px 8px",
      fontFamily: "var(--fl-font-mono)",
      fontSize: "12px",
      color: "var(--fl-color-text)",
    },
    ".cm-tooltip-autocomplete > ul > li[aria-selected]": {
      backgroundColor: "var(--fl-color-accent)",
      color: "#fff",
    },
    ".cm-completionDetail": { color: "var(--fl-color-text-muted)", fontStyle: "italic", marginLeft: "6px" },
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
   * k8s-aware validation: given the YAML, resolve to server-side validation
   * error messages (empty = valid). Wired to `k8s.validateManifest`. When set,
   * the editor lints against the API server in addition to YAML syntax.
   */
  schemaValidate?: (yaml: string) => Promise<Array<{ docIndex: number; message: string }>>;
  /**
   * k8s field autocomplete: resolve the OpenAPI schema for a kind (wired to
   * `k8s.openApiSchema`). When set, the editor offers field-name and enum-value
   * completions from the cluster's schema (CRDs included).
   */
  schemaSource?: (apiVersion: string, kind: string) => Promise<SchemaBundle | null>;
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
  schemaSource,
}: CodeEditorProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  // Keep the latest onChange/validate without re-creating the editor on every render.
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const validateRef = useRef(schemaValidate);
  validateRef.current = schemaValidate;
  const schemaSourceRef = useRef(schemaSource);
  schemaSourceRef.current = schemaSource;
  // Cache fetched schemas per (apiVersion, kind) for the editor's lifetime.
  const schemaCacheRef = useRef(new Map<string, Promise<SchemaBundle | null>>());

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
        if (u.docChanged) onChangeRef.current?.(u.state.doc.toString());
      }),
    ];
    if (language === "yaml") {
      // Lint YAML syntax first (local, instant); if it parses, validate against
      // the API server (debounced via the linter delay) for k8s-aware errors.
      const yamlLinter = linter(
        async (view) => {
          const text = view.state.doc.toString();
          const syntax = yamlDiagnostics(text);
          if (syntax.length) return syntax;
          const validate = validateRef.current;
          if (!validate || !text.trim()) return [];
          try {
            return k8sDiagnostics(text, await validate(text));
          } catch {
            return [];
          }
        },
        { delay: 500 },
      );
      extensions.push(yaml(), yamlLinter, lintGutter());

      // k8s field/value autocomplete from the cluster's OpenAPI schema.
      const completionSource = async (ctx: CompletionContext): Promise<CompletionResult | null> => {
        const provide = schemaSourceRef.current;
        if (!provide) return null;
        const text = ctx.state.doc.toString();
        const kv = extractApiVersionKind(text);
        if (!kv) return null;
        const cacheKey = `${kv.apiVersion}\n${kv.kind}`;
        let bundle = schemaCacheRef.current.get(cacheKey);
        if (!bundle) {
          bundle = provide(kv.apiVersion, kv.kind).catch(() => null);
          schemaCacheRef.current.set(cacheKey, bundle);
        }
        const schema = await bundle;
        if (!schema?.key) return null;

        const { path, onValue, valueKey } = pathAtCursor(text, ctx.pos);
        const items =
          onValue && valueKey ? valueCompletions(schema, path, valueKey) : fieldCompletions(schema, path);
        if (!items.length) return null;

        const word = ctx.matchBefore(/[\w.-]*/);
        if (word?.from === word?.to && !ctx.explicit) return null; // don't pop on empty unless invoked
        const from = word ? word.from : ctx.pos;
        const lineEnd = text.indexOf("\n", ctx.pos);
        const rest = text.slice(ctx.pos, lineEnd === -1 ? undefined : lineEnd);
        const addColon = !onValue && !rest.includes(":");
        return {
          from,
          options: items.map((it) => ({
            label: it.label,
            type: onValue ? "enum" : "property",
            detail: it.detail,
            info: it.info,
            apply: !onValue && addColon ? `${it.label}: ` : it.label,
          })),
        };
      };
      extensions.push(autocompletion({ override: [completionSource] }));
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

  // Push external value changes into the editor (e.g. after Reset or reload).
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (value !== current) {
      view.dispatch({ changes: { from: 0, to: current.length, insert: value } });
    }
  }, [value]);

  return <div ref={parentRef} className="fl-editor" />;
}
