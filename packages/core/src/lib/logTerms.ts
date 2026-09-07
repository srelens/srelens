import type { LogLine } from "./logBuffer";
import type { HealthKind } from "./k8sHealth";

/**
 * The rail's recurring-term tally: what a log stream keeps saying, most
 * frequent first, each toned by how bad the lines behind it are.
 *
 * ## The normalisation rule, and why
 *
 * Every log line is unique once you look at its whole text — a timestamp, a
 * trace id, a duration, a request path all vary line to line even when the
 * *event* repeats. Tallying whole lines therefore yields a list of ones,
 * which is the failure mode this file exists to avoid.
 *
 * ### Framing first: skip it, do not stop on it
 *
 * A real log line does not open with its message. It opens with its framing —
 * `2026/08/25 07:53:02 [notice] 1#1:` in front of nginx's `start worker
 * process`, `I0825 08:23:00.651626       1 cidrallocator.go:278]` in front of
 * every line klog has ever written. The first version of this rule read a
 * leading digit as data and ENDED the term there, which meant the most
 * repeated line in an nginx pod contributed nothing at all and a klog buffer
 * tallied to `I0825` — a severity letter and a month-day, identical on every
 * line of every control-plane component. That is the log's framing, not its
 * message.
 *
 * So {@link STRUCTURAL_PREFIXES} is **skipped** rather than terminating the
 * run: a stamp, a klog header, an nginx `pid#tid:`, a bracketed level, a
 * leading level word. This is the same principle the old rule already applied
 * to one case — a bare leading `error`/`warn` was skipped as structural — with
 * the rest of the framing a real cluster emits brought under it.
 *
 * **Leading only.** A numeric token in the MIDDLE of a line still ends the
 * term, and that is the whole of what keeps `duration=30011ms` and an nginx
 * worker's pid out of the tally. "Skip when leading, terminate when internal"
 * is the distinction; a line whose *message* opens with a number (`503 errors
 * spiking`) still contributes nothing, because nothing about it is stable.
 *
 * ### A JSON line tallies its message
 *
 * Structured logging is everywhere, and a JSON line is one whitespace-free
 * token that is unique per line — its own timestamp sees to that — so a
 * buffer of them used to tally to nothing. If a line parses as a JSON OBJECT
 * carrying a `msg` or `message` string, that string is the line as far as
 * this file is concerned. Merely opening with `{` proves nothing, and a parse
 * failure is not an error here: {@link logObject} returns `null` and the raw
 * text is used, exactly as before.
 *
 * ### The key=value cut, unchanged
 *
 * `status=503` and `duration=30011ms` are shaped identically — a word, an
 * `=`, and something that starts with a digit — yet one is the rail's
 * strongest signal and the other is noise. Nothing about either token's
 * *shape* tells them apart. What does is how each behaves **across the
 * buffer**: `status=503` is the literal same six characters on every failed
 * request in this window, while `duration=` and `trace_id=` carry a
 * different value practically every time. That is cardinality, and it is the
 * one thing a shape-based regex cannot see but a pass over the buffer can.
 *
 * So: **a `key=value` token is trusted, and used whole as the term, once it
 * has been seen recurring — the exact same literal pair, twice or more —
 * anywhere in the buffer.** An unrepeated pair ends the run without being
 * included, because a value seen once carries no more meaning here than a raw
 * timestamp would. A trusted pair wins outright and becomes the term by
 * itself: `request failed status=503 …` yields `status=503`, because the code
 * is the more specific, more diagnostic fact and there is no principled way
 * to keep both without the rail scrolling.
 *
 * ### How long a term is: the clause, if the clause recurs
 *
 * The old rule capped the term at two words, which is `pool timeout` but also
 * `start worker` (nginx says `start worker process`) and `cannot reach` (the
 * pod said `cannot reach mainframe gateway`). Two is arbitrary, and the
 * buffer can answer the question itself.
 *
 * The run is the line's opening clause: bare tokens from the start of the
 * message, ending at a number, an untrusted pair, a comma or a full stop —
 * the punctuation the writer themselves put where the headline ends. **If
 * that whole clause recurs {@link MIN_RECURRENCES} times and fits the rail,
 * the whole clause is the term**; otherwise it is trimmed back to the
 * {@link HEADLINE_WORDS}-word headline. Self-tuning, and it costs one more
 * pass over the buffer: `start worker process` survives whole because those
 * three words are the same three words 44 times over, while `liveness
 * deadline extended for the drain` collapses to `liveness deadline` the
 * moment the next line says `… for the shutdown` instead.
 *
 * A sliding version of the same idea — the longest *prefix* that recurs,
 * trimmed word by word — was tried first and is not what shipped: it turns
 * `pool saturated, queueing request depth=…` into `pool saturated queueing
 * request` and cuts long messages mid-phrase (`updated ClusterIP allocator
 * for Service`), because a prefix that fits the rail almost never lands on a
 * phrase boundary. All-or-nothing at the clause is the same self-tuning
 * behaviour without the ragged edges.
 *
 * {@link MAX_TERM_CHARS} is the rail's own width talking: the terms sit in a
 * 272px column, so a 90-character term is useless even when it is accurate.
 *
 * ## Tone
 *
 * "Every status word and tone comes from core" (plan constraint) — so this
 * file does not invent a colour vocabulary. It reuses `HealthKind`
 * ({@link "./k8sHealth"}), core's one canonical severity vocabulary, rather
 * than adding a sibling.
 *
 * A term's lines are not all the same severity — the same `status=503`
 * fires from a request logged at `warn` on retry and `error` on final
 * failure. **Worst wins**: the same rule the cluster overview already uses
 * when one fact is read more than once (see the note on `worst()` beside
 * `StatusVerdict` in `k8sStatus.ts`) — a term is exactly as alarming as its
 * single worst occurrence, because that is the line the reader most needs
 * the colour to point at.
 */
const HEADLINE_WORDS = 2;

/**
 * A tally is only useful once something has actually recurred; a term (or a
 * `key=value` token, or a whole clause) seen once is, by definition, not
 * recurring, and showing it turns the rail back into the wall-of-noise it
 * exists to summarise. Two occurrences is the lowest bar that still means
 * "recurred".
 */
const MIN_RECURRENCES = 2;

/** How many rows the rail shows by default — enough breadth, not a scroll. */
const DEFAULT_CAP = 8;

/**
 * The widest term the rail can actually show. `STREAM_RAIL_WIDTH` is 272px
 * and a term row spends part of that on a status dot and a right-aligned
 * count, so a term much past forty characters is drawn as an ellipsis — and
 * two terms that differ only after the ellipsis are one useless row. Terms
 * are trimmed at a word boundary rather than mid-word.
 */
const MAX_TERM_CHARS = 40;

/** A leading digit means the token is a number, a duration, or an id. */
const OPENS_WITH_DIGIT = /^-?\d/;

/**
 * Every level word this file recognises anywhere — the kit's `LEVEL_TONE`
 * vocabulary (`packages/ui-kit/src/LogLine.tsx`) plus the syslog spellings an
 * application can declare. Longest alternatives first so `error` is never
 * matched as `err`.
 */
const LEVEL_WORDS =
  "fatal|dpanic|panic|critical|crit|alert|emergency|emerg|error|err|warning|warn|notice|info|debug|trace";

/**
 * The framing a real log line opens with, in the order it is peeled off.
 * Applied to the START of the message only, repeatedly, until nothing
 * matches — nginx puts four of these in front of one sentence.
 *
 * Each is deliberately shaped, never "anything numeric": a message that
 * genuinely opens with a figure (`503 errors spiking`) has to keep
 * contributing nothing, which is what stops the tally inventing rows out of
 * per-line data.
 */
const STRUCTURAL_PREFIXES: readonly RegExp[] = [
  // A quote around the whole message — klog wraps its structured warnings.
  /^["'`]+/,
  // klog: `I0825 08:23:00.651626       1 cidrallocator.go:278]`.
  /^[IWEF]\d{4}\s+\d{1,2}:\d{2}:\d{2}(?:\.\d+)?\s+\d+\s+\S+:\d+\]\s*/,
  // RFC3339 / ISO, with or without a zone.
  /^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?\s*/,
  // nginx and friends: `2026/08/25 07:53:02`.
  /^\d{4}\/\d{2}\/\d{2}(?:\s+\d{1,2}:\d{2}:\d{2}(?:[.,]\d+)?)?\s*/,
  // A bare wall clock, which is what a stripped stream line can start with.
  /^\d{1,2}:\d{2}:\d{2}(?:[.,]\d+)?\s*/,
  // nginx's `1#1:` — worker pid and thread id, the same on every line.
  /^\d+#\d+:\s*/,
  // A bracketed level: CoreDNS's `[ERROR]`, nginx's `[notice]`. The level
  // leaves the text and stays in the tone, which is the channel the rail
  // already draws it in — and skipping it is what gets past `[notice]` to
  // nginx's actual sentence.
  new RegExp(`^\\[(?:${LEVEL_WORDS})\\]\\s*`, "i"),
  // A bare leading level word, `error pool timeout` / `FATAL cannot reach`.
  new RegExp(`^(?:${LEVEL_WORDS})\\b[:,]?\\s+`, "i"),
];

/** Words classic already keys the danger tone off, verbatim. */
const DANGER_WORD = /\b(?:error|fatal|panic)\b/i;
/** ditto, the warning tone. */
const WARNING_WORD = /\bwarn(?:ing)?\b/i;
/** ditto, the info tone. */
const INFO_WORD = /\binfo\b/i;
/**
 * Recognised but untoned: `logLineHealth` has never coloured these, and this
 * scan does not start now — see {@link logLineLevel}'s doc for why they are
 * still worth returning to a caller that wants the level column's word.
 */
const DEBUG_TRACE_WORD = /\b(?:debug|trace)\b/i;

/** A parsed JSON log line. `unknown` values: the shape is the app's, not ours. */
type LogObject = Readonly<Record<string, unknown>>;

/** The fields a structured logger states its own severity in. */
const LEVEL_FIELDS = ["level", "severity"] as const;
/** The fields a structured logger puts the human sentence in. */
const MESSAGE_FIELDS = ["msg", "message"] as const;

/**
 * The line as a JSON object, or `null` for the overwhelming majority of log
 * lines that are not one.
 *
 * Opening with `{` proves nothing — `{not json at all` opens with `{` — so
 * this parses and lets the parse decide, and a failure is a `null` rather
 * than a throw: a malformed line is a normal thing for a log to contain and
 * must not take the rail down with it. Arrays and bare scalars parse fine and
 * are still not log objects, so they are rejected too.
 *
 * A stamped line is tried a second time with its framing off, so a JSON
 * logger behind a k8s timestamp is still read as JSON.
 */
function logObject(text: string): LogObject | null {
  return parseObject(text) ?? parseObject(skipStructure(text));
}

function parseObject(text: string): LogObject | null {
  const trimmed = text.trim();
  if (!trimmed.startsWith("{") || !trimmed.endsWith("}")) return null;
  try {
    const parsed: unknown = JSON.parse(trimmed);
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return null;
    return parsed as LogObject;
  } catch {
    return null;
  }
}

/** The first of `fields` the object carries as a non-empty string. */
function stringField(obj: LogObject, fields: readonly string[]): string | undefined {
  for (const field of fields) {
    const value = obj[field];
    if (typeof value === "string" && value.trim() !== "") return value.trim();
  }
  return undefined;
}

/** Peel every structural prefix off the front of a line; see {@link STRUCTURAL_PREFIXES}. */
function skipStructure(text: string): string {
  let rest = text.trim();
  // Bounded rather than `while (true)`: every pass must shorten the string, so
  // this can only spin if a pattern ever matches empty. It cannot, but a log
  // line is untrusted input and an infinite loop in it would freeze the rail.
  for (let pass = 0; pass < STRUCTURAL_PREFIXES.length; pass += 1) {
    const before = rest;
    for (const prefix of STRUCTURAL_PREFIXES) {
      rest = rest.replace(prefix, "").trimStart();
    }
    if (rest === before) break;
  }
  return rest;
}

/**
 * What the tally reads: a JSON logger's own `msg`, or the raw line — either
 * way with the framing skipped. This is "the human-meaningful part of the
 * line", and everything below counts words in it rather than in the text the
 * stream happened to deliver.
 */
function messageOf(text: string, obj: LogObject | null): string {
  const declared = obj === null ? undefined : stringField(obj, MESSAGE_FIELDS);
  return skipStructure(declared ?? text);
}

/**
 * One word of a message, and whether the writer ended their clause on it.
 *
 * The clause boundary is the writer's own punctuation — `pool saturated,
 * queueing request` is a headline and an aside, and the comma says so. A
 * colon is NOT a boundary: `plugin/kubernetes: Failed to watch` is one
 * thought, and CoreDNS would otherwise tally to its component name alone.
 */
export interface Token {
  readonly text: string;
  readonly endsClause: boolean;
}

/** The characters a writer's own quoting puts around a word. */
const QUOTE_CHARS = "\"'`";
/** Those, plus the punctuation that ends a word rather than belonging to it. */
const TRAILING_CHARS = "\"'`.,;:!?";
/** The punctuation that closes a clause. No colon — see {@link Token}. */
const CLAUSE_END_CHARS = ",;.!?";

/** The index of the first character of `token` that is not in `chars`. */
function firstKept(token: string, chars: string): number {
  let start = 0;
  while (start < token.length && chars.includes(token[start])) start += 1;
  return start;
}

/** The index one past the last character of `token` that is not in `chars`. */
function lastKept(token: string, chars: string): number {
  let end = token.length;
  while (end > 0 && chars.includes(token[end - 1])) end -= 1;
  return end;
}

/**
 * Whitespace-split tokens, quotes and trailing sentence punctuation stripped.
 *
 * **The three strips are index walks, not regexes.** They were repetition with
 * a `$` anchor, which the engine re-tries from every start position in the
 * token: a run of quotes cost 709ms at 20k characters, 2.7s at 40k and 11.1s
 * at 80k — each, and all three ran over every token (js/polynomial-redos,
 * #380). These lines are STREAMED from the cluster, so this is the least
 * controlled input in the app and the only one that arrives continuously. One
 * quote-run token in one line was seconds of frozen UI thread.
 *
 * {@link firstKept}/{@link lastKept} walk each end once, which is linear, and
 * answer the same three questions the patterns did — see `tokenize`'s own
 * table in the tests, every row of which was taken off the regex version.
 */
export function tokenize(text: string): Token[] {
  const tokens: Token[] = [];
  for (const raw of text.trim().split(/\s+/)) {
    // The clause end is read past the writer's closing quote but through
    // nothing else: `saturated,"` ends a clause, `kubernetes:` does not.
    const unquoted = lastKept(raw, QUOTE_CHARS);
    const endsClause = unquoted > 0 && CLAUSE_END_CHARS.includes(raw[unquoted - 1]);
    const start = firstKept(raw, QUOTE_CHARS);
    const end = lastKept(raw, TRAILING_CHARS);
    // `end <= start` is a token of nothing but quotes and punctuation, which
    // the two chained `replace`s also reduced to the empty string.
    const stripped = end > start ? raw.slice(start, end) : "";
    if (stripped.length > 0) tokens.push({ text: stripped, endsClause });
  }
  return tokens;
}

function isBareNumeric(token: string): boolean {
  return OPENS_WITH_DIGIT.test(token);
}

/** Splits a `key=value` token, or returns `null` for anything else. */
function splitKeyValue(token: string): { key: string; value: string } | null {
  const eq = token.indexOf("=");
  if (eq <= 0) return null;
  return { key: token.slice(0, eq), value: token.slice(eq + 1) };
}

/** What one line offers the tally: a trusted pair, or its opening clause. */
interface Candidate {
  /** A `key=value` token seen recurring; it wins outright. */
  readonly trusted: string | null;
  /** The line's opening clause, word by word. */
  readonly clause: readonly string[];
}

/**
 * One line's candidate term: the first `key=value` token that has recurred
 * (trusted) anywhere in the buffer, or else the run of bare words from the
 * start of the message to the first number, untrusted pair, or clause-ending
 * punctuation. See the module doc for the reasoning.
 */
function candidateOf(
  tokens: readonly Token[],
  kvFrequency: ReadonlyMap<string, number>,
): Candidate {
  const clause: string[] = [];
  for (const token of tokens) {
    if (splitKeyValue(token.text) !== null) {
      if ((kvFrequency.get(token.text) ?? 0) >= MIN_RECURRENCES) {
        return { trusted: token.text, clause };
      }
      break; // an unrepeated pair is where the varying detail starts
    }
    if (isBareNumeric(token.text)) break; // mid-line, a number is data
    clause.push(token.text);
    if (token.endsClause) break;
  }
  return { trusted: null, clause };
}

/** The first {@link HEADLINE_WORDS} words, and never wider than the rail. */
function headline(clause: readonly string[]): string {
  let term = clause[0];
  for (let i = 1; i < Math.min(clause.length, HEADLINE_WORDS); i += 1) {
    const wider = `${term} ${clause[i]}`;
    if (wider.length > MAX_TERM_CHARS) break;
    term = wider;
  }
  return term;
}

/**
 * The clause if the whole clause recurred and fits the rail, else its
 * headline. The buffer decides the length, so `start worker process` keeps
 * its third word and `liveness deadline extended for the drain` does not keep
 * its last four.
 */
function termOf(
  clause: readonly string[],
  clauseFrequency: ReadonlyMap<string, number>,
): string | null {
  if (clause.length === 0) return null;
  const whole = clause.join(" ");
  if (whole.length <= MAX_TERM_CHARS && (clauseFrequency.get(whole) ?? 0) >= MIN_RECURRENCES) {
    return whole;
  }
  return headline(clause);
}

/**
 * A raw log line's level word, exactly as the line spelled it — `"error"`,
 * `"WARNING"`, `"warn"`, `"info"`, `"debug"`, `"trace"` — or `undefined` when
 * the line carries none. The ONE place in srelens that scans a line for this;
 * `logLineHealth` below and the Logs screen's level column both read it
 * through here rather than running their own regex over the same text.
 *
 * **A line that declares its own level is believed.** A JSON logger writes
 * `{"level":"warn",…}`; the application has stated the answer and there is
 * nothing to guess. Guessing was a real bug: `{"level":"warn",…,"error":{},…}`
 * rendered ERROR, because `\berror\b` matches a JSON field NAME — a quote is
 * a non-word character — so an `error` key holding an empty object repainted
 * the whole line. Same family as `informant` reading as `info`: a word scan
 * reading a word in the wrong context. `level` and `severity` are read, in
 * that order, and only as strings: pino writes `"level":30`, which is no word
 * to print, so a numeric level falls through to the scan below.
 *
 * The declared word is returned VERBATIM, so the column prints what the
 * application called it. A level outside {@link LEVEL_HEALTH} — an
 * application's own `audit`, say — is still returned and still reads
 * `neutral`: the word is theirs to print, but inventing a severity for a
 * vocabulary we do not know is exactly the guess this change removes. (Such a
 * word is also outside the kit's `LEVEL_TONE`, so the level column draws it
 * muted; closing that is the kit's to do, as it was for `panic` in 277924e.)
 *
 * Everything below the declared level is unchanged, because every
 * unstructured log in the world depends on it. It is a **text-scan heuristic,
 * not a parsed field** — `LogLine` (`./logBuffer`) carries only
 * `{ source, text }`, so this scans the whole raw line, case-insensitively,
 * for a recognised level word — the same vocabulary classic's `lineLevel`
 * already keys off (`apps/desktop/src/components/LogsView.tsx:61`), plus
 * `debug`/`trace`, which classic never needed a colour for but a level column
 * still has room to print.
 *
 * Checked worst-first (danger family, then warning, then info, then
 * debug/trace) so a line that somehow carries more than one recognised word
 * — "escalated to error after a warn" — returns the word that matters more,
 * not whichever regex happened to match first.
 *
 * **Returns the literal word, not `logLineHealth`'s tone name** — the level
 * column wants "error", and printing `logLineHealth`'s "danger" there was
 * the bug this function exists to fix.
 */
export function logLineLevel(text: string): string | undefined {
  return levelOf(text, logObject(text));
}

function levelOf(text: string, obj: LogObject | null): string | undefined {
  const declared = obj === null ? undefined : stringField(obj, LEVEL_FIELDS);
  if (declared !== undefined) return declared;
  return (
    DANGER_WORD.exec(text)?.[0] ??
    WARNING_WORD.exec(text)?.[0] ??
    INFO_WORD.exec(text)?.[0] ??
    DEBUG_TRACE_WORD.exec(text)?.[0] ??
    undefined
  );
}

/**
 * {@link logLineLevel}'s words mapped to their tone — the scan's own
 * vocabulary, plus the syslog spellings only a declared level can produce
 * (`err`, `crit`, `emerg`, `notice`). A real level landing on `neutral`
 * because this table forgot it is the bug 277924e fixed for `panic`, so the
 * table covers everything {@link LEVEL_WORDS} recognises; `debug` and `trace`
 * are absent on purpose, having never been toned.
 */
const LEVEL_HEALTH: Record<string, HealthKind> = {
  emergency: "danger",
  emerg: "danger",
  alert: "danger",
  critical: "danger",
  crit: "danger",
  fatal: "danger",
  panic: "danger",
  dpanic: "danger",
  error: "danger",
  err: "danger",
  warning: "warning",
  warn: "warning",
  notice: "info",
  info: "info",
};

/**
 * A raw log line's severity — the ONE place in srelens that decides this, on
 * core's canonical `HealthKind` vocabulary (`./k8sHealth`).
 *
 * Derived from {@link logLineLevel} rather than scanning the text itself:
 * there is exactly one rule for "what level word does this line carry", and
 * this is a second consumer of it, not a second copy of the regexes. A level
 * `logLineLevel` recognises but does not tone — `debug`, `trace` — reads
 * `neutral` here, same as no level word at all; every other caller (the term
 * tally below, and the Logs screen's `LogLine` level prop and level filter)
 * goes through here rather than re-deriving severity on its own, which is
 * exactly what the plan's "every status word and tone comes from core"
 * constraint is guarding against: a second hand-paired label/tone table,
 * invented at the call site.
 */
export function logLineHealth(text: string): HealthKind {
  return healthOf(levelOf(text, logObject(text)));
}

function healthOf(level: string | undefined): HealthKind {
  if (level === undefined) return "neutral";
  return LEVEL_HEALTH[level.toLowerCase()] ?? "neutral";
}

const HEALTH_RANK: Record<HealthKind, number> = {
  danger: 4,
  warning: 3,
  info: 2,
  success: 1,
  neutral: 0,
};

/** One recurring term the rail can show: its count, and its worst tone. */
export interface LogTerm {
  readonly term: string;
  readonly count: number;
  readonly tone: HealthKind;
}

/**
 * Tally the recurring terms across a buffer's lines, most frequent first,
 * dropping anything that only occurred once and capping the result so the
 * rail never has to scroll through dozens of rows.
 *
 * Tallies over `line.text` only — `line.source` (the pod/container tag) plays
 * no part, so the same message from three different pods still counts as one
 * term.
 */
export function tallyLogTerms(
  lines: readonly LogLine[],
  options?: { readonly cap?: number },
): LogTerm[] {
  const cap = options?.cap ?? DEFAULT_CAP;
  // Parsed once per line, not once per question: `logObject` is the only
  // expensive step here and three passes below want the same answer from it.
  const parsed = lines.map((l) => logObject(l.text));
  const tokenized = lines.map((l, i) => tokenize(messageOf(l.text, parsed[i])));

  // Pass 1: how many times has each literal key=value token recurred, over
  // the WHOLE buffer? This is the cardinality signal the rule is built on.
  const kvFrequency = new Map<string, number>();
  for (const tokens of tokenized) {
    for (const token of tokens) {
      if (splitKeyValue(token.text) !== null) {
        kvFrequency.set(token.text, (kvFrequency.get(token.text) ?? 0) + 1);
      }
    }
  }

  // Pass 2: each line's opening clause, and how often each WHOLE clause
  // recurred — which is what decides whether a term keeps its third word.
  const candidates = tokenized.map((tokens) => candidateOf(tokens, kvFrequency));
  const clauseFrequency = new Map<string, number>();
  for (const candidate of candidates) {
    if (candidate.trusted !== null || candidate.clause.length === 0) continue;
    const whole = candidate.clause.join(" ");
    clauseFrequency.set(whole, (clauseFrequency.get(whole) ?? 0) + 1);
  }

  // Pass 3: pick each line's term, and track how many lines chose it and the
  // worst tone among them.
  const counts = new Map<string, number>();
  const tones = new Map<string, HealthKind>();
  for (let i = 0; i < lines.length; i += 1) {
    const candidate = candidates[i];
    const term = candidate.trusted ?? termOf(candidate.clause, clauseFrequency);
    if (term === null) continue;
    counts.set(term, (counts.get(term) ?? 0) + 1);
    const health = healthOf(levelOf(lines[i].text, parsed[i]));
    const worst = tones.get(term);
    if (worst === undefined || HEALTH_RANK[health] > HEALTH_RANK[worst]) {
      tones.set(term, health);
    }
  }

  return Array.from(counts.entries())
    .filter(([, count]) => count >= MIN_RECURRENCES)
    .sort((a, b) => b[1] - a[1])
    .slice(0, cap)
    .map(([term, count]) => ({ term, count, tone: tones.get(term) ?? "neutral" }));
}
