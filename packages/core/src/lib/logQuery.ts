import type { HealthKind } from "./k8sHealth";
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

/** The line as the query reads it. The Logs screen's `Row` extends this. */
export interface QueryLine {
  /** The line with its k8s stamp stripped: what a regex tests. */
  readonly text: string;
  /** `${text} ${health}`, lowercased: what free text and phrases search. */
  readonly haystack: string;
  /** `logLineWords(message)`: what a phrase also matches, at word boundaries. */
  readonly words: string;
  readonly fields: Readonly<Record<string, string>>;
  /** The level word as the line spelled it; `""` when it has none. */
  readonly level: string;
  readonly health: HealthKind;
  /** Epoch ms from the stream's stamp; absent for a line that came without one. */
  readonly at?: number;
}

interface ClauseBase {
  /** The field it reads, as typed. Absent for free text; `level` is the line's level word. */
  readonly field?: string;
  readonly negated: boolean;
  /** Where it sits in the query text, `-` included: `[start, end)`. */
  readonly span: readonly [number, number];
}

export type QueryClause = ClauseBase &
  (
    | { readonly match: "text" | "equals"; readonly value: string }
    | { readonly match: "phrase"; readonly value: string; readonly words: string }
    | { readonly match: "regex"; readonly value: string; readonly regex: RegExp }
  );

export type ParsedQuery =
  | { readonly ok: true; readonly clauses: readonly QueryClause[] }
  | { readonly ok: false; readonly error: string; readonly at: number };

const QUERYABLE_KEY = /^[A-Za-z_@][\w.@/-]*$/;
const FIELD_PREFIX = /[A-Za-z_@][\w.@/-]*:/y;

/** Whether `key:` would parse as a field. A facet or a filter button may only offer one that does. */
export function isQueryableKey(key: string): boolean {
  return QUERYABLE_KEY.test(key);
}

const isSpace = (ch: string) => /\s/.test(ch);

type Term =
  | { readonly kind: "bare" | "quoted" | "regex"; readonly value: string; readonly end: number }
  | { readonly error: string; readonly at: number };

/**
 * Whitespace-separated clauses: `word`, `"phrase"`, `/regex/`, `field:value`,
 * `field:"value"`, `field:/regex/`, each optionally `-`-negated. Everything
 * that compares is lowercased here once, so matching 5,000 lines does no
 * case folding of the query.
 */
export function parseLogQuery(query: string): ParsedQuery {
  const clauses: QueryClause[] = [];
  let i = 0;
  while (i < query.length) {
    if (isSpace(query[i])) {
      i += 1;
      continue;
    }
    const start = i;
    const negated = query[i] === "-";
    if (negated) i += 1;
    if (i >= query.length || isSpace(query[i])) {
      return { ok: false, error: "A '-' needs something to exclude", at: start };
    }
    FIELD_PREFIX.lastIndex = i;
    const prefix = FIELD_PREFIX.exec(query);
    const field = prefix === null ? undefined : prefix[0].slice(0, -1);
    if (prefix !== null) i += prefix[0].length;
    const term = readTerm(query, i);
    if ("error" in term) return { ok: false, error: term.error, at: term.at };
    if (term.end < query.length && !isSpace(query[term.end])) {
      return { ok: false, error: "Put a space after a quote or a regex", at: term.end };
    }
    const clause = toClause(field, term, negated, [start, term.end]);
    if ("error" in clause) return { ok: false, error: clause.error, at: start };
    clauses.push(clause);
    i = term.end;
  }
  return { ok: true, clauses };
}

function readTerm(q: string, i: number): Term {
  if (i >= q.length || isSpace(q[i])) {
    // Only reachable after a field prefix: the loop never starts a term on a space.
    return { error: "A field needs a value after ':'", at: i };
  }
  if (q[i] === '"') {
    const close = q.indexOf('"', i + 1);
    if (close < 0) return { error: "This quote is never closed", at: i };
    return { kind: "quoted", value: q.slice(i + 1, close), end: close + 1 };
  }
  if (q[i] === "/") {
    let j = i + 1;
    while (j < q.length && q[j] !== "/") j += q[j] === "\\" ? 2 : 1;
    if (j >= q.length) {
      return { error: "This regex is never closed; to search for a path, put it in quotes", at: i };
    }
    return { kind: "regex", value: q.slice(i + 1, j), end: j + 1 };
  }
  let j = i;
  while (j < q.length && !isSpace(q[j])) j += 1;
  return { kind: "bare", value: q.slice(i, j), end: j };
}

function toClause(
  field: string | undefined,
  term: { readonly kind: "bare" | "quoted" | "regex"; readonly value: string },
  negated: boolean,
  span: readonly [number, number],
): QueryClause | { readonly error: string } {
  const base = { ...(field === undefined ? {} : { field }), negated, span };
  if (term.kind === "regex") {
    if (term.value === "") {
      return { error: "An empty regex matches everything; to search for a URL, put it in quotes" };
    }
    try {
      return { ...base, match: "regex", value: term.value, regex: new RegExp(term.value, "i") };
    } catch (e) {
      return { error: `Invalid regex: ${e instanceof Error ? e.message : String(e)}` };
    }
  }
  const value = term.value.toLowerCase();
  if (field !== undefined) return { ...base, match: "equals", value };
  if (term.kind === "quoted") {
    if (value === "") return { error: "An empty phrase matches everything" };
    return { ...base, match: "phrase", value, words: logLineWords(term.value) };
  }
  return { ...base, match: "text", value };
}

/**
 * Whether a line passes. Positive clauses on ONE field OR together — so two
 * clicks on the Level facet show both levels — and everything else ANDs.
 * `ignoreField` skips that field's clauses, which is how a facet counts its
 * own values without filtering itself down to the one already picked.
 */
export function matchLine(line: QueryLine, clauses: readonly QueryClause[], ignoreField?: string): boolean {
  const ignored = ignoreField?.toLowerCase();
  let anyOf: Map<string, boolean> | undefined;
  for (const clause of clauses) {
    const field = clause.field?.toLowerCase();
    if (field !== undefined && field === ignored) continue;
    const hit = hits(line, clause);
    if (clause.negated) {
      if (hit) return false;
    } else if (field === undefined) {
      if (!hit) return false;
    } else {
      anyOf ??= new Map();
      anyOf.set(field, (anyOf.get(field) ?? false) || hit);
    }
  }
  if (anyOf !== undefined) for (const ok of anyOf.values()) if (!ok) return false;
  return true;
}

function hits(line: QueryLine, clause: QueryClause): boolean {
  if (clause.field === undefined) {
    if (clause.match === "regex") return clause.regex.test(line.text);
    if (line.haystack.includes(clause.value)) return true;
    return clause.match === "phrase" && clause.words !== "" && ` ${line.words} `.includes(` ${clause.words} `);
  }
  const value = fieldValue(line, clause.field);
  if (value === undefined) return false;
  return clause.match === "regex" ? clause.regex.test(value) : value.toLowerCase() === clause.value;
}

/** A field's value, with `level` meaning the line's level word. Own keys only. */
export function fieldValue(line: QueryLine, field: string): string | undefined {
  const lower = field.toLowerCase();
  if (lower === "level") return line.level === "" ? undefined : line.level;
  if (Object.hasOwn(line.fields, field)) return line.fields[field];
  for (const key of Object.keys(line.fields)) if (key.toLowerCase() === lower) return line.fields[key];
  return undefined;
}

/** What a click filters on: a field's value, or a Top term as a phrase. */
export type ClauseTarget = { readonly field: string; readonly value: string } | { readonly phrase: string };
export type ClauseMode = "include" | "exclude";

/** Whether the query already includes or excludes `target`. */
export function clauseState(clauses: readonly QueryClause[], target: ClauseTarget): ClauseMode | undefined {
  const found = clauses.find((c) => sameTarget(c, target));
  return found === undefined ? undefined : found.negated ? "exclude" : "include";
}

/**
 * The query after a facet click: absent → appended; same mode → removed;
 * other mode → flipped in place. Every other character of what was typed is
 * left alone — the box is the reader's, and a click must not rewrite it.
 */
export function toggleClause(query: string, target: ClauseTarget, mode: ClauseMode): string {
  const parsed = parseLogQuery(query);
  const found = parsed.ok ? parsed.clauses.find((c) => sameTarget(c, target)) : undefined;
  const spelled = mode === "exclude" ? `-${spell(target)}` : spell(target);
  if (found === undefined) return append(query, spelled);
  if ((mode === "exclude") === found.negated) return removeSpan(query, found.span);
  return query.slice(0, found.span[0]) + spelled + query.slice(found.span[1]);
}

/** The query without `clause`, which must come from parsing this same text. */
export function removeClause(query: string, clause: QueryClause): string {
  return removeSpan(query, clause.span);
}

function sameTarget(c: QueryClause, t: ClauseTarget): boolean {
  if ("phrase" in t) {
    if (c.field !== undefined) return false;
    return (
      (c.match === "phrase" && c.value === t.phrase.toLowerCase()) ||
      (c.match === "regex" && c.value === escapeRegex(t.phrase))
    );
  }
  if (c.field?.toLowerCase() !== t.field.toLowerCase()) return false;
  return (
    (c.match === "equals" && c.value === t.value.toLowerCase()) ||
    (c.match === "regex" && c.value === `^${escapeRegex(t.value)}$`)
  );
}

function spell(t: ClauseTarget): string {
  if ("phrase" in t) return t.phrase.includes('"') ? `/${escapeRegex(t.phrase)}/` : `"${t.phrase}"`;
  return `${t.field}:${spellValue(t.value)}`;
}

/** A value spelled so it parses back to itself: a quote cannot be quoted, so it becomes an exact regex. */
function spellValue(value: string): string {
  if (value.includes('"')) return `/^${escapeRegex(value)}$/`;
  if (value === "" || /[\s:]/.test(value) || value.startsWith("/")) return `"${value}"`;
  return value;
}

function escapeRegex(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&");
}

function append(query: string, clause: string): string {
  const base = query.trimEnd();
  return base === "" ? clause : `${base} ${clause}`;
}

function removeSpan(query: string, [start, end]: readonly [number, number]): string {
  const left = query.slice(0, start).trimEnd();
  const right = query.slice(end).trimStart();
  return left !== "" && right !== "" ? `${left} ${right}` : left + right;
}

export interface FacetValue {
  readonly value: string;
  readonly count: number;
}

const MIN_COVERAGE = 0.2;
const MIN_DISTINCT = 2;
const MAX_DISTINCT = 50;
const MAX_FIELDS = 6;

/**
 * The fields worth a facet: carried by at least a fifth of the lines that have
 * fields at all, with between two and fifty values. One value leaves nothing
 * to choose between, and more than fifty is an id or a timestamp, not a
 * category. Run over the whole buffer, never the filtered view, so the rail's
 * sections don't come and go as the reader filters.
 */
export function discoverFields(lines: readonly QueryLine[]): string[] {
  let structured = 0;
  const seen = new Map<string, { lines: number; values: Set<string> }>();
  for (const line of lines) {
    const keys = Object.keys(line.fields);
    if (keys.length === 0) continue;
    structured += 1;
    for (const key of keys) {
      if (HIDDEN_KEYS.has(key.toLowerCase()) || !isQueryableKey(key)) continue;
      let entry = seen.get(key);
      if (entry === undefined) {
        entry = { lines: 0, values: new Set() };
        seen.set(key, entry);
      }
      entry.lines += 1;
      // Past the cap the key is out anyway, so the set stops growing.
      if (entry.values.size <= MAX_DISTINCT) entry.values.add(line.fields[key].toLowerCase());
    }
  }
  return [...seen.entries()]
    .filter(
      ([, e]) =>
        e.lines >= structured * MIN_COVERAGE && e.values.size >= MIN_DISTINCT && e.values.size <= MAX_DISTINCT,
    )
    .sort((a, b) => b[1].lines - a[1].lines || compare(a[0], b[0]))
    .slice(0, MAX_FIELDS)
    .map(([key]) => key);
}

/**
 * A field's values among the lines every OTHER clause keeps, most frequent
 * first. Grouped case-insensitively, because that is how they match, and
 * named by the first spelling seen.
 */
export function facetCounts(lines: readonly QueryLine[], clauses: readonly QueryClause[], field: string): FacetValue[] {
  const counts = new Map<string, { value: string; count: number }>();
  for (const line of lines) {
    const value = fieldValue(line, field);
    if (value === undefined || !matchLine(line, clauses, field)) continue;
    const key = value.toLowerCase();
    const entry = counts.get(key);
    if (entry === undefined) counts.set(key, { value, count: 1 });
    else entry.count += 1;
  }
  return [...counts.values()].sort((a, b) => b.count - a.count || compare(a.value, b.value));
}

function compare(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}
