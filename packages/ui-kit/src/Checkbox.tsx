import { useEffect, useRef, type ReactNode } from "react";
import { cx } from "./cx";
import { filled } from "./slot";

export interface CheckboxProps {
  checked: boolean;
  /** Given the state the box is moving to, not the event. */
  onChange?: (checked: boolean) => void;
  /** Shown beside the box, and the box's accessible name. */
  label?: ReactNode;
  /** The third state, for a header box over a partial selection. */
  indeterminate?: boolean;
  disabled?: boolean;
  /** Names the box when there is no visible label to name it. */
  ariaLabel?: string;
  /** Lands on the row when there is a label, on the box itself when there is not. */
  className?: string;
}

/**
 * A checkbox, with its label beside it when it has one.
 *
 * A native `<input type="checkbox">` rather than a Radix wrapper, which the
 * kit's other interactive components reach for. That rule came out of
 * hand-writing a modal focus trap, which is a library-sized problem; this is
 * the opposite one. The element already carries Space activation, focus, form
 * participation and the semantics a screen reader announces, and the design
 * colours it with `accent-color` — which is the native control themed, not
 * replaced. A wrapper here would mean re-implementing all of that to arrive
 * back where the browser started. (#320)
 *
 * The label wraps the input, so clicking the text toggles the box and the text
 * is the accessible name without any wiring. `ariaLabel` is for the boxes that
 * have no text at all — a select-all in a table header, a box in a row — and is
 * dropped when there is a visible label, because two names for one control are
 * either noise or a contradiction. (#320)
 */
export function Checkbox({
  checked,
  onChange,
  label,
  indeterminate,
  disabled,
  ariaLabel,
  className,
}: CheckboxProps) {
  const ref = useRef<HTMLInputElement>(null);
  const labelled = filled(label);

  // `indeterminate` is a DOM property with no matching attribute, so a ref is
  // the only way to set it — the mock's arrangement, kept, because a header box
  // over a partial selection is the one place a checkbox needs a third state.
  //
  // What changed is when it runs. Clicking an indeterminate box clears the
  // property in the DOM, so an effect keyed on the prop sees no change and
  // never puts the dash back: the header of a bulk selection loses it on the
  // first click and stays lost. Unconditional, it is one property write per
  // render and the DOM cannot drift from the prop. (#320)
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = Boolean(indeterminate);
  });

  const box = (
    <input
      ref={ref}
      type="checkbox"
      className={cx("accent-[var(--accent)]", !labelled && className)}
      checked={checked}
      disabled={disabled}
      aria-label={labelled ? undefined : ariaLabel}
      onChange={(e) => onChange?.(e.target.checked)}
    />
  );

  if (!labelled) return box;
  return (
    <label
      className={cx(
        "flex items-center gap-2 text-[0.8125rem]",
        disabled ? "cursor-default text-muted" : "cursor-pointer",
        className,
      )}
    >
      {box}
      {label}
    </label>
  );
}
