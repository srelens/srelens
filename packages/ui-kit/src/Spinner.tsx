import type { ComponentProps } from "react";
import { cx } from "./cx";

export interface SpinnerProps extends Omit<ComponentProps<"svg">, "aria-label"> {
  /** Accessible label; defaults to "Loading". */
  label?: string;
}

/**
 * Indeterminate loading spinner: a muted track ring with a spinning accent arc.
 *
 * Inherits the current text colour so it blends wherever it sits inline with a
 * label, and takes `className` to scale or recolour it.
 *
 * The new design has no spinner of its own — it shimmers skeletons instead — so
 * this is the "only the old kit has it" path: restyled against the new tokens,
 * API and tests unchanged. In practice nothing needed restyling, since it was
 * already drawn in `currentColor` rather than a named colour. (#318)
 */
export function Spinner({ label = "Loading", className, ...props }: SpinnerProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      role="status"
      aria-label={label}
      className={cx("size-4 animate-spin", className)}
      {...props}
    >
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="2.5" className="opacity-20" />
      <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" />
    </svg>
  );
}
