import { logObject, tokenize } from "./logTerms";

/**
 * A log line read as structured data — the message the row should lead with,
 * and every field the line carries — for the Logs screen's query, facets and
 * line detail (#898).
 */
export interface LogFields {
  /** What the message column shows. */
  readonly message: string;
  /** Flattened, stringified, keys as written. Empty for plain text. */
  readonly fields: Readonly<Record<string, string>>;
  /**
   * Whether `message` was taken OUT of a field: any JSON line, or logfmt that
   * carries `msg`. Only then does a row append the other fields after it. A
   * prose line with a `key=value` tail is already readable as written, so it
   * keeps its text and contributes its fields to the facets alone.
   */
  readonly structured: boolean;
}

const MESSAGE_KEYS = ["msg", "message"] as const;

/** Keys a row never repeats inline and a facet never offers: each already has a column. */
export const HIDDEN_KEYS: ReadonlySet<string> = new Set([
  "msg",
  "message",
  "level",
  "severity",
  "time",
  "ts",
  "timestamp",
  "@timestamp",
]);

/**
 * One logfmt token. A quoted run may hold spaces, and an unterminated one runs
 * to the end of the line. The quoted alternative therefore never fails once it
 * starts, so no start position is rescanned and the match is linear in the
 * line. These lines are streamed from the cluster, the least controlled input
 * the app reads (see `tokenize` on #380).
 */
const LOGFMT_TOKEN = /(?:[^\s"]|"(?:[^"\\]|\\.)*"?)+/g;
const LOGFMT_PAIR = /^([A-Za-z_][\w.@/-]*)=(.*)$/s;

/**
 * Reads a line as structured data. A streamed line is untrusted input, so this
 * never throws: a line it cannot read comes back as plain text.
 */
export function parseLogFields(text: string): LogFields {
  const obj = logObject(text);
  if (obj !== null) {
    const fields = emptyFields();
    flatten(obj, "", fields, 0);
    return { message: messageField(fields) ?? "", fields, structured: true };
  }
  const fields = logfmtTail(text);
  if (fields === null) return { message: text, fields: {}, structured: false };
  const message = messageField(fields);
  return message === undefined
    ? { message: text, fields, structured: false }
    : { message, fields, structured: true };
}

/**
 * Null-prototype, so a line's own `__proto__` or `constructor` key is a field
 * like any other rather than a prototype assignment or an inherited function.
 */
function emptyFields(): Record<string, string> {
  return Object.create(null) as Record<string, string>;
}

/**
 * A nested object whose path would reach this many keys is stored as its JSON
 * text rather than flattened further: the facets need a few levels, and
 * recursing without a cap is what overflowed the stack on a hostile line.
 */
const MAX_DEPTH = 8;

/**
 * A dotted key and a nested key can collide (`{"a.b":1,"a":{"b":2}}`); the
 * last one wins, accepted.
 */
function flatten(
  obj: Readonly<Record<string, unknown>>,
  prefix: string,
  out: Record<string, string>,
  depth: number,
): void {
  for (const [key, value] of Object.entries(obj)) {
    const path = prefix === "" ? key : `${prefix}.${key}`;
    if (value !== null && typeof value === "object" && !Array.isArray(value) && depth + 1 < MAX_DEPTH) {
      flatten(value as Record<string, unknown>, path, out, depth + 1);
    } else {
      out[path] = typeof value === "string" ? value : stringifyValue(value);
    }
  }
}

/**
 * JSON.stringify recurses too, so a deeply nested value can throw where the
 * line parsed fine. An unreadable value becomes an ellipsis instead.
 */
function stringifyValue(value: unknown): string {
  try {
    return JSON.stringify(value);
  } catch {
    return "…";
  }
}

function messageField(fields: Readonly<Record<string, string>>): string | undefined {
  for (const key of MESSAGE_KEYS) {
    const value = Object.hasOwn(fields, key) ? fields[key] : undefined;
    if (value !== undefined && value.trim() !== "") return value;
  }
  return undefined;
}

/**
 * The longest run of `key=value` tokens at the END of the line, when it holds
 * at least two. That covers whole-line logfmt and klog's structured tail
 * alike. One pair is too weak a signal: `starting api build=4f2a` is prose.
 */
function logfmtTail(text: string): Record<string, string> | null {
  const tokens = text.match(LOGFMT_TOKEN) ?? [];
  const pairs: [string, string][] = [];
  for (let i = tokens.length - 1; i >= 0; i -= 1) {
    const pair = LOGFMT_PAIR.exec(tokens[i]);
    if (pair === null) break;
    pairs.push([pair[1], unquote(pair[2])]);
  }
  if (pairs.length < 2) return null;
  const fields = emptyFields();
  for (const [key, value] of pairs.reverse()) fields[key] = value;
  return fields;
}

function unquote(value: string): string {
  if (value.length < 2 || !value.startsWith('"') || !value.endsWith('"')) return value;
  try {
    const parsed: unknown = JSON.parse(value);
    return typeof parsed === "string" ? parsed : value.slice(1, -1);
  } catch {
    return value.slice(1, -1);
  }
}

/**
 * A message as the term tally reads it — `tokenize`'s words, lowercased,
 * single-spaced. A phrase is matched against this too, which is how a
 * clicked Top term (`Success Wrote batch`, tallied from `Success: Wrote
 * batch`) finds the lines it was counted from.
 */
export function logLineWords(message: string): string {
  return tokenize(message)
    .map((t) => t.text.toLowerCase())
    .join(" ");
}

/** A structured row's fields after its message: `key=value`, in the line's order. */
export function inlineFields(fields: Readonly<Record<string, string>>): string {
  return Object.entries(fields)
    .filter(([key]) => !HIDDEN_KEYS.has(key.toLowerCase()))
    .map(([key, value]) => `${key}=${/\s/.test(value) ? JSON.stringify(value) : value}`)
    .join(" ");
}
