import type { HealthKind } from "./k8sHealth";
import { logObject, QUOTE_CHARS, TRAILING_CHARS, tokenize } from "./logTerms";

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
  if (fields === null) return { message: text, fields: emptyFields(), structured: false };
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

/**
 * The first non-blank `msg`, else `message`, in any case (`inlineFields` hides
 * them in any case, so a message it hid must be one the row shows); among
 * spellings of one name, the key that came first.
 */
function messageField(fields: Readonly<Record<string, string>>): string | undefined {
  const entries = Object.entries(fields);
  for (const name of MESSAGE_KEYS) {
    for (const [key, value] of entries) {
      if (key.toLowerCase() === name && value.trim() !== "") return value;
    }
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

export type CompareOp = ">" | ">=" | "<" | "<=";

export type QueryClause = ClauseBase &
  (
    | { readonly match: "text" | "equals"; readonly value: string }
    | { readonly match: "phrase"; readonly value: string; readonly words: string }
    | { readonly match: "regex"; readonly value: string; readonly regex: RegExp }
    /**
     * `field>5`, `field<=1.5s`. `value` is the right side as typed, for the pill
     * and for editing by span; `target` is the number, or the duration in seconds.
     */
    | {
        readonly match: "compare";
        readonly field: string;
        readonly op: CompareOp;
        readonly kind: "number" | "duration";
        readonly target: number;
        readonly value: string;
      }
  );

export type ParsedQuery =
  | { readonly ok: true; readonly clauses: readonly QueryClause[] }
  | { readonly ok: false; readonly error: string; readonly at: number };

/**
 * A field name is `[A-Za-z_@][A-Za-z0-9_.@/-]*`. It is scanned by index, not by
 * a regex over the query, which CodeQL reads as polynomial on a run of `@`
 * (see `tokenize` on #380). The parser and `isQueryableKey` share these two
 * predicates, so they cannot disagree about what a field is.
 */
const isKeyStart = (ch: string) => /^[A-Za-z_@]$/.test(ch);
const isKeyChar = (ch: string) => /^[A-Za-z0-9_.@/-]$/.test(ch);

/** Index just past the key that starts at `from`, or `from` when no key does. */
function keyEnd(q: string, from: number): number {
  if (from >= q.length || !isKeyStart(q[from])) return from;
  let j = from + 1;
  while (j < q.length && isKeyChar(q[j])) j += 1;
  return j;
}

/** Index just past the `:` of a `key:` that starts at `i`, or -1 when none does. */
function fieldPrefixEnd(q: string, i: number): number {
  const end = keyEnd(q, i);
  return end > i && q[end] === ":" ? end + 1 : -1;
}

/** Whether `key:` would parse as a field. A facet or a filter button may only offer one that does. */
export function isQueryableKey(key: string): boolean {
  return key !== "" && keyEnd(key, 0) === key.length;
}

const isSpace = (ch: string) => /\s/.test(ch);

const NUMBER = /^[+-]?(\d+(\.\d*)?|\.\d+)([eE][+-]?\d+)?$/;
/**
 * Go spells micro with U+00B5 or U+03BC, and a logger can use either. The
 * order matters to `readMeasure`: `ms` has to come before `m`, which it starts
 * with, so `Object.keys` (insertion order for these keys) is also the search order.
 */
const UNIT_SECONDS: Readonly<Record<string, number>> = {
  ns: 1e-9,
  us: 1e-6,
  "µs": 1e-6,
  "μs": 1e-6,
  ms: 1e-3,
  s: 1,
  m: 60,
  h: 3600,
};
const UNITS = Object.keys(UNIT_SECONDS);

const isDigit = (ch: string) => ch >= "0" && ch <= "9";

/** Index just past the run of digits that starts at `from`. */
function digitsEnd(s: string, from: number): number {
  let j = from;
  while (j < s.length && isDigit(s[j])) j += 1;
  return j;
}

/**
 * Index just past the `12`, `1.5`, `1.` or `.5` that starts at `from`, or `from`
 * when no number does. Scanned by index, not by a regex, which CodeQL reads as
 * polynomial on a long run of digits (js/polynomial-redos, as for `keyEnd`);
 * `readMeasure` is exported and fed every line's field values.
 */
function partEnd(s: string, from: number): number {
  const whole = digitsEnd(s, from);
  if (s[whole] !== ".") return whole;
  const end = digitsEnd(s, whole + 1);
  return whole > from || end > whole + 1 ? end : from;
}

/**
 * A number, or a Go-style duration in seconds (`350ms`, `1h2m3s`), or undefined
 * when `s` is neither. The query's right side and a line's field value are read
 * by this one function, so they cannot disagree about what a measure is.
 */
export function readMeasure(s: string): { kind: "number" | "duration"; value: number } | undefined {
  if (NUMBER.test(s)) return { kind: "number", value: Number(s) };
  let i = s[0] === "-" || s[0] === "+" ? 1 : 0;
  const sign = s[0] === "-" ? -1 : 1;
  if (i >= s.length) return undefined;
  let total = 0;
  while (i < s.length) {
    const end = partEnd(s, i);
    const unit = end > i ? UNITS.find((u) => s.startsWith(u, end)) : undefined;
    if (unit === undefined) return undefined;
    total += Number(s.slice(i, end)) * UNIT_SECONDS[unit];
    i = end + unit.length;
  }
  return { kind: "duration", value: sign * total };
}

/**
 * `key>v`, `key>=v`, `key<v` or `key<=v` at `i`, when `v` reads as a number or a
 * duration; otherwise undefined and the token stays text, so `a->b` and `x>y`
 * still search for what they say.
 */
function readComparison(q: string, i: number) {
  const keyStop = keyEnd(q, i);
  if (keyStop === i || (q[keyStop] !== ">" && q[keyStop] !== "<")) return undefined;
  const op = (q[keyStop + 1] === "=" ? `${q[keyStop]}=` : q[keyStop]) as CompareOp;
  const from = keyStop + op.length;
  let end = from;
  while (end < q.length && !isSpace(q[end])) end += 1;
  const measure = readMeasure(q.slice(from, end));
  if (measure === undefined) return undefined;
  return { field: q.slice(i, keyStop), op, kind: measure.kind, target: measure.value, value: q.slice(from, end), end };
}

type Term =
  | { readonly kind: "bare" | "quoted" | "regex"; readonly value: string; readonly end: number }
  | { readonly error: string; readonly at: number };

/**
 * Whitespace-separated clauses: `word`, `"phrase"`, `/regex/`, `field:value`,
 * `field:"value"`, `field:/regex/`, `field>5`, `field<=1.5s`, each optionally
 * `-`-negated. Values are lowercased here once (a regex is made
 * case-insensitive instead), so matching 5,000 lines does no case folding of
 * the query. Field names keep their spelling and compare case-insensitively at
 * match time.
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
    const cmp = readComparison(query, i);
    if (cmp !== undefined) {
      clauses.push({
        match: "compare",
        field: cmp.field,
        op: cmp.op,
        kind: cmp.kind,
        target: cmp.target,
        value: cmp.value,
        negated,
        span: [start, cmp.end],
      });
      i = cmp.end;
      continue;
    }
    const prefixEnd = fieldPrefixEnd(query, i);
    const field = prefixEnd < 0 ? undefined : query.slice(i, prefixEnd - 1);
    if (prefixEnd >= 0) i = prefixEnd;
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
 * clicks on the Level facet show both levels — and everything else ANDs,
 * comparisons included, so `latency>100 latency<500` is a range.
 * `ignoreField` skips that field's clauses, which is how a facet counts its
 * own values without filtering itself down to the one already picked. A
 * comparison is no pick, it is a bound, so it is never skipped: under
 * `status>=500` the status facet must not offer a `200` that would show nothing.
 */
export function matchLine(line: QueryLine, clauses: readonly QueryClause[], ignoreField?: string): boolean {
  const ignored = ignoreField?.toLowerCase();
  let anyOf: Map<string, boolean> | undefined;
  for (const clause of clauses) {
    const field = clause.field?.toLowerCase();
    if (field !== undefined && field === ignored && clause.match !== "compare") continue;
    const hit = hits(line, clause);
    if (clause.negated) {
      if (hit) return false;
    } else if (field === undefined || clause.match === "compare") {
      // A comparison narrows, so two on one field are a range and not an either-or.
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
  if (clause.match === "compare") {
    // Like compares with like: `5` is no answer to `took>1s`, and `30.1s` none to `took>5`.
    const raw = fieldValue(line, clause.field);
    const got = raw === undefined ? undefined : readMeasure(raw.trim());
    if (got === undefined || got.kind !== clause.kind) return false;
    switch (clause.op) {
      case ">":
        return got.value > clause.target;
      case ">=":
        return got.value >= clause.target;
      case "<":
        return got.value < clause.target;
      case "<=":
        return got.value <= clause.target;
    }
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
 *
 * A target typed twice (`level:error level:error`) is one filter to the facet,
 * so a click acts on every copy: the first one decides the mode it is read in,
 * and when the click flips it the rest go, or the facet would stay lit.
 */
export function toggleClause(query: string, target: ClauseTarget, mode: ClauseMode): string {
  const parsed = parseLogQuery(query);
  const found = parsed.ok ? parsed.clauses.filter((c) => sameTarget(c, target)) : [];
  const spelled = mode === "exclude" ? `-${spell(target)}` : spell(target);
  if (found.length === 0) return append(query, spelled);
  const [first, ...rest] = found;
  // From the end backwards, so the spans still to come are where they were parsed.
  const flip = (mode === "exclude") !== first.negated;
  const without = (flip ? rest : found).reduceRight((q, c) => removeSpan(q, c.span), query);
  return flip ? without.slice(0, first.span[0]) + spelled + without.slice(first.span[1]) : without;
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

/**
 * A value spelled so it parses back to itself. A quote cannot be quoted, and a
 * control character cannot be typed into the `<input type=search>` the query
 * lives in (it strips newlines), so each becomes an exact regex in which the
 * control character is escaped.
 */
function spellValue(value: string): string {
  if (value.includes('"') || CONTROL.test(value)) return `/^${escapeRegex(value)}$/`;
  if (value === "" || /[\s:]/.test(value) || value.startsWith("/")) return `"${value}"`;
  return value;
}

const CONTROL = /[\u0000-\u001f]/;

/** A regex source that matches `text` literally. A control character is written escaped, so no source holds a raw newline. */
function escapeRegex(text: string): string {
  return text
    .replace(/[.*+?^${}()|[\]\\/]/g, "\\$&")
    .replace(/[\u0000-\u001f]/g, (c) =>
      c === "\n" ? "\\n" : c === "\r" ? "\\r" : `\\x${c.charCodeAt(0).toString(16).padStart(2, "0")}`,
    );
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
  // Keyed by lowercase, as the query matches field names. The first spelling seen is the one offered.
  const seen = new Map<string, { spelling: string; lines: number; values: Set<string> }>();
  for (const line of lines) {
    const keys = Object.keys(line.fields);
    if (keys.length === 0) continue;
    structured += 1;
    // A row carrying both `System` and `system` is still one row of coverage.
    const counted = new Set<string>();
    for (const key of keys) {
      if (HIDDEN_KEYS.has(key.toLowerCase()) || !isQueryableKey(key)) continue;
      const lower = key.toLowerCase();
      let entry = seen.get(lower);
      if (entry === undefined) {
        entry = { spelling: key, lines: 0, values: new Set() };
        seen.set(lower, entry);
      }
      if (!counted.has(lower)) {
        counted.add(lower);
        entry.lines += 1;
      }
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
    .map(([, e]) => e.spelling);
}

/**
 * A field's values among the lines every OTHER clause keeps, most frequent
 * first. Grouped case-insensitively, because that is how they match, and
 * named by the first spelling seen. Ties sort on the lowercased value, so the
 * order does not depend on which spelling arrived first.
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
  return [...counts.values()].sort(
    (a, b) => b.count - a.count || compare(a.value.toLowerCase(), b.value.toLowerCase()),
  );
}

function compare(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

export interface VolumeBucket {
  /** Epoch ms of the bucket's start. A sub-hour bar is exactly `step` long; a bar of an hour or more can differ from `step` across a DST change. */
  readonly start: number;
  readonly end: number;
  readonly danger: number;
  readonly warning: number;
  /** Lines core reads as info (`info`, `notice`): the common case, kept apart so `other` is only what has no severity. */
  readonly info: number;
  /** Neutral and success lines: no level word, or one core doesn't tone (`debug`, `trace`). */
  readonly other: number;
}

export interface Volume {
  /** Bar width in ms; 0 when no line had a time. A local-calendar bar can be longer or shorter across DST. */
  readonly step: number;
  /** Contiguous, empty ones included, so a quiet minute is drawn as a gap. */
  readonly buckets: readonly VolumeBucket[];
  readonly untimed: number;
}

const SECOND = 1000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
/** The widest instant a Date can hold (ECMAScript's limit, ±8.64e15 ms). */
const MAX_TIME = 8.64e15;
/** The round bar widths, narrowest first. A time axis cuts its ticks on the same ones. */
export const VOLUME_STEPS: readonly number[] = [
  ...[1, 2, 5, 10, 15, 30].map((n) => n * SECOND),
  ...[1, 2, 5, 10, 15, 30].map((n) => n * MINUTE),
  ...[1, 2, 3, 6, 12].map((n) => n * HOUR),
  DAY,
];

/** The UTC offset in ms at `at`. Clamped into the Date range, so an edge stamp never reads NaN. */
const offsetAt = (at: number): number =>
  new Date(Math.max(-MAX_TIME, Math.min(MAX_TIME, at))).getTimezoneOffset() * MINUTE;

/** A stamp the histogram can place. NaN, infinite and out-of-range stamps are untimed. */
const isTimed = (at: number | undefined): at is number =>
  at !== undefined && Number.isFinite(at) && Math.abs(at) <= MAX_TIME;

/** A stamp's wall-clock reading. Taken per line, because the offset changes across DST. */
export const wallClock = (at: number): number => at - offsetAt(at);

/**
 * The instant whose wall clock reads `local`. Of the two candidates (the offset
 * before, the offset after), keep those that really read `local`. In a repeated
 * hour both do, and the earlier is taken, so the bar starts at the first copy
 * and spans both. In a skipped hour neither does, and the later candidate is the
 * transition itself, so the bar is empty and never overlaps its neighbour. Each
 * offset is read a day either side: a real instant can sit 14 hours from its wall
 * reading (UTC+14), so half a day reads the wrong side of a change above UTC+12.
 * Exported for the time axis, which cuts its ticks on the same local clock.
 */
export const instantOf = (local: number): number => {
  const before = local + offsetAt(local - DAY);
  const after = local + offsetAt(local + DAY);
  const reads = [before, after].filter((c) => wallClock(c) === local);
  return reads.length > 0 ? Math.min(...reads) : Math.max(before, after);
};

/**
 * The lines' volume over time, stacked by core's severity, in the smallest round
 * step that fits the span in `maxBars`. Steps under an hour cut bars on absolute
 * time from the earliest line's offset: one fixed grid, so a bar never inverts and
 * every line sits inside its bar. Steps of an hour or more cut bars on the local
 * calendar, so an hour bar starts at :00 where the reader is and a day bar at local
 * midnight, across DST. When no round step fits, the step is the fewest whole days
 * that fit, so the bucket count stays at or below `maxBars`, except for `maxBars`
 * of 1 with a span straddling local 1970-01-01.
 */
export function volumeBuckets(lines: readonly QueryLine[], maxBars = 60): Volume {
  let minAt = Infinity;
  let maxAt = -Infinity;
  let min = Infinity;
  let max = -Infinity;
  let untimed = 0;
  for (const line of lines) {
    if (!isTimed(line.at)) {
      untimed += 1;
      continue;
    }
    if (line.at < minAt) minAt = line.at;
    if (line.at > maxAt) maxAt = line.at;
    const local = wallClock(line.at);
    if (local < min) min = local;
    if (local > max) max = local;
  }
  if (minAt === Infinity) return { step: 0, buckets: [], untimed };

  const bars = Number.isFinite(maxBars) ? Math.max(1, Math.floor(maxBars)) : 1;
  // A sub-hour step counts absolute time from one origin; a longer step counts the wall clock.
  const origin = offsetAt(minAt);
  const lo = (s: number) => (s < HOUR ? minAt - origin : min);
  const hi = (s: number) => (s < HOUR ? maxAt - origin : max);
  const fits = (s: number) => Math.floor(hi(s) / s) - Math.floor(lo(s) / s) + 1 <= bars;
  const wholeDays = (): number => {
    // Fewer than span / (bars * DAY) days cannot fit, so the search starts there.
    // Past `cap` the count no longer changes with the step.
    const cap = Math.ceil((Math.abs(min) + Math.abs(max)) / DAY) + 1;
    let days = Math.max(1, Math.floor((max - min) / (bars * DAY)));
    while (days < cap && !fits(days * DAY)) days += 1;
    return days * DAY;
  };
  const step = VOLUME_STEPS.find(fits) ?? wholeDays();
  const bucketOf = (at: number): number => Math.floor((step < HOUR ? at - origin : wallClock(at)) / step);
  const edgeAt = (k: number): number => (step < HOUR ? k * step + origin : instantOf(k * step));
  const first = Math.floor(lo(step) / step);
  const last = Math.floor(hi(step) / step);
  const edges = Array.from({ length: last - first + 2 }, (_, i) => edgeAt(first + i));
  const buckets = Array.from({ length: last - first + 1 }, (_, i) => ({
    start: edges[i],
    end: edges[i + 1],
    danger: 0,
    warning: 0,
    info: 0,
    other: 0,
  }));
  for (const line of lines) {
    if (!isTimed(line.at)) continue;
    const bucket = buckets[bucketOf(line.at) - first];
    if (line.health === "danger") bucket.danger += 1;
    else if (line.health === "warning") bucket.warning += 1;
    else if (line.health === "info") bucket.info += 1;
    else bucket.other += 1;
  }
  return { step, buckets, untimed };
}

/** A window picked off the histogram: `[from, to)`, epoch ms. */
export interface TimeRange {
  readonly from: number;
  readonly to: number;
}

/** A line with no usable time can't be placed in a window, so it is never inside one. */
export function inRange(line: QueryLine, range: TimeRange): boolean {
  return isTimed(line.at) && line.at >= range.from && line.at < range.to;
}

/**
 * Where a drawn row should mark what the query matched: `[start, end)`
 * pairs, sorted and merged. Built from the original text, not a lowercased
 * copy, because lowercasing can change a string's length and shift every
 * index after it. A negation matched nothing on a line still in view, and
 * `level` has a column of its own, so neither is marked. Field-regex clauses
 * (`f:/re/`, and the exact-regex form a quote-containing facet value is written
 * in) are not marked either: their regex tests the value, not the text a row
 * shows. A field clause marks its whole `key=value` token.
 *
 * A free-text regex is the user's own pattern, and one that backtracks
 * catastrophically would freeze the thread it runs on, so it is never run
 * here: its marks are worked out in the regex worker and come in as `extra`.
 */
export function highlightRanges(
  text: string,
  clauses: readonly QueryClause[],
  extra: readonly (readonly [number, number])[] = [],
): [number, number][] {
  const found: [number, number][] = extra.map(([start, end]) => [start, end]);
  for (const clause of clauses) {
    if (clause.negated) continue;
    const mark = highlightPattern(clause);
    if (mark === undefined) continue;
    const re = new RegExp(mark.pattern, "gi");
    for (let m = re.exec(text); m !== null; m = re.exec(text)) {
      if (m[0] === "") {
        re.lastIndex += 1;
        continue;
      }
      // A prefixed mark's group 1 is the character before the word; the mark leaves it out.
      const start = mark.prefixed ? m.index + m[1].length : m.index;
      found.push([start, m.index + m[0].length]);
    }
  }
  found.sort((a, b) => a[0] - b[0]);
  const merged: [number, number][] = [];
  for (const [start, end] of found) {
    const last = merged[merged.length - 1];
    if (last !== undefined && start <= last[1]) last[1] = Math.max(last[1], end);
    else merged.push([start, end]);
  }
  return merged;
}

/**
 * What marks a clause: its pattern, and whether group 1 is a one-character
 * prefix the mark leaves out. The prefix does the work of a lookbehind, which
 * older WKWebView (Safari before 16.4) throws on when a RegExp is built, and the
 * Logs screen builds these while it renders.
 */
interface Mark {
  readonly pattern: string;
  readonly prefixed: boolean;
}

function highlightPattern(clause: QueryClause): Mark | undefined {
  // A regex is the user's, and is never run on the main thread (see `highlightRanges`).
  if (clause.match === "regex") return undefined;
  if (clause.field === undefined) {
    if (clause.match === "phrase") return phrasePattern(clause.value, clause.words);
    return { pattern: escapeRegex(clause.value), prefixed: false };
  }
  if (clause.match !== "equals" || clause.field.toLowerCase() === "level") return undefined;
  // klog and prose rows keep the writer's quoting, so a value with no space is
  // marked raw or quoted. A value with a space is only ever quoted: raw
  // `key=a b` is text inlineFields never writes. The key may not continue an
  // earlier key name, and the value must end its token.
  const quoted = escapeRegex(JSON.stringify(clause.value));
  const shown = /\s/.test(clause.value) ? quoted : `(?:${escapeRegex(clause.value)}|${quoted})`;
  return { pattern: `(^|[^\\w.@/-])${escapeRegex(clause.field)}=${shown}(?=\\s|$)`, prefixed: true };
}

/** A character class of `chars`, each one escaped so none of them is read as syntax. */
function charClass(chars: string): string {
  return `[${[...chars].map((c) => c.replace(/[\\\]\[^-]/g, "\\$&")).join("")}]`;
}

const QUOTE = charClass(QUOTE_CHARS);
const TRAILING = charClass(TRAILING_CHARS);

/**
 * A phrase as `matchLine` accepts it: its literal text, or its words in order
 * as `tokenize` reads them. The mark starts at a token's first word, or at the
 * quote the writer put before it, and stops before the punctuation that ends
 * the token: `timeout:` marks `timeout`, and `wrote batch` does not mark the
 * front of `wrote batches`.
 */
function phrasePattern(value: string, words: string): Mark {
  const alternatives = [escapeRegex(value)];
  if (words !== "") alternatives.push(words.split(" ").map(escapeRegex).join(`${TRAILING}*\\s+${QUOTE}*`));
  // The trailing lookahead consumes nothing, so a phrase right after this one can use the space as its prefix.
  return {
    pattern: `(^|\\s)${QUOTE}*(?:${alternatives.join("|")})(?=${TRAILING}*(?:\\s|$))`,
    prefixed: true,
  };
}
