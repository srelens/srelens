// Pure, dependency-free relative-time formatting for the chat history
// rail/popover (Task 19) — takes both timestamps explicitly (rather than
// reading `Date.now()` internally) so it's trivially unit-testable with
// fixed inputs and never drifts between a render and its test assertion.

const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

/**
 * Formats how long ago `tsMs` was relative to `nowMs`:
 * "just now" (< 1 minute), "Xm ago" (< 1 hour), "Xh ago" (< 1 day), else
 * "Xd ago". A `tsMs` in the future (clock skew, or a reload racing a save)
 * is clamped to "just now" rather than a nonsensical negative duration.
 */
export function relativeTime(tsMs: number, nowMs: number): string {
  const diffMs = Math.max(0, nowMs - tsMs);
  if (diffMs < MINUTE_MS) return "just now";
  if (diffMs < HOUR_MS) return `${Math.floor(diffMs / MINUTE_MS)}m ago`;
  if (diffMs < DAY_MS) return `${Math.floor(diffMs / HOUR_MS)}h ago`;
  return `${Math.floor(diffMs / DAY_MS)}d ago`;
}
