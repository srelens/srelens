import type { ReactNode } from "react";
import { cx } from "./cx";

export interface FieldProps {
  label: ReactNode;
  /**
   * Optional control shown opposite the label (e.g. a "Preview" button).
   * Rendered as a SIBLING of the `<label>`, never inside it: a `<button>` is a
   * labelable element, so nesting it would make label clicks activate it and
   * would swallow its accessible name into the label's name computation.
   */
  action?: ReactNode;
  /** Helper text under the control. */
  hint?: ReactNode;
  /** Replaces the hint when the field has failed validation. */
  error?: ReactNode;
  children: ReactNode;
  className?: string;
}

/** The small print under a control: the error if there is one, else the hint. */
function Note({ hint, error }: { hint?: ReactNode; error?: ReactNode }) {
  // Never both. Two lines of small print under one control, one of them advice
  // the user has already failed to follow, reads as a rendering fault.
  if (error) {
    return (
      <div className="mt-1 text-[0.75rem]" style={{ color: "var(--sev)" }}>
        {error}
      </div>
    );
  }
  if (hint) return <div className="mt-1 text-[0.75rem] text-muted">{hint}</div>;
  return null;
}

/**
 * A labelled form control: label above, control, optional hint or error below.
 *
 * Wraps the control in the `<label>` rather than rendering the two as siblings,
 * which is both what the new design's markup does and an association the
 * classic component never had — it emitted a bare `<label>` with no `htmlFor`,
 * so clicking the text did nothing and assistive technology had to guess from
 * proximity. (#318)
 *
 * With an `action` the wrapper cannot be the `<label>`, for the reason given on
 * the prop, so that form keeps the classic component's arrangement.
 */
export function Field({ label, action, hint, error, children, className }: FieldProps) {
  if (action) {
    return (
      <div className={cx("py-1", className)}>
        <div className="mb-0.5 flex items-center justify-between gap-2">
          <span className="field-label">{label}</span>
          {action}
        </div>
        {children}
        <Note hint={hint} error={error} />
      </div>
    );
  }
  return (
    <label className={cx("block py-1", className)}>
      <span className="field-label mb-0.5">{label}</span>
      {children}
      <Note hint={hint} error={error} />
    </label>
  );
}
