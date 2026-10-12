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
 * One logfmt token. A quoted run may hold spaces. Each alternative eats one
 * character or one whole quoted string, so a failed match has nothing to
 * backtrack into: these lines are streamed from the cluster, the least
 * controlled input the app reads (see `tokenize` on #380).
 */
const LOGFMT_TOKEN = /(?:[^\s"]|"(?:[^"\\]|\\.)*")+/g;
const LOGFMT_PAIR = /^([A-Za-z_][\w.@/-]*)=(.*)$/s;

export function parseLogFields(text: string): LogFields {
  const obj = logObject(text);
  if (obj !== null) {
    const fields = emptyFields();
    flatten(obj, "", fields);
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

function flatten(obj: Readonly<Record<string, unknown>>, prefix: string, out: Record<string, string>): void {
  for (const [key, value] of Object.entries(obj)) {
    const path = prefix === "" ? key : `${prefix}.${key}`;
    if (value !== null && typeof value === "object" && !Array.isArray(value)) {
      flatten(value as Record<string, unknown>, path, out);
    } else {
      out[path] = typeof value === "string" ? value : JSON.stringify(value);
    }
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
    pairs.unshift([pair[1], unquote(pair[2])]);
  }
  if (pairs.length < 2) return null;
  const fields = emptyFields();
  for (const [key, value] of pairs) fields[key] = value;
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
