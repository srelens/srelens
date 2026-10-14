import { useEffect, useMemo, useReducer, useRef } from "react";
import { fieldValue, matchLine, type ParsedQuery, type QueryClause, type QueryLine } from "@srelens/core";
import type { RegexItem } from "../../lib/regexMatch";
import { createRegexOracle, type RegexOracle } from "../../lib/regexWorker";
import { shownText, type MessageRow } from "./LogMessage";

type RegexClause = Extract<QueryClause, { match: "regex" }>;
type Ranges = readonly (readonly [number, number])[];

/**
 * One pattern and target, the worker that answers it, and the rows it has been asked about. One regex has one,
 * shared by every query that holds it, so going back to an earlier query reuses its worker and its answers.
 */
interface Entry {
  readonly clause: RegexClause;
  readonly oracle: RegexOracle;
  readonly asked: WeakSet<object>;
  /** Its first batch, the buffer as it stood when the regex came, has been sent. */
  begun: boolean;
  /** And answered. Until then the regex isn't applied, and the screen keeps what it had. */
  primed: boolean;
}

/** Clauses the screen can filter by, and the workers their regexes read. */
interface View {
  readonly clauses: readonly QueryClause[];
  readonly entries: readonly Entry[];
}

/** The workers for one set of regexes, and what to go back to if one of them can't run. */
interface Generation {
  readonly entries: readonly Entry[];
  readonly fallback: View;
}

/** No clauses: the query is empty, or it does not parse and so is not applied. */
const NO_CLAUSES: readonly QueryClause[] = [];
const NO_VIEW: View = { clauses: NO_CLAUSES, entries: [] };
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
 * - A new regex applies once its worker has answered the buffer as it stood.
 *   Until then the screen keeps the clauses and rows it had, not an empty list.
 * - From then on, `rows` is the rows answered for every regex, in order: a line
 *   that lands later is held back until it is, typically a frame.
 * - A regex too slow on some batch, or with no worker to run it, sends the
 *   screen back to the clauses it had before, and says which. Their regexes'
 *   workers are still running, so lines that land after that are answered too.
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
  /** Every worker the gate holds, by pattern and target. */
  const pool = useRef(new Map<string, Entry>());
  /** The view last given out for a query that parsed: what a new regex is held back on, and goes back to. */
  const shown = useRef<View>(NO_VIEW);

  const regexes = useMemo(() => (applied.ok ? applied.clauses.filter(isRegex) : []), [applied]);
  const key = regexes.map(keyOf).join("\n");

  // Made in render, for a worker starts only when its first batch is sent, from the effect below.
  const generation = useMemo<Generation | undefined>(
    () =>
      regexes.length === 0
        ? undefined
        : {
            entries: regexes.map((clause) => {
              const had = pool.current.get(keyOf(clause));
              // One that couldn't run is not kept: a new query is its next try.
              if (had !== undefined && had.oracle.state !== "unavailable") return had;
              had?.oracle.dispose();
              const made: Entry = {
                clause,
                oracle: createRegexOracle(clause.value, clause.regex.flags),
                asked: new WeakSet<object>(),
                begun: false,
                primed: false,
              };
              pool.current.set(keyOf(clause), made);
              return made;
            }),
            fallback: shown.current,
          },
    // New workers for a new set of regexes only: the rest of the query and the rows don't touch them.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [key, tries],
  );

  const current = useMemo<View>(() => {
    if (!applied.ok) return NO_VIEW;
    if (generation === undefined) return { clauses: applied.clauses, entries: [] };
    return { clauses: withLookups(applied.clauses, generation.entries), entries: generation.entries };
  }, [applied, generation]);
  const usable = (v: View) => v.entries.every((e) => e.primed && e.oracle.state === "ready");
  // The fallback's own regex can go slow on a later line too; with nothing left to go back to, nothing is filtered.
  const view =
    generation === undefined || usable(current) ? current : usable(generation.fallback) ? generation.fallback : NO_VIEW;

  useEffect(() => {
    // Not a query that does not parse: typing passes through those on the way to a regex, and none is a filter to go back to.
    if (applied.ok) shown.current = view;
    // A worker lives while the applied query, the view it would go back to, or the view on screen uses it.
    const live = new Set([...(generation?.entries ?? []), ...(generation?.fallback.entries ?? []), ...shown.current.entries]);
    for (const [k, entry] of pool.current) {
      if (live.has(entry)) continue;
      entry.oracle.dispose();
      pool.current.delete(k);
    }
    // Ask each about the rows it hasn't been asked about. The first batch is the buffer; a commit brings the rest.
    for (const entry of live) {
      if (entry.oracle.state !== "ready") continue;
      const items: RegexItem[] = [];
      for (const row of rows) {
        if (entry.asked.has(row)) continue;
        entry.asked.add(row);
        const test = testedOn(entry.clause, row);
        if (test === undefined) continue;
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
  }, [applied, generation, view, rows]);
  useEffect(() => () => pool.current.forEach((entry) => entry.oracle.dispose()), []);

  const slow = generation?.entries.find((e) => e.oracle.state === "slow");
  const unavailable = generation?.entries.some((e) => e.oracle.state === "unavailable") ?? false;
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
    const marking = view.entries.filter((e) => e.clause.field === undefined && !e.clause.negated);
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
    const asked: Promise<void>[] = [];
    for (const { clause, oracle } of entries) {
      const items: RegexItem[] = [];
      for (const line of lines) {
        const test = testedOn(clause, line);
        if (test !== undefined) items.push({ test });
      }
      for (let at = 0; at < items.length; at += CHUNK) asked.push(oracle.ask(items.slice(at, at + CHUNK)));
    }
    await Promise.all(asked);
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
