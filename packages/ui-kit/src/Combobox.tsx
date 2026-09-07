import { Picker, PickerRow, optionLabel, type ComboboxOption } from "./picker";

// The option shape is shared with `MultiSelect` and so is declared alongside
// the shell they share, but it is named for this component and belongs to its
// public surface — so it is exported from here, where a caller expects it.
export type { ComboboxOption } from "./picker";

export interface ComboboxProps {
  value: string;
  onValueChange: (value: string) => void;
  options: ComboboxOption[];
  placeholder?: string;
  searchPlaceholder?: string;
  ariaLabel?: string;
  className?: string;
}

/**
 * A searchable, height-capped single select.
 *
 * Reach for it over `Select` when the option set is long enough that a native
 * dropdown stops being a list and becomes a wall: the trigger reads like a
 * select, but behind it is a search box over a bounded, scrolling list. The
 * classic API is unchanged — a value in, a value out — because every call site
 * is written against it. (#318)
 *
 * `""` is an ordinary value here and not a sentinel for "nothing chosen": the
 * app's own callers use it for the option meaning "no filter", and `Select`
 * made the same call for the same reason. So the trigger shows the placeholder
 * only when no option claims the current value at all, rather than whenever the
 * value is empty.
 *
 * Choosing closes the popover, which is what separates this from `MultiSelect`
 * and is the only behavioural difference between them; the shell they share
 * lives in `picker`. (#318)
 */
export function Combobox({
  value,
  onValueChange,
  options,
  placeholder = "Select…",
  searchPlaceholder,
  ariaLabel,
  className,
}: ComboboxProps) {
  const selected = options.find((option) => option.value === value);

  return (
    <Picker
      summary={selected ? optionLabel(selected) : placeholder}
      ariaLabel={ariaLabel}
      searchPlaceholder={searchPlaceholder}
      className={className}
    >
      {(close) =>
        options.map((option) => (
          <PickerRow
            key={option.value}
            value={option.value}
            label={optionLabel(option)}
            checked={option.value === value}
            onSelect={() => {
              onValueChange(option.value);
              close();
            }}
          />
        ))
      }
    </Picker>
  );
}
