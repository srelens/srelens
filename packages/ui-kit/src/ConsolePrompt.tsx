import { forwardRef, type KeyboardEvent, type ReactNode } from "react";
import { Eyebrow } from "./Eyebrow";
import { Spinner } from "./Spinner";
import { cx } from "./cx";
import { filled } from "./slot";
import { toneColor } from "./tone";

export interface ConsolePromptProps {
  /** What the input holds; the caller owns it. */
  value: string;
  onValueChange: (value: string) => void;
  /** Enter, or the send control. Never called while `busy` or empty. */
  onSubmit: () => void;
  /** Keys the host wants first — a `/` menu's arrows, Escape. Returning true
   *  means handled, so this component leaves Enter alone. */
  onKeyDown?: (e: KeyboardEvent<HTMLInputElement>) => boolean;
  placeholder?: string;
  /** Names the input, since a placeholder is not a label. */
  label: string;
  /** A turn is in flight: the send control gives way to a spinner, and the
   *  host's Stop (if it has one) is what acts. */
  busy?: boolean;
  /** A keycap, `⌘K` — drawn only where the host actually binds one. */
  shortcutHint?: ReactNode;
  /** The dock opens itself when its input takes focus; the agent screen has
   *  nothing to open. */
  onFocus?: () => void;
  /**
   * Stop the turn in flight. Offered beside the working spinner, because
   * `busy` is the only state in which it means anything.
   *
   * This is the ONLY Stop in the app now. It used to live on the agent
   * screen's own composer, and that composer is gone — one prompt component,
   * every screen. A question that cannot be stopped is worse than an untidy
   * bar.
   */
  onStop?: () => void;
  /** Anything the host puts to the LEFT of the input: the dock's collapse
   *  chevron, the agent screen's nothing. */
  lead?: ReactNode;
  /** Anything after the send control — the agent screen's Stop. */
  trail?: ReactNode;
  className?: string;
}

/**
 * The one prompt bar, wherever srelens asks for a question.
 *
 * **Why this is a component and not two.** `ConsoleDock` had this row inline,
 * and the `/agent` screen's own composer grew its own: a plain input with the
 * agent's name as loose text beside a `Send` button. Two bars for one job,
 * which looked like two different products on two screens of the same app —
 * reported as "replace this with the general chat bar used everywhere".
 *
 * The host keeps everything that differs (what a `/` opens, whether there are
 * attachments, what Stop does) and this keeps the shape: the input, the
 * keycap, the send control, and what a turn in flight looks like.
 */
export const ConsolePrompt = forwardRef<HTMLInputElement, ConsolePromptProps>(function ConsolePrompt(
  {
    value,
    onValueChange,
    onSubmit,
    onKeyDown,
    placeholder,
    label,
    busy = false,
    shortcutHint,
    onFocus,
    onStop,
    lead,
    trail,
    className,
  },
  ref,
) {
  const ready = value.trim().length > 0 && !busy;

  return (
    <div className={cx("flex h-[34px] min-w-0 items-center gap-2 px-2.5", className)}>
      {lead}
      <input
        ref={ref}
        className="console-input !text-[0.8125rem]"
        // A placeholder is not a label: it is gone the moment anything is
        // typed, and there is nothing else for the input to go on.
        aria-label={`${label} prompt`}
        value={value}
        placeholder={placeholder}
        onFocus={onFocus}
        onChange={(e) => onValueChange(e.target.value)}
        onKeyDown={(e) => {
          if (onKeyDown?.(e)) return;
          if (e.key === "Enter" && ready) {
            e.preventDefault();
            onSubmit();
          }
        }}
      />
      {busy ? (
        <span className="flex shrink-0 items-center gap-1.5">
          <Spinner label="Working" className="size-3" style={{ color: toneColor("accent") }} />
          <Eyebrow className="text-[0.5625rem]">working</Eyebrow>
          {onStop && (
            <button type="button" className="text-btn" aria-label="Stop" onClick={onStop}>
              <span className="eyebrow whitespace-nowrap text-[0.5625rem]">stop</span>
            </button>
          )}
        </span>
      ) : (
        <>
          {filled(shortcutHint) && <span className="kbd shrink-0">{shortcutHint}</span>}
          <button
            type="button"
            className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-md transition-opacity disabled:opacity-25"
            style={{ background: toneColor("accent"), color: "var(--accent-ink)" }}
            aria-label="Send"
            disabled={!ready}
            onClick={onSubmit}
          >
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" aria-hidden="true">
              <path
                d="M12 19V5m0 0-7 7m7-7 7 7"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
        </>
      )}
      {trail}
    </div>
  );
});
