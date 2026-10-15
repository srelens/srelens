import { useEffect, useMemo, useReducer, useRef } from "react";
import { fieldValue, matchLine, type ParsedQuery, type QueryClause, type QueryLine } from "@srelens/core";
import type { RegexItem } from "../../lib/regexMatch";
import { createRegexOracle, type RegexOracle } from "../../lib/regexWorker";
import { shownText, type MessageRow } from "./LogMessage";

type RegexClause = Extract<QueryClause, { match: "regex" }>;
type Ranges = readonly (readonly [number, number])[];

/**
 * One pattern and target, and the worker that answers it. The pattern, the
 * target and the flags are its key, and the sign is not: `/x/` and `-/x/` ask
 * the same question of the same lines, so editing a query keeps the worker and
 * its answers.
 */
interface Entry {
  readonly clause: RegexClause;
  readonly oracle: RegexOracle;
  /** Its first batch, the buffer as it stood when the regex came, has been sent. */
  begun: boolean;
  /** And answered. Until then the regex isn't applied. */
  primed: boolean;
}

/** Clauses the screen can filter by, and the workers their regexes read. */
interface View {
  readonly clauses: readonly QueryClause[];
  /** The workers of the regexes in `clauses`. */
  readonly entries: readonly Entry[];
  /** Those of them that mark a row: free text, and not negated in the query as it is now. */
  readonly marking: readonly Entry[];
}

/** No clauses: the query is empty, or it does not parse and so is not applied. */
const NO_CLAUSES: readonly QueryClause[] = [];
const NO_VIEW: View = { clauses: NO_CLAUSES, entries: [], marking: [] };
const NO_RANGES: Ranges = [];
const noRanges = () => NO_RANGES;

/**
 * Lines per batch for lines that are not the buffer (an export of a time range): the ring's default size, so the
 * budget allows a pattern the same time per line there as it does on screen.
 */
const CHUNK = 5000;

const isRegex = (clause: QueryClause): clause is RegexClause => clause.match === "regex";
const keyOf = (clause: RegexClause) => JSON.stringify([clause.field ?? null, clause.regex.flags, clause.value]);

/** What the clause tests on a line: its text, or the value of the field it names (none when the line has no such field). */
const testedOn = (clause: RegexClause, line: QueryLine) =>
  clause.field === undefined ? line.text : fieldValue(line, clause.field);

/**
 * The clause with a `regex` that reads the worker's answers. It is compiled,
 * for its source and flags, but never run: its `test` looks the answer up, and
 * a string with no answer yet does not match.
 */
function lookup(clause: RegexClause, oracle: RegexOracle): RegexClause {
  const regex = new RegExp(clause.value, clause.regex.flags);
  regex.test = (s: string) => oracle.get(s)?.hit ?? false;
  return { ...clause, regex };
}

/** The applied clauses with each regex replaced by its lookup, in the order `entries` were made from them. */
function withLookups(clauses: readonly QueryClause[], entries: readonly { readonly oracle: RegexOracle }[]) {
  let at = 0;
  return clauses.map((c) => (isRegex(c) ? lookup(c, entries[at++].oracle) : c));
}

/** Whether a regex is applied: its worker has answered its first batch, and can still answer. */
const ready = (e: Entry) => e.primed && e.oracle.state === "ready";

/** Whether every regex of the view has its answer for the row. A row with no value for a field regex has nothing to wait on. */
function answered(entries: readonly Entry[], row: QueryLine): boolean {
  return entries.every((e) => {
    const test = testedOn(e.clause, row);
    return test === undefined || e.oracle.get(test) !== undefined;
  });
}

/**
 * The clauses and rows the Logs screen may use (#898). A user's `/regex/` is
 * never run on the main thread: a pattern that backtracks catastrophically on
 * one line freezes the app, and no trial over the lines so far can vouch for
 * the next. Each regex clause gets a worker that answers every row, and the
 * clauses returned read those answers.
 *
 * - With no regex, the rows and the applied clauses come back as they are.
 * - A regex applies once its worker has answered the buffer as it stood. Until
 *   then, the query is applied without it, not as an empty list.
 * - From then on, `rows` is the rows answered for every applied regex, in
 *   order: a line that lands later is held back until it is, typically a frame.
 * - A regex too slow on some batch, or with no worker to run it, is left out of
 *   the query the same way, and named. What stays is the query as the box has
 *   it now, never a filter the reader has since edited away; its other regexes'
 *   workers go on answering the lines that land after.
 */
export function useRegexGate<R extends QueryLine & MessageRow>(
  applied: ParsedQuery,
  rows: readonly R[],
): {
  clauses: readonly QueryClause[];
  rows: readonly R[];
  slow: string | undefined;
  unavailable: boolean;
  rangesFor(row: R): Ranges;
} {
  /** Bumped when a batch is answered or a worker gives up: both are read off the workers at render. */
  const [answers, answeredSome] = useReducer((n: number) => n + 1, 0);
  /** Bumped to try again a regex no worker could run. */
  const [tries, retry] = useReducer((n: number) => n + 1, 0);
  /** Every worker the gate holds, by its key. */
  const pool = useRef(new Map<string, Entry>());

  const regexes = useMemo(() => (applied.ok ? applied.clauses.filter(isRegex) : []), [applied]);
  const key = regexes.map(keyOf).join("\n");

  // Made in render, for a worker starts only when its first batch is sent, from the effect below.
  const entries = useMemo(
    () =>
      regexes.map((clause) => {
        const had = pool.current.get(keyOf(clause));
        // One that couldn't run is not kept: a new query is its next try.
        if (had !== undefined && had.oracle.state !== "unavailable") return had;
        had?.oracle.dispose();
        const made: Entry = { clause, oracle: createRegexOracle(clause.value, clause.regex.flags), begun: false, primed: false };
        pool.current.set(keyOf(clause), made);
        return made;
      }),
    // New workers for a new set of regexes only: the rest of the query and the rows don't touch them.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [key, tries],
  );

  /** Which regexes are applied, as a key: the view below changes when one comes in or drops out, and not otherwise. */
  const readiness = entries.map((e) => (ready(e) ? "1" : "0")).join("");
  const view = useMemo<View>(() => {
    if (!applied.ok) return NO_VIEW;
    if (entries.length === 0) return { clauses: applied.clauses, entries: [], marking: [] };
    const clauses: QueryClause[] = [];
    const kept: Entry[] = [];
    const marking: Entry[] = [];
    let at = 0;
    for (const clause of applied.clauses) {
      if (!isRegex(clause)) {
        clauses.push(clause);
        continue;
      }
      const entry = entries[at++];
      // Not answered yet, too slow, or no worker: the query is applied without it.
      if (!ready(entry)) continue;
      clauses.push(lookup(clause, entry.oracle));
      kept.push(entry);
      // The sign is the query's now: the entry keeps the clause it was made for, which may have been `-/x/`.
      if (clause.field === undefined && !clause.negated) marking.push(entry);
    }
    return { clauses, entries: kept, marking };
    // `readiness` is what moves when a worker answers its first batch or gives up; the entries are read live.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [applied, entries, readiness]);

  useEffect(() => {
    // While the query doesn't parse, the workers are left be: typing passes through that on the way to a regex.
    if (!applied.ok) return;
    const live = new Set(entries);
    for (const [k, entry] of pool.current) {
      if (live.has(entry)) continue;
      entry.oracle.dispose();
      pool.current.delete(k);
    }
    // Ask each about the rows it has no answer for. The first batch is the buffer; a commit brings the rest.
    for (const entry of live) {
      if (entry.oracle.state !== "ready") continue;
      const items: RegexItem[] = [];
      for (const row of rows) {
        const test = testedOn(entry.clause, row);
        // Asked again when the answer is gone, pruned while the row was out of the buffer. One on its way is not
        // sent twice: the worker's side skips it.
        if (test === undefined || entry.oracle.get(test) !== undefined) continue;
        // A free-text regex marks the row as it is shown, so the worker works the marks out too.
        items.push(entry.clause.field === undefined ? { test, shown: shownText(row) } : { test });
      }
      const asked = items.length > 0 ? entry.oracle.ask(items) : undefined;
      if (!entry.begun) {
        entry.begun = true;
        void (asked ?? Promise.resolve()).then(() => {
          entry.primed = true;
          answeredSome();
        });
      } else if (asked !== undefined) void asked.then(answeredSome);
      // A live buffer drops its oldest lines as new ones come: the answers for them go once they are the bulk.
      if (entry.oracle.size > 2 * rows.length) {
        const keep = new Set<string>();
        for (const row of rows) {
          const test = testedOn(entry.clause, row);
          if (test !== undefined) keep.add(test);
        }
        entry.oracle.prune(keep);
      }
    }
  }, [applied, entries, rows]);
  useEffect(() => () => pool.current.forEach((entry) => entry.oracle.dispose()), []);

  const slow = entries.find((e) => e.oracle.state === "slow");
  const unavailable = entries.some((e) => e.oracle.state === "unavailable");
  // A worker that couldn't run isn't held against the pattern: the next change to the query tries again.
  useEffect(() => {
    if (unavailable) retry();
    // On a change to the query only, not on the render that found it unavailable.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [applied]);

  const shownRows = useMemo(
    () => (view.entries.length === 0 ? rows : rows.filter((row) => answered(view.entries, row))),
    // `answers` is what moves when the workers answer; the entries themselves are read live.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [rows, view, answers],
  );

  const rangesFor = useMemo(() => {
    const { marking } = view;
    if (marking.length === 0) return noRanges;
    return (row: R): Ranges => marking.flatMap((e) => e.oracle.get(row.text)?.ranges ?? NO_RANGES);
  }, [view]);

  return {
    clauses: view.clauses,
    rows: shownRows,
    slow: slow === undefined ? undefined : `/${slow.clause.value}/`,
    unavailable,
    rangesFor,
  };
}

/**
 * The lines that pass `clauses`, for lines that are not in the buffer: an
 * export of a time range. Each regex is answered by a worker of its own, made
 * for the call and ended after it. Rejects, in the screen's words, when one is
 * too slow or can't run.
 */
export async function keepMatching<L extends QueryLine>(clauses: readonly QueryClause[], lines: readonly L[]): Promise<L[]> {
  const entries = clauses.filter(isRegex).map((clause) => ({
    clause,
    oracle: createRegexOracle(clause.value, clause.regex.flags),
  }));
  try {
    await Promise.all(
      entries.map(async ({ clause, oracle }) => {
        const items: RegexItem[] = [];
        for (const line of lines) {
          const test = testedOn(clause, line);
          if (test !== undefined) items.push({ test });
        }
        // One chunk at a time: chunks queued behind a batch are sent as one, which would be the whole range again.
        for (let at = 0; at < items.length && oracle.state === "ready"; at += CHUNK) {
          await oracle.ask(items.slice(at, at + CHUNK));
        }
      }),
    );
    for (const { clause, oracle } of entries) {
      if (oracle.state === "slow") throw new Error(`Too slow to run on these lines: /${clause.value}/`);
      if (oracle.state === "unavailable") throw new Error("Regex search is unavailable here");
    }
    const looked = withLookups(clauses, entries);
    return lines.filter((line) => matchLine(line, looked));
  } finally {
    for (const { oracle } of entries) oracle.dispose();
  }
}
