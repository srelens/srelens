import { cloneElement, isValidElement, useId, type ReactNode } from "react";
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
function Note({ id, hint, error }: { id: string; hint?: ReactNode; error?: ReactNode }) {
  // Never both. Two lines of small print under one control, one of them advice
  // the user has already failed to follow, reads as a rendering fault.
  if (error) {
    return (
      <div id={id} className="mt-1 text-[0.75rem]" style={{ color: "var(--sev)" }}>
        {error}
      </div>
    );
  }
  if (hint) {
    return (
      <div id={id} className="mt-1 text-[0.75rem] text-muted">
        {hint}
      </div>
    );
  }
  return null;
}

interface DescribedControlProps {
  id?: string;
  "aria-describedby"?: string;
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
 * The note is deliberately outside that label. Text inside a wrapping label is
 * part of the control's accessible name, so putting a sentence-long hint there
 * renamed "Chart version" to "Chart version Leave empty…". The control points
 * to the note with `aria-describedby` instead: its name says what it is and its
 * description says how to fill it. Existing descriptions are retained. (#359)
 *
 * With an `action` the wrapper cannot be the `<label>`, for the reason given on
 * the prop. That form uses an explicit `htmlFor`/`id` association instead.
 */
export function Field({ label, action, hint, error, children, className }: FieldProps) {
  const generatedControlId = useId();
  const noteId = useId();
  const note = error ?? hint;
  const child = isValidElement<DescribedControlProps>(children) ? children : null;
  const controlId = child?.props.id ?? generatedControlId;
  const describedBy = [child?.props["aria-describedby"], note ? noteId : undefined]
    .filter(Boolean)
    .join(" ") || undefined;
  const control = child
    ? cloneElement(child, { id: controlId, "aria-describedby": describedBy })
    : children;

  if (action) {
    return (
      <div className={cx("py-1", className)}>
        <div className="mb-0.5 flex items-center justify-between gap-2">
          <label className="field-label" htmlFor={child ? controlId : undefined}>
            {label}
          </label>
          {action}
        </div>
        {control}
        <Note id={noteId} hint={hint} error={error} />
      </div>
    );
  }
  return (
    <div className={cx("py-1", className)}>
      <label className="block">
        <span className="field-label mb-0.5">{label}</span>
        {control}
      </label>
      <Note id={noteId} hint={hint} error={error} />
    </div>
  );
}
