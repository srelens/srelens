import { clauseState, type QueryClause, type QueryLine } from "@srelens/core";
import { groupNumber } from "../../lib/numbers";
import { Dot, GROUPS } from "./volumeGroups";

/** One legend entry's worth: how many lines, and the level words they carry. */
export interface LegendGroup {
  count: number;
  levels: string[];
}

/**
 * Per group, in `GROUPS` order: how many lines, and the level words they carry,
 * which is what a click on the entry puts in the query. Core decides which
 * group a line is in (`health`) and what its word is (`level`); a line whose
 * health is none of the three coloured ones, `success` included, is "other".
 */
export function tallyLegend(lines: readonly QueryLine[]): LegendGroup[] {
  const groups = GROUPS.map(() => ({ count: 0, levels: new Set<string>() }));
  for (const line of lines) {
    const found = GROUPS.findIndex(({ kind }) => kind === line.health);
    const group = groups[found === -1 ? GROUPS.length - 1 : found];
    group.count += 1;
    if (line.level !== "") group.levels.add(line.level.toLowerCase());
  }
  // Sorted, so the query a click writes does not depend on which line arrived first.
  return groups.map(({ count, levels }) => ({ count, levels: [...levels].sort() }));
}

/**
 * The chart's key, and a filter: error, warn and info are toggles that put
 * every level word of their group in the search box or take them out, `other`
 * is only a count (a line with no level can't be named in the query). An entry
 * is pressed when all of its words are in the query as typed.
 */
export function VolumeLegend({
  groups,
  clauses,
  onToggleLevels,
}: {
  groups: readonly LegendGroup[];
  clauses: readonly QueryClause[];
  onToggleLevels: (words: string[], include: boolean) => void;
}) {
  return (
    <div data-slot="volume-legend" className="mt-0.5 flex flex-wrap items-center gap-x-1 gap-y-0.5">
      {GROUPS.map(({ key, kind, word }, g) => {
        const { count, levels } = groups[g];
        if (count === 0) return null;
        const label = (
          <>
            <Dot kind={kind} faint={key === "other"} />
            {word} <span className="tabular-nums text-faint">{groupNumber(count)}</span>
          </>
        );
        const cls = "inline-flex items-center gap-1.5 rounded px-1.5 py-0.5 text-[0.6875rem] text-muted";
        if (key === "other") {
          return (
            <span key={key} data-slot="legend-item" className={cls}>
              {label}
            </span>
          );
        }
        const pressed =
          levels.length > 0 && levels.every((w) => clauseState(clauses, { field: "level", value: w }) === "include");
        return (
          <button
            key={key}
            type="button"
            data-slot="legend-item"
            aria-pressed={pressed}
            onClick={() => onToggleLevels(levels, !pressed)}
            className={`${cls} hover:bg-[var(--field)] aria-pressed:bg-[var(--field)] aria-pressed:text-ink`}
          >
            {label}
          </button>
        );
      })}
    </div>
  );
}
