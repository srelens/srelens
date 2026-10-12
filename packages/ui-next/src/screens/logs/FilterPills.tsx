import { removeClause, type QueryClause } from "@srelens/core";
import { toneColor } from "@srelens/ui-kit";
import { Icons } from "../../lib/icons";

/**
 * The query's clauses as removable pills. A VIEW of the search box's text,
 * never a second copy of it: each × removes its clause from the text by the
 * span it was parsed from, so the box and the pills can't disagree.
 */
export function FilterPills({
  query,
  clauses,
  onQueryChange,
}: {
  query: string;
  clauses: readonly QueryClause[];
  onQueryChange: (query: string) => void;
}) {
  if (clauses.length === 0) return null;
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
            onRemove={() => onQueryChange(removeClause(query, clause))}
          />
        );
      })}
      <button
        type="button"
        className="text-[0.75rem] text-muted hover:text-soft"
        onClick={() => onQueryChange("")}
      >
        Clear all
      </button>
    </div>
  );
}

export function Pill({
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
      className="inline-flex items-center gap-1 rounded border px-1.5 py-[1px] font-mono text-[0.6875rem]"
      style={negated ? { borderColor: toneColor("sev"), color: toneColor("sev") } : { borderColor: "var(--rule)" }}
    >
      {label}
      <button type="button" className="icon-btn" aria-label={removeLabel} onClick={onRemove}>
        <Icons.close size={11} aria-hidden="true" />
      </button>
    </span>
  );
}
