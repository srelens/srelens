import { useRef, type ReactNode } from "react";
import { cx } from "./cx";
import { filled } from "./slot";

export interface FilterBarProps {
  value: string;
  onValueChange: (value: string) => void;
  /**
   * Names both the field and the landmark — "Filter pods", "Filter releases".
   * Required, because a filter with no name is the fault this component exists
   * to stop repeating.
   */
  label: string;
  /** Shown while the field is empty. Not a substitute for `label`. */
  placeholder?: string;
  disabled?: boolean;
  /** Controls that filter alongside the text — a namespace picker, a toggle. */
  children?: ReactNode;
  className?: string;
}

/**
 * The filter row above a list: a text field, a way out of it, and room for the
 * controls that filter alongside it.
 *
 * The mock's field had a placeholder and nothing else. A placeholder is not a
 * name — it disappears at the first keystroke, so a screen-reader user who
 * comes back to a half-typed filter is told only "edit text", and it is grey on
 * grey by design. `label` is required here rather than optional, because an
 * optional accessible name is one nobody passes.
 *
 * That name does double duty on the landmark. This is the search for the region
 * it sits above, and a `search` landmark is how a screen-reader user reaches it
 * without walking the list first; two lists on one screen means two of them, so
 * it is named rather than anonymous.
 *
 * The mock had no way out of a filter. Escape clears it, which is what the rest
 * of this kit does — but only while there is something to clear, because
 * Escape also closes the drawer the list is standing in, and a field that eats
 * the key it does not need traps the user one level down. The clear button
 * appears for the same reason and disappears when the filter is empty, and
 * focus goes back to the field after it is used, or the next keystroke goes
 * nowhere. It is `type="button"`: a lists screen is often a form, and a bare
 * button in one submits it.
 *
 * Not built on `TextInput`, which is the design's bordered field — a box inside
 * this box is one border too many, and the bar itself is what reads as the
 * field. Not built on `Toolbar` either: that strip sits on `--surface` and this
 * one sits on the sunk surface, which is what separates a filter row from the
 * chrome above it. The magnifier is drawn here rather than imported from
 * lucide, as everywhere else in the kit. (#320)
 */
export function FilterBar({
  value,
  onValueChange,
  label,
  placeholder,
  disabled,
  children,
  className,
}: FilterBarProps) {
  const fieldRef = useRef<HTMLInputElement>(null);

  return (
    <div
      role="search"
      aria-label={label}
      className={cx("rule-b flex shrink-0 flex-wrap items-center gap-3 px-2.5 py-1.5", className)}
      style={{ background: "var(--surface-sunk)" }}
    >
      <div className="flex min-w-[200px] flex-1 items-center gap-1.5">
        {/* Inline rather than an icon-set import: the kit takes no dependency
            on lucide, and this is the only glyph it needs. */}
        <svg
          width="13"
          height="13"
          viewBox="0 0 24 24"
          fill="none"
          aria-hidden="true"
          className="shrink-0 text-faint"
        >
          <circle cx="11" cy="11" r="7" stroke="currentColor" strokeWidth="2" />
          <path d="m20 20-3.6-3.6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
        </svg>
        <input
          ref={fieldRef}
          type="search"
          value={value}
          onChange={(e) => onValueChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key !== "Escape" || value === "") return;
            // Claimed only while there is a filter to drop. Left alone, Escape
            // belongs to whatever this list is inside.
            e.preventDefault();
            e.stopPropagation();
            onValueChange("");
          }}
          placeholder={placeholder}
          disabled={disabled}
          aria-label={label}
          className="w-full bg-transparent text-[0.8125rem] outline-none placeholder:text-faint"
        />
        {value !== "" && (
          <button
            type="button"
            className="icon-btn shrink-0"
            aria-label="Clear filter"
            title="Clear filter"
            disabled={disabled}
            onClick={() => {
              onValueChange("");
              // The button is about to vanish with the value that summoned it,
              // and focus would land on the body.
              fieldRef.current?.focus();
            }}
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" aria-hidden="true">
              <path d="M18 6 6 18M6 6l12 12" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
            </svg>
          </button>
        )}
      </div>
      {filled(children) && (
        <div data-slot="controls" className="flex flex-wrap items-center gap-1.5">
          {children}
        </div>
      )}
    </div>
  );
}
