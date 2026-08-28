import { useId, type ReactNode } from "react";
import { cx } from "./cx";
import { filled } from "./slot";

export interface SwitchProps {
  on: boolean;
  /** Given the state the switch is moving to, not the event. */
  onChange?: (on: boolean) => void;
  /** Shown to the left of the track, and the switch's accessible name. */
  label?: ReactNode;
  /** A line under the label saying what turning it on does. */
  hint?: ReactNode;
  /** For a setting whose "on" is destructive: the track turns the severity colour. */
  danger?: boolean;
  disabled?: boolean;
  /** Names the switch when there is no visible label to name it. */
  ariaLabel?: string;
  /** Lands on the row when there is a label, on the track itself when there is not. */
  className?: string;
}

/**
 * An on/off toggle for a setting that takes effect as soon as it is flipped.
 *
 * There is no native switch element, so this keeps the mock's
 * `<button role="switch" aria-checked>` — the standard pattern, and already
 * operable by keyboard because underneath it is a button. Three things about
 * the mock's version could not come along.
 *
 * It rendered a bare `<button>`. Inside a `<form>` that is a submit button, so
 * flipping a setting would submit the form around it; `type="button"` is not
 * optional on any button the kit owns.
 *
 * It set `aria-label={label}` while also rendering the label visibly. With a
 * label that is the same name given twice; with no label it is
 * `aria-label={undefined}` on a button whose only content is a decorative disc,
 * which leaves the switch with no accessible name at all — the case that needed
 * `aria-label` most was the one it did not cover. So the visible label names
 * the control through `htmlFor`, the hint describes it through
 * `aria-describedby` rather than being swallowed into the name, and a switch
 * with no visible label takes `ariaLabel` from the caller.
 *
 * And the thumb was `bg-white`: a fixed colour on a control that is otherwise
 * entirely themed, so on the dark themes it is the one part of the switch that
 * ignores the theme. It is now the ink of whichever track it is sitting on.
 * (#320)
 */
export function Switch({
  on,
  onChange,
  label,
  hint,
  danger,
  disabled,
  ariaLabel,
  className,
}: SwitchProps) {
  const id = useId();
  const hintId = `${id}-hint`;
  const labelled = filled(label);
  const described = filled(hint);

  const control = (
    <button
      id={id}
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={labelled ? undefined : ariaLabel}
      aria-describedby={described ? hintId : undefined}
      disabled={disabled}
      onClick={() => onChange?.(!on)}
      className={cx(
        "h-[18px] w-8 shrink-0 rounded-full p-[2px] transition-colors disabled:opacity-50",
        !labelled && className,
      )}
      style={{ background: on ? (danger ? "var(--sev)" : "var(--accent)") : "var(--rule-strong)" }}
    >
      <span
        className="block h-[14px] w-[14px] rounded-full transition-transform"
        style={{
          background: on ? "var(--accent-ink)" : "var(--surface)",
          transform: on ? "translateX(14px)" : "none",
        }}
      />
    </button>
  );

  if (!labelled) return control;
  return (
    <div className={cx("kv !items-start", className)}>
      {/* The text dims on its own rather than the row dimming: the track is a
          sibling and carries `disabled:opacity-50` itself, so a faded wrapper
          would put the switch at a quarter opacity. */}
      <div className={cx("min-w-0", disabled && "opacity-50")}>
        <label
          htmlFor={id}
          className={cx(
            "block text-[0.8125rem] font-medium",
            disabled ? "cursor-default" : "cursor-pointer",
          )}
        >
          {label}
        </label>
        {described && (
          <div id={hintId} className="mt-0.5 text-[0.75rem] leading-snug text-muted">
            {hint}
          </div>
        )}
      </div>
      {control}
    </div>
  );
}
