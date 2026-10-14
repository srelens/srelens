import { useEffect, useMemo, useReducer, useRef } from "react";
import { fieldValue, type ParsedQuery, type QueryClause, type QueryLine } from "@srelens/core";
import { cachedVerdict, trialRegex } from "../../lib/regexTrial";

type RegexClause = Extract<QueryClause, { match: "regex" }>;

/** No clauses: the query is empty, or it does not parse and so is not applied. */
const NO_CLAUSES: readonly QueryClause[] = [];

const keyOf = (clause: RegexClause) => `${clause.regex.flags}/${clause.value}`;

/** Exactly what the clause will test: the line, or the value of the field it names. */
function textsFor(clause: RegexClause, lines: readonly QueryLine[]): string[] {
  if (clause.field === undefined) return lines.map((line) => line.text);
  const texts: string[] = [];
  for (const line of lines) {
    const value = fieldValue(line, clause.field);
    if (value !== undefined) texts.push(value);
  }
  return texts;
}

/**
 * The clauses the Logs screen may filter by (#898). The query as applied goes
 * through unchanged unless it holds a `/regex/`, and a regex is trial-run in a
 * worker over the buffer first (`trialRegex`): until every one has passed, the
 * screen goes on filtering by what it had, and one that is too slow is never
 * applied, so a pattern that would freeze the app is named instead.
 *
 * `lines` is the buffer the clauses will be tested on. A query restored with a
 * tab meets an empty one, and a trial over nothing passes at once and is
 * remembered, so the first trial waits for the first line.
 */
export function useRegexGate(
  applied: ParsedQuery,
  lines: readonly QueryLine[],
): { clauses: readonly QueryClause[]; slow: string | undefined } {
  /** A trial finishing is a change to the verdicts, which are read from the memory at render. */
  const [, trialDone] = useReducer((n: number) => n + 1, 0);
  /** What the screen filters by now: what a query waiting on its trial is held back on. */
  const shown = useRef<readonly QueryClause[]>(NO_CLAUSES);
  /** The patterns this screen has started a trial for, so lines arriving do not start it again. */
  const tried = useRef(new Set<string>());

  const regexes = useMemo(
    () => (applied.ok ? applied.clauses.filter((c): c is RegexClause => c.match === "regex") : []),
    [applied],
  );
  const regexKey = regexes.map(keyOf).join("\n");
  const hasLines = lines.length > 0;

  useEffect(() => {
    if (!hasLines) return;
    for (const clause of regexes) {
      const key = keyOf(clause);
      if (tried.current.has(key) || cachedVerdict(clause.value, clause.regex.flags) !== undefined) continue;
      tried.current.add(key);
      void trialRegex(clause.value, clause.regex.flags, textsFor(clause, lines)).then(trialDone);
    }
    // The trial is for the patterns, taken over the buffer as it stands when one is new or the first line lands.
    // Every batch of lines would otherwise run this pass again for patterns that already have a trial.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [regexKey, hasLines]);

  let clauses: readonly QueryClause[] = NO_CLAUSES;
  let slow: string | undefined;
  if (applied.ok) {
    const verdicts = regexes.map((c) => cachedVerdict(c.value, c.regex.flags));
    const slowAt = verdicts.indexOf("slow");
    if (slowAt >= 0) {
      clauses = shown.current;
      slow = `/${regexes[slowAt].value}/`;
    } else {
      clauses = verdicts.includes(undefined) ? shown.current : applied.clauses;
    }
  }
  useEffect(() => {
    shown.current = clauses;
  });
  return { clauses, slow };
}
