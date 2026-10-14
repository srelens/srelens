import type { ReactNode } from "react";
import { Picker, PickerGroup, PickerRow, optionLabel, type ComboboxOption } from "./picker";
import { filled } from "./slot";

export interface MultiSelectProps {
  options: ComboboxOption[];
  selection: string[];
  onChange: (selection: string[]) => void;
  /**
   * Names the row that clears the selection, and what an empty selection reads
   * as on the trigger. Leave it out and an empty selection is just empty.
   */
  allLabel?: string;
  placeholder?: string;
  searchPlaceholder?: string;
  ariaLabel?: string;
  className?: string;
  /**
   * Rows above the options — another kind of choice the caller makes in the
   * same place, given a way to close the popover. With it, the options sit
   * under `heading`.
   */
  leading?: (close: () => void) => ReactNode;
  /** The options' group heading; used when there is a `leading` section. */
  heading?: string;
  /**
   * What the trigger says, when the selection alone does not say it — the
   * caller's other choice is in force.
   */
  summary?: string;
  /** Whether the "all" row is the current state; by default, an empty selection is. */
  allChecked?: boolean;
  /**
   * Footer controls of the caller's own, on a row of their own under the
   * count, Select all and Reset: the popover is narrow, and one row of five
   * controls wraps their labels and cuts the last one off.
   */
  footerExtra?: ReactNode;
}

/**
 * A searchable multi select: `Combobox`'s twin, differing only in that a row
 * toggles rather than chooses, and that the popover stays open while it does.
 * Staying open is deliberate and is the whole point of the control — picking
 * six things out of a list of sixty should not cost six round trips through the
 * trigger. They share their shell; see `picker`. (#318)
 *
 * The classic version of this was a picker for one particular kind of thing,
 * and it hard-coded that vocabulary twice: in its prop names, and in the
 * sentinel that an empty selection means "all of them". The vocabulary stays in
 * the app — the same call `NavIcon` made when its icon map did not come across
 * — and the sentinel generalises into `allLabel`. A caller that wants the
 * sentinel names the row and gets it; a caller for whom "none selected" is a
 * real, different state simply leaves it out and never sees the row. (#318)
 *
 * Selected options are hoisted to the top, in the order they were given, then
 * the rest. Carried over from the classic version and for its reason: a
 * selection made from dozens of options is otherwise lost among them, and a
 * list that reorders under you as you scan it is worse than one that does not.
 * The hoist is computed from the selection prop, so it settles between renders
 * rather than while the pointer is over a row.
 */
export function MultiSelect({
  options,
  selection,
  onChange,
  allLabel,
  placeholder = "Select…",
  searchPlaceholder,
  ariaLabel,
  className,
  leading,
  heading,
  summary,
  allChecked,
  footerExtra,
}: MultiSelectProps) {
  // A Set, so a repeated value cannot produce two rows with the same key, and
  // iterating it still yields the caller's order.
  const chosen = new Set(selection);
  const byValue = new Map(options.map((option) => [option.value, option]));

  // `filled` rather than `allLabel != null`: `allLabel={scoped && "Everything"}`
  // is how a caller makes the row conditional, and it hands over `false`, which
  // renders nothing but would still buy a row and a place on the trigger. The
  // second comparison below is TypeScript's rather than ours — `filled` answers
  // with a boolean, not a type guard.
  const all = filled(allLabel) ? allLabel : undefined;

  const ordered =
    chosen.size === 0
      ? options
      : [
          // A selection can outlive the options it was made from — restored
          // from storage, or naming something that has since disappeared. Those
          // values still count towards the summary, but they get no row.
          ...[...chosen].map((value) => byValue.get(value)).filter((option) => option !== undefined),
          ...options.filter((option) => !chosen.has(option.value)),
        ];

  const toggle = (value: string) => {
    const next = new Set(chosen);
    if (next.has(value)) next.delete(value);
    else next.add(value);
    onChange([...next]);
  };

  const summarize = (): string => {
    if (selection.length === 0) return all ?? placeholder;
    // One selection reads better as itself than as "1 selected"; past that the
    // names stop fitting and the count is the useful fact. The label is
    // preferred over the value for the same reason the rows show it.
    if (selection.length === 1) {
      const only = byValue.get(selection[0]);
      return only ? optionLabel(only) : selection[0];
    }
    return `${selection.length} selected`;
  };

  return (
    <Picker
      summary={summary ?? summarize()}
      ariaLabel={ariaLabel}
      searchPlaceholder={searchPlaceholder}
      className={className}
      footer={
        <>
          {/* What the list currently amounts to, in the design's label voice.
              "all" rather than "0 selected", because an empty selection is not
              an empty result — it is the filter switched off. */}
          <span className="eyebrow flex-1">
            {selection.length === 0 ? "all" : selection.length} selected
          </span>
          <button type="button" className="btn !py-0" onClick={() => onChange(options.map((o) => o.value))}>
            Select all
          </button>
          <button type="button" className="btn !py-0" onClick={() => onChange([])}>
            Reset
          </button>
          {footerExtra !== undefined && (
            <div className="flex w-full basis-full items-center gap-1.5">{footerExtra}</div>
          )}
        </>
      }
    >
      {(close) => {
        const rows = (
        <>
          {all !== undefined && (
            <PickerRow
              value={all}
              label={all}
              checked={allChecked ?? selection.length === 0}
              onSelect={() => onChange([])}
              // A check, not a box: this is not one more thing to switch on
              // beside the others, it is the absence of any of them.
              trailing={<span className="path text-faint">{options.length}</span>}
            />
          )}
          {ordered.map((option) => (
            <PickerRow
              key={option.value}
              value={option.value}
              label={optionLabel(option)}
              checked={chosen.has(option.value)}
              onSelect={() => toggle(option.value)}
              mark="box"
              // Narrowing to one option is the other thing people do with a
              // list like this, and doing it by hand means turning every other
              // one off first. It closes, because it is the whole choice.
              trailing={
                <button
                  type="button"
                  className="ns-only"
                  aria-label={`Only ${optionLabel(option)}`}
                  onClick={(event) => {
                    event.stopPropagation();
                    onChange([option.value]);
                    close();
                  }}
                >
                  only
                </button>
              }
            />
          ))}
        </>
        );
        if (!leading) return rows;
        return (
          <>
            {leading(close)}
            {heading ? <PickerGroup heading={heading}>{rows}</PickerGroup> : rows}
          </>
        );
      }}
    </Picker>
  );
}
