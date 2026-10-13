import { useEffect, useRef, useState } from "react";

/**
 * `value`, changing at most once every `ms` milliseconds: for a value that changes faster than what reads it can
 * keep up with, like a live log's rows under the counts drawn from them.
 *
 * A change after a quiet spell lands at once, so a single change is never late. Changes inside the spell wait for
 * its end, and then the latest of them lands, so the last value is never lost. With `ms` of 0 or less the value is
 * handed straight through.
 */
export function useThrottledValue<T>(value: T, ms: number): T {
  const [shown, setShown] = useState(value);
  /** When the last change landed, by `Date.now()`. */
  const landedRef = useRef(Number.NEGATIVE_INFINITY);
  useEffect(() => {
    if (Object.is(shown, value)) return;
    // A function, so a value that is itself one is not taken for an updater.
    const land = () => {
      landedRef.current = Date.now();
      setShown(() => value);
    };
    // Unthrottled, `shown` is kept up too: the wait coming back must not hand back a value from before it went.
    const wait = ms <= 0 ? 0 : landedRef.current + ms - Date.now();
    if (wait <= 0) {
      land();
      return;
    }
    const timer = setTimeout(land, wait);
    return () => clearTimeout(timer);
  }, [value, ms, shown]);
  return ms <= 0 ? value : shown;
}
