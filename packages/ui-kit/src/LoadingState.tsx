import { Spinner } from "./Spinner";
import { cx } from "./cx";

export interface LoadingStateProps {
  /** Text shown beneath the spinner; also the spinner's accessible label. */
  label?: string;
  /** One line inside a block, under a heading that may hold a single table row. */
  compact?: boolean;
  className?: string;
}

/**
 * Prominent, centred loading placeholder for a content area whose data is still
 * being fetched. Wraps {@link Spinner} with a caption so an in-flight load reads
 * clearly instead of looking like an empty result.
 *
 * The classic version coloured itself with `text-muted-foreground` and the
 * spinner with `text-primary`, both classic tokens that do not exist here; the
 * new design's equivalents are `text-muted` and the accent. (#318)
 */
export function LoadingState({ label = "Loading", compact, className }: LoadingStateProps) {
  if (compact) {
    return (
      <div className={cx("flex items-center gap-2 py-1 text-[0.75rem] text-muted", className)}>
        <Spinner label={label} className="size-3.5" style={{ color: "var(--accent)" }} />
        <span>{label}</span>
      </div>
    );
  }
  return (
    <div
      className={cx(
        "flex flex-col items-center justify-center gap-3 py-16 text-muted",
        className,
      )}
    >
      <Spinner label={label} className="size-8" style={{ color: "var(--accent)" }} />
      <span className="text-[0.8125rem]">{label}</span>
    </div>
  );
}
