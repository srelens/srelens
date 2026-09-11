import { useCallback, useEffect, useState } from "react";
import {
  describeError,
  extractApiVersionKind,
  fieldCompletions,
  openApiSchema,
  pathAtCursor,
  valueCompletions,
  type FieldCompletion,
  type SchemaBundle,
} from "@srelens/core";
import type { CodeEditorProps } from "@srelens/ui-kit";

/**
 * The OpenAPI schema behind a manifest, and what it says about the cursor.
 *
 * Core has the pieces — `openApiSchema` fetches a kind's schema from the
 * cluster, `pathAtCursor` reads the mapping path out of block YAML, and the
 * two completion helpers walk the schema — and the kit's editor takes a
 * completion source but resolves nothing itself. This is the join: one hook
 * that owns the schema for whatever kind the draft names, one function that
 * says what goes at a position, and one completion source built on the same
 * schema, so the sidebar and the popup can never disagree.
 */

export type SchemaStatus =
  /** The draft names no apiVersion and kind yet, so there is nothing to ask for. */
  | "none"
  | "loading"
  | "ready"
  /** The cluster answered, and publishes no schema for this kind. */
  | "absent"
  /** The lookup itself failed. Kept apart from `absent`: they are different
   *  facts, and saying "the cluster has no schema for Secret" when the request
   *  was refused or timed out is a confident wrong answer about a built-in
   *  kind that certainly has one. */
  | "failed";

/** The schema for the kind the draft names, loaded once per cluster, apiVersion and kind. */
export function useManifestSchema(
  context: string,
  yaml: string,
): {
  bundle: SchemaBundle | null;
  status: SchemaStatus;
  kind: string | null;
  /** Why the lookup failed, when it did. */
  error: string;
  /** Ask again — the failure may have been the cluster, not the kind. */
  retry: () => void;
} {
  const ident = extractApiVersionKind(yaml);
  const apiVersion = ident?.apiVersion ?? null;
  const kind = ident?.kind ?? null;
  const [state, setState] = useState<{ bundle: SchemaBundle | null; status: SchemaStatus; error: string }>({
    bundle: null,
    status: "none",
    error: "",
  });
  const [attempt, setAttempt] = useState(0);
  const retry = useCallback(() => setAttempt((n) => n + 1), []);

  useEffect(() => {
    if (!apiVersion || !kind) {
      setState({ bundle: null, status: "none", error: "" });
      return;
    }
    let active = true;
    setState({ bundle: null, status: "loading", error: "" });
    void openApiSchema(context, apiVersion, kind).then((out) => {
      if (!active) return;
      if ("error" in out) {
        setState({ bundle: null, status: "failed", error: describeError(out.error).detail });
      } else if (!out.key) {
        // The document came back and nothing in it declares this kind.
        setState({ bundle: null, status: "absent", error: "" });
      } else {
        setState({ bundle: out, status: "ready", error: "" });
      }
    });
    return () => {
      active = false;
    };
  }, [context, apiVersion, kind, attempt]);

  return { bundle: state.bundle, status: state.status, error: state.error, kind, retry };
}

/** What the schema allows at one position: the keys of the mapping the cursor
 *  is in, or — on a `key: ` line — the values that key takes. */
export interface KeysHere {
  /** The mapping path down to the cursor, outermost first; empty at the top level. */
  path: string[];
  /** True when the cursor sits after `key:` and `entries` are that key's values. */
  onValue: boolean;
  valueKey?: string;
  entries: FieldCompletion[];
}

export function keysAt(bundle: SchemaBundle, yaml: string, pos: number): KeysHere {
  const at = pathAtCursor(yaml, Math.max(0, Math.min(pos, yaml.length)));
  if (at.onValue && at.valueKey) {
    const values = valueCompletions(bundle, at.path, at.valueKey);
    // A key with no fixed set of values (a name, a number) has nothing to
    // list; the keys beside it are more use than an empty section.
    if (values.length > 0) return { path: at.path, onValue: true, valueKey: at.valueKey, entries: values };
  }
  return { path: at.path, onValue: false, entries: fieldCompletions(bundle, at.path) };
}

type Completions = NonNullable<CodeEditorProps["completions"]>;

/**
 * The editor's completion source: field names for the mapping at the cursor,
 * enum members and booleans after `key: `. Reads the bundle through a getter
 * so one source serves the editor's whole life while the schema arrives, or
 * changes with the kind, underneath it.
 */
export function schemaCompletions(bundle: () => SchemaBundle | null): Completions {
  return (ctx) => {
    const current = bundle();
    if (!current) return null;
    const word = ctx.matchBefore(/[\w.-]*/);
    if (!word || (word.from === word.to && !ctx.explicit)) return null;
    const { path, onValue, valueKey } = pathAtCursor(ctx.state.doc.toString(), ctx.pos);
    const found =
      onValue && valueKey ? valueCompletions(current, path, valueKey) : fieldCompletions(current, path);
    if (found.length === 0) return null;
    return {
      from: word.from,
      options: found.map((f) => ({ label: f.label, detail: f.detail, info: f.info })),
    };
  };
}
