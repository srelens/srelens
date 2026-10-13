import { useMemo, type KeyboardEvent } from "react";
import {
  clauseState,
  isQueryableKey,
  type ClauseMode,
  type ClauseTarget,
  type HealthKind,
  type QueryClause,
} from "@srelens/core";
import { CopyButton, StatusPill } from "@srelens/ui-kit";
import { Icons } from "../../lib/icons";

/** What the panel needs from a row. */
export interface DetailLine {
  readonly raw: string;
  readonly text: string;
  readonly pod: string;
  readonly container: string;
  readonly level: string;
  /** What core made of the level: the tone the header's pill takes. */
  readonly health: HealthKind;
  readonly fields: Readonly<Record<string, string>>;
}

/**
 * One line, every field of it, docked under the log (#898). A panel rather
 * than a row that expands in place: the body windows on a fixed row height,
 * and one tall row would break the arithmetic for every row after it.
 *
 * It holds the line it was opened with, so the ring dropping that line, or a
 * filter hiding it, does not snatch the panel away mid-read. It says so
 * instead.
 */
export function LineDetail({
  line,
  inView,
  clauses,
  onToggle,
  onClose,
}: {
  line: DetailLine;
  inView: boolean;
  clauses: readonly QueryClause[];
  onToggle: (target: ClauseTarget, mode: ClauseMode) => void;
  onClose: () => void;
}) {
  const stamp = line.raw.slice(0, line.raw.length - line.text.length).trim();
  // The level as a row of its own only when no field already carries it.
  const levelRow = line.level !== "" && !Object.keys(line.fields).some((k) => k.toLowerCase() === "level");
  const rows: { key: string; value: string; target?: { field: string; value: string } }[] = [
    ...(levelRow ? [{ key: "level", value: line.level, target: { field: "level", value: line.level } }] : []),
    ...Object.entries(line.fields).map(([key, value]) => {
      // `level:` in a query means the line's level word, which is what the filter must name: the field's own value
      // may be a number, and a line with no level word has nothing to filter on.
      if (key.toLowerCase() === "level") {
        return { key, value, ...(line.level === "" ? {} : { target: { field: "level", value: line.level } }) };
      }
      return { key, value, ...(isQueryableKey(key) ? { target: { field: key, value } } : {}) };
    }),
  ];
  const pretty = useMemo(() => prettyJson(line.text), [line.text]);

  function onKeyDown(e: KeyboardEvent) {
    if (e.key !== "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    onClose();
  }

  return (
    <section
      aria-label="Line detail"
      data-slot="line-detail"
      onKeyDown={onKeyDown}
      className="rule-t flex h-[40%] min-h-0 shrink-0 flex-col"
      style={{ background: "var(--ground-sunk)" }}
    >
      <header className="rule-b flex items-center gap-3 px-3 py-1.5 font-mono text-[0.75rem]">
        <span className="text-soft">{stamp === "" ? "no timestamp" : stamp}</span>
        <span className="text-muted">{line.container === "" ? line.pod : `${line.pod} · ${line.container}`}</span>
        {line.level !== "" && <StatusPill status={line.level} kind={line.health} />}
        {!inView && <span className="text-faint">No longer in view</span>}
        <span className="flex-1" />
        {/* The kit's copy control, so the click answers: "Copied", or "Copy failed" when the clipboard refuses.
            Keyed by the line so moving to another line starts it fresh rather than carrying the last answer. */}
        <CopyButton key={line.raw} text={line.raw} label="Copy line" />
        <button type="button" className="icon-btn" aria-label="Close the line detail" onClick={onClose}>
          <Icons.close size={13} aria-hidden="true" />
        </button>
      </header>
      <div className="scroll min-h-0 flex-1 px-3 py-1.5 font-mono text-[0.75rem]">
        <table className="w-full border-collapse">
          <tbody>
            {rows.map((r) => {
              const target = r.target;
              const state = target === undefined ? undefined : clauseState(clauses, target);
              return (
                <tr key={r.key} data-slot="detail-field" className="align-top">
                  <th scope="row" className="w-[1%] whitespace-nowrap pr-4 text-left font-normal text-muted">
                    {r.key}
                  </th>
                  {/* Machine text does not wrap. `max-w-0` bounds the cell to the room the row leaves; the value scrolls inside it. */}
                  <td className="w-full max-w-0 text-soft">
                    <div data-slot="detail-value" className="overflow-x-auto whitespace-pre">
                      {r.value === "" ? <span className="text-faint">(empty)</span> : r.value}
                    </div>
                  </td>
                  <td className="w-[1%] whitespace-nowrap pl-2">
                    {target !== undefined && (
                      <>
                        <button
                          type="button"
                          className="icon-btn"
                          aria-pressed={state === "include"}
                          aria-label={`Filter for ${target.field} = ${target.value}`}
                          onClick={() => onToggle(target, "include")}
                        >
                          <span aria-hidden="true">+</span>
                        </button>
                        <button
                          type="button"
                          className="icon-btn"
                          aria-pressed={state === "exclude"}
                          aria-label={`Filter out ${target.field} = ${target.value}`}
                          onClick={() => onToggle(target, "exclude")}
                        >
                          <span aria-hidden="true">−</span>
                        </button>
                      </>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        <details className="mt-2">
          <summary className="cursor-pointer text-muted">Raw</summary>
          <pre className="overflow-x-auto whitespace-pre text-soft">{pretty ?? line.raw}</pre>
        </details>
      </div>
    </section>
  );
}

function prettyJson(text: string): string | undefined {
  try {
    const value: unknown = JSON.parse(text.trim());
    return typeof value === "object" && value !== null ? JSON.stringify(value, null, 2) : undefined;
  } catch {
    return undefined;
  }
}
