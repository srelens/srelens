import { cx } from "./cx";

export interface SelectOption {
  value: string;
  label?: string;
}

export interface SelectProps {
  value: string;
  onValueChange: (value: string) => void;
  options: SelectOption[];
  className?: string;
  placeholder?: string;
  /**
   * Greys the control and stops it emitting. For a control whose value has no
   * meaning in the current state — rather than one the reader merely lacks
   * permission to use, which wants an explanation instead of a dead control.
   */
  disabled?: boolean;
  "aria-label"?: string;
}

/**
 * Dropdown with a value-first change contract.
 *
 * A native `<select>`, as the new design uses. The classic version wrapped
 * shadcn's Radix select and had to encode `""` as a sentinel, because Radix
 * forbids an empty-string item value — the whole `enc`/`dec` dance exists for
 * that one restriction. A native select has no such rule, so `""` is simply a
 * value and the sentinel is gone. (#318)
 *
 * `placeholder` becomes a disabled leading option, which is how a native select
 * says "nothing chosen yet"; it appears only when no option matches the current
 * value, so it never competes with a real selection — and when it appears, it is
 * what the control shows.
 */
export function Select({
  value,
  onValueChange,
  options,
  className,
  placeholder,
  disabled,
  "aria-label": ariaLabel,
}: SelectProps) {
  const unmatched = !options.some((o) => o.value === value);
  // A controlled value matching no option leaves the browser free to choose,
  // and it chooses the first enabled one — so the control displayed a value the
  // parent state did not hold, in the very state where nothing is selected yet.
  // Rendering the placeholder's value instead selects the placeholder when
  // there is one, and selects nothing when there is not. (#322 review)
  const rendered = unmatched ? "" : value;
  return (
    <div className="relative inline-flex items-center">
      <select
        value={rendered}
        disabled={disabled}
        onChange={(e) => onValueChange(e.target.value)}
        aria-label={ariaLabel}
        className={cx(
          "appearance-none rounded-md border py-1 pl-2 pr-6 text-[0.8125rem] outline-none",
          className,
        )}
        style={{ background: "var(--surface-sunk)", borderColor: "var(--rule)" }}
      >
        {unmatched ? (
          // Rendered whenever the value matches nothing, with or without
          // placeholder text: `rendered` needs an option to land on, or the
          // browser falls back to the first real one and the control shows a
          // value nobody chose. Hidden when it has no text, so an unlabelled
          // blank never becomes a visible row in the list.
          <option value="" disabled hidden={!placeholder}>
            {placeholder ?? ""}
          </option>
        ) : null}
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label ?? o.value}
          </option>
        ))}
      </select>
      {/* Decorative: the select already announces itself. */}
      <svg
        aria-hidden="true"
        width="12"
        height="12"
        viewBox="0 0 24 24"
        fill="none"
        className="pointer-events-none absolute right-1.5"
        style={{ color: "var(--ink-faint)" }}
      >
        <path d="m6 9 6 6 6-6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
      </svg>
    </div>
  );
}
