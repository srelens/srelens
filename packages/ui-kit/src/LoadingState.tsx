import { Spinner } from "./Spinner";
import { cx } from "./cx";

export interface LoadingStateProps {
  /** Text shown beneath the spinner; also the spinner's accessible label. */
  label?: string;
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
export function LoadingState({ label = "Loading", className }: LoadingStateProps) {
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
