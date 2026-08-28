import type { ReactNode } from "react";
import { cx } from "./cx";
import { filled } from "./slot";

export interface EmptyStateProps {
  title: ReactNode;
  /** One line of context under the title: why it is empty, or what fills it. */
  hint?: ReactNode;
  /** A control the caller owns — usually the button that ends the emptiness. */
  action?: ReactNode;
  /**
   * The rail-sized form: a quarter of the padding and a step down in type.
   *
   * A prop rather than a `className` at the call site. Both forms are Tailwind
   * utilities, and two utilities that set the same padding are resolved by the
   * order of the generated stylesheet rather than by the order the JSX writes
   * them — so an override from outside is a coin flip. Exactly one set is
   * emitted here.
   *
   * It earns its place because the page-sized form is not neutral in a 286px
   * rail: `py-10` around three wrapped lines spends more height stating an
   * absence than the section below it gets to exist in, and the cluster
   * overview's `Fleet` went below the fold behind one.
   */
  compact?: boolean;
  className?: string;
}

/**
 * The placeholder for a list or panel that loaded successfully and has nothing
 * in it — the settled counterpart to `LoadingState`, which speaks for a load
 * still in flight.
 *
 * The classic version offered a title and a description, which left every
 * caller with an empty list and no way out of it; the design's shape adds an
 * `action` slot, and takes a node rather than a label and a handler so the
 * caller's own button arrives with its variant, its disabled state and its
 * confirmation intact. The hint is capped at 42ch because a centred column of
 * prose stops being readable long before it reaches the width of a table.
 *
 * It stays silent to assistive technology: this is the resting state of a
 * region the reader navigated to, not an event worth announcing. (#318)
 */
export function EmptyState({ title, hint, action, compact, className }: EmptyStateProps) {
  return (
    <div
      data-compact={compact ? "true" : undefined}
      className={cx(
        "flex flex-col items-center justify-center text-center",
        compact ? "gap-0.5 px-3 py-3" : "gap-1.5 px-6 py-10",
        className,
      )}
    >
      <div className={compact ? "text-[0.8125rem] font-medium" : "text-[0.875rem] font-medium"}>
        {title}
      </div>
      {filled(hint) && (
        <div
          data-slot="hint"
          className={cx(
            "max-w-[42ch] text-muted",
            compact ? "text-[0.75rem] leading-snug" : "text-[0.8125rem] leading-relaxed",
          )}
        >
          {hint}
        </div>
      )}
      {filled(action) && (
        <div data-slot="action" className="mt-2">
          {action}
        </div>
      )}
    </div>
  );
}
