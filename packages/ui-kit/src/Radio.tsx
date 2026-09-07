import { useId, type ReactNode } from "react";
import { cx } from "./cx";
import { filled } from "./slot";

export interface RadioProps {
  checked: boolean;
  /** Called when this option is chosen. Never called to say it was un-chosen: a radio only ever turns on. */
  onChange?: () => void;
  label: ReactNode;
  /** A line under the label saying what choosing this does. */
  hint?: ReactNode;
  /** The group. Every option offering the same choice shares one. */
  name: string;
  disabled?: boolean;
  className?: string;
}

/**
 * One option in a group, with an optional line of explanation under it.
 *
 * A native `<input type="radio">`, for the reason given on `Checkbox`, and for
 * one that belongs to radios alone: the browser turns every input sharing a
 * `name` into a single tab stop with arrow-key movement between the options and
 * wrap-around at the ends. That is the entire keyboard contract of an ARIA
 * radio group, correct in every browser, for the price of an attribute. (#320)
 *
 * The label is an explicit `<label htmlFor>` around the option text alone, with
 * the hint referenced by `aria-describedby`. The mock wrapped label and hint in
 * one `<label>`, which folds the explanation into the control's accessible
 * name: a screen reader then announces "Every 30 seconds Refreshes the list in
 * the background" as the name of the option, when only the first half names it.
 * (#320)
 */
export function Radio({ checked, onChange, label, hint, name, disabled, className }: RadioProps) {
  const id = useId();
  const hintId = `${id}-hint`;
  const described = filled(hint);
  return (
    <div
      className={cx(
        "flex items-start gap-2 py-1 text-[0.8125rem]",
        disabled && "text-muted",
        className,
      )}
    >
      <input
        id={id}
        type="radio"
        name={name}
        className="mt-0.5 accent-[var(--accent)]"
        checked={checked}
        disabled={disabled}
        aria-describedby={described ? hintId : undefined}
        onChange={() => onChange?.()}
      />
      <span className="min-w-0">
        <label htmlFor={id} className={cx("block", disabled ? "cursor-default" : "cursor-pointer")}>
          {label}
        </label>
        {described && (
          <span id={hintId} className="block text-[0.75rem] text-muted">
            {hint}
          </span>
        )}
      </span>
    </div>
  );
}
