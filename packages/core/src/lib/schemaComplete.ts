import type { SchemaBundle } from "./schema";

// A JSON-schema node from the OpenAPI doc (loosely typed; we only read a few keys).
type Schema = {
  $ref?: string;
  type?: string;
  description?: string;
  enum?: unknown[];
  properties?: Record<string, Schema>;
  items?: Schema;
  /** k8s wraps object property refs as `allOf: [{ $ref }]` (to add description). */
  allOf?: Schema[];
};

export interface FieldCompletion {
  label: string;
  /** Type hint shown after the label (e.g. "integer", "PodSpec"). */
  detail?: string;
  /** Longer description for the docs popup. */
  info?: string;
}

/** Read `apiVersion`/`kind` from the manifest text (order-independent). */
export function extractApiVersionKind(text: string): { apiVersion: string; kind: string } | null {
  const apiVersion = text.match(/^\s*apiVersion:\s*["']?([\w./-]+)["']?\s*$/m)?.[1];
  const kind = text.match(/^\s*kind:\s*["']?([\w.-]+)["']?\s*$/m)?.[1];
  return apiVersion && kind ? { apiVersion, kind } : null;
}

/**
 * Compute the mapping-key path to the cursor using indentation, plus whether the
 * cursor sits in a value position (`key: <here>`). Block-YAML heuristic — good
 * enough for editing manifests, and cheap.
 */
export function pathAtCursor(
  text: string,
  pos: number,
): { path: string[]; onValue: boolean; valueKey?: string } {
  const before = text.slice(0, pos);
  const lines = before.split("\n");
  const current = lines[lines.length - 1];
  const curIndent = current.match(/^(\s*)/)?.[1].length ?? 0;

  const path: string[] = [];
  let indent = curIndent;
  for (let i = lines.length - 2; i >= 0; i--) {
    const line = lines[i];
    if (!line.trim() || /^\s*#/.test(line)) continue;
    const m = line.match(/^(\s*)(-\s+)?([\w.-]+):/);
    if (!m) continue;
    const keyCol = m[1].length + (m[2]?.length ?? 0);
    if (keyCol < indent) {
      path.unshift(m[3]);
      indent = m[1].length;
    }
  }

  // `key: partialValue` on the current line → completing a value.
  const valueMatch = current.match(/^\s*(-\s+)?([\w.-]+):\s+\S*$/);
  if (valueMatch) return { path, onValue: true, valueKey: valueMatch[2] };
  return { path, onValue: false };
}

/**
 * Resolve a schema node to something with concrete `properties`/`type`:
 * follows `$ref` and flattens `allOf` (merging member properties). k8s v3
 * property refs look like `{ description, allOf: [{ $ref }] }`, so a plain
 * `$ref` walk isn't enough.
 */
function deref(bundle: SchemaBundle, schema: Schema | undefined, depth = 0): Schema | undefined {
  if (!schema || depth > 20) return schema;
  if (schema.$ref) {
    const key = schema.$ref.replace("#/components/schemas/", "");
    return deref(bundle, bundle.schemas[key] as Schema | undefined, depth + 1);
  }
  if (Array.isArray(schema.allOf) && schema.allOf.length) {
    const merged: Schema = { ...schema, properties: { ...(schema.properties ?? {}) } };
    delete merged.allOf;
    for (const sub of schema.allOf) {
      const r = deref(bundle, sub, depth + 1);
      if (!r) continue;
      merged.properties = { ...merged.properties, ...(r.properties ?? {}) };
      merged.type ??= r.type;
      merged.items ??= r.items;
      merged.enum ??= r.enum;
    }
    return merged;
  }
  return schema;
}

/** Resolve the schema of the mapping at `path` (following $ref/allOf and array items). */
function schemaAtPath(bundle: SchemaBundle, path: string[]): Schema | undefined {
  if (!bundle.key) return undefined;
  let s = deref(bundle, bundle.schemas[bundle.key] as Schema);
  for (const seg of path) {
    if (!s) return undefined;
    // Descend into array items when the previous segment was an array.
    if (s.items && (s.type === "array" || !s.properties)) s = deref(bundle, s.items);
    s = s?.properties ? deref(bundle, s.properties[seg]) : undefined;
  }
  return s;
}

function typeLabel(bundle: SchemaBundle, schema: Schema | undefined): string {
  const s = schema;
  if (!s) return "";
  const ref = s.$ref ?? s.allOf?.find((a) => a.$ref)?.$ref;
  if (ref) return ref.split(".").pop() ?? "object";
  if (s.type === "array") {
    const item = deref(bundle, s.items);
    return `${typeLabel(bundle, item) || "item"}[]`;
  }
  return s.type ?? "object";
}

/** Field-name completions for the mapping at `path` (its schema's properties). */
export function fieldCompletions(bundle: SchemaBundle, path: string[]): FieldCompletion[] {
  let s = schemaAtPath(bundle, path);
  // Cursor inside an array item → complete the item type's fields.
  if (s?.type === "array" || (s?.items && !s.properties)) s = deref(bundle, s?.items);
  const props = s?.properties;
  if (!props) return [];
  return Object.entries(props).map(([name, raw]) => ({
    label: name,
    detail: typeLabel(bundle, raw),
    info: deref(bundle, raw)?.description,
  }));
}

/** Value completions for `path.key` — enum members, or booleans. */
export function valueCompletions(bundle: SchemaBundle, path: string[], key: string): FieldCompletion[] {
  let s = schemaAtPath(bundle, path);
  if (s?.type === "array" || (s?.items && !s.properties)) s = deref(bundle, s?.items);
  const prop = deref(bundle, s?.properties?.[key]);
  if (!prop) return [];
  if (Array.isArray(prop.enum)) return prop.enum.map((v) => ({ label: String(v), detail: "enum" }));
  if (prop.type === "boolean") return [{ label: "true" }, { label: "false" }];
  return [];
}
