import { useEffect, useMemo, useReducer, useRef } from "react";
import { fieldValue, matchLine, type ParsedQuery, type QueryClause, type QueryLine } from "@srelens/core";
import type { RegexItem } from "../../lib/regexMatch";
import { createRegexOracle, type RegexOracle } from "../../lib/regexWorker";
import { shownText, type MessageRow } from "./LogMessage";

type RegexClause = Extract<QueryClause, { match: "regex" }>;
type Ranges = readonly (readonly [number, number])[];

/** One regex clause of the applied query, the worker that answers it, and the rows it has been asked about. */
interface Entry {
  readonly clause: RegexClause;
  readonly oracle: RegexOracle;
  readonly asked: WeakSet<object>;
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
 * - With one, `rows` is the rows answered for every regex, in order. A row is
 *   held back until it is, typically a frame.
 * - A regex too slow on some batch, or with no worker to run it, sends the
 *   screen back to the clauses it last applied in full, and says which.
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
  /** The last clauses applied with every row answered: what a regex that can't run goes back to. */
  const settled = useRef<View>(NO_VIEW);

  const regexes = useMemo(() => (applied.ok ? applied.clauses.filter(isRegex) : []), [applied]);
  const key = regexes.map(keyOf).join("\n");

  // Made in render, for a worker starts only when the first batch is sent, from an effect.
  const generation = useMemo<Generation | undefined>(
    () =>
      regexes.length === 0
        ? undefined
        : {
            entries: regexes.map((clause) => ({
              clause,
              oracle: createRegexOracle(clause.value, clause.regex.flags),
              asked: new WeakSet<object>(),
            })),
            fallback: settled.current,
          },
    // A new set of workers for a new set of regexes only: the rest of the query and the rows don't touch them.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [key, tries],
  );
  useEffect(() => () => generation?.entries.forEach((e) => e.oracle.dispose()), [generation]);

  // Ask each worker about the rows it hasn't been asked about. The first batch is the buffer; a commit brings the rest.
  useEffect(() => {
    if (generation === undefined) return;
    for (const entry of generation.entries) {
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
      if (items.length > 0) void entry.oracle.ask(items).then(answeredSome);
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
  }, [generation, rows]);

  const slow = generation?.entries.find((e) => e.oracle.state === "slow");
  const unavailable = generation?.entries.some((e) => e.oracle.state === "unavailable") ?? false;
  const failed = slow !== undefined || unavailable;
  // A worker that couldn't run isn't held against the pattern: the next change to the query tries again.
  useEffect(() => {
    if (unavailable) retry();
    // On a change to the query only, not on the render that found it unavailable.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [applied]);

  const current = useMemo<View>(() => {
    if (!applied.ok) return NO_VIEW;
    if (generation === undefined) return { clauses: applied.clauses, entries: [] };
    return { clauses: withLookups(applied.clauses, generation.entries), entries: generation.entries };
  }, [applied, generation]);
  const view = failed && generation !== undefined ? generation.fallback : current;

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

  // Not a query that does not parse: typing passes through those on the way to a regex, and none is a filter to go back to.
  const allIn = applied.ok && !failed && shownRows.length === rows.length;
  useEffect(() => {
    if (allIn) settled.current = view;
  });

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
