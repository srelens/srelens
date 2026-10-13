import { removeClause, type QueryClause, type TimeRange } from "@srelens/core";
import { toneColor } from "@srelens/ui-kit";
import { Icons } from "../../lib/icons";
import { rangeLabel } from "./time";

/**
 * The query's clauses as removable pills, and the time window beside them. The
 * clause pills are a VIEW of the search box's text, never a second copy of it:
 * each × removes its clause from the text by the span it was parsed from, so
 * the box and the pills can't disagree. The window is the one filter that is
 * not query text, so it gets a pill of its own and is cleared by Clear all too.
 */
export function FilterPills({
  query,
  clauses,
  range,
  onQueryChange,
  onRangeChange,
  onRemoved,
}: {
  query: string;
  clauses: readonly QueryClause[];
  range: TimeRange | undefined;
  onQueryChange: (query: string) => void;
  onRangeChange: (range: TimeRange | undefined) => void;
  /**
   * Called after a × or Clear all. The control that had focus is about to
   * unmount, and focus would otherwise fall to the body.
   */
  onRemoved?: () => void;
}) {
  if (clauses.length === 0 && range === undefined) return null;
  return (
    <div data-slot="filter-pills" className="flex flex-wrap items-center gap-1.5 px-3 pt-2">
      {clauses.map((clause) => {
        const spelled = query.slice(clause.span[0], clause.span[1]);
        return (
          <Pill
            key={clause.span[0]}
            label={clause.negated ? `NOT ${spelled.slice(1)}` : spelled}
            negated={clause.negated}
            removeLabel={`Remove filter ${spelled}`}
            onRemove={() => {
              onQueryChange(removeClause(query, clause));
              onRemoved?.();
            }}
          />
        );
      })}
      {range !== undefined && (
        <Pill
          label={rangeLabel(range)}
          removeLabel="Remove the time window"
          onRemove={() => {
            onRangeChange(undefined);
            onRemoved?.();
          }}
        />
      )}
      <button
        type="button"
        // The kit's text button takes the size of what it sits in, and a row of pills is not a size.
        className="text-btn text-[0.75rem]"
        onClick={() => {
          onQueryChange("");
          onRangeChange(undefined);
          onRemoved?.();
        }}
      >
        Clear all
      </button>
    </div>
  );
}

function Pill({
  label,
  negated = false,
  removeLabel,
  onRemove,
}: {
  label: string;
  negated?: boolean;
  removeLabel: string;
  onRemove: () => void;
}) {
  return (
    <span
      data-slot="pill"
      className="inline-flex max-w-full items-center gap-1 overflow-x-auto whitespace-nowrap rounded border px-1.5 py-[1px] font-mono text-[0.6875rem]"
      style={negated ? { borderColor: toneColor("sev"), color: toneColor("sev") } : { borderColor: "var(--rule)" }}
    >
      {label}
      <button type="button" className="icon-btn" aria-label={removeLabel} onClick={onRemove}>
        <Icons.close size={11} aria-hidden="true" />
      </button>
    </span>
  );
}
