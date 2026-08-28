/**
 * A bounded ring for a live log stream: it holds at most `capacity` lines and
 * says how many it has thrown away, instead of the silent splice classic does
 * (`apps/desktop/src/components/LogsView.tsx`, `MAX_LINES`).
 *
 * A reader who scrolls to the top of a capped buffer is not at the beginning
 * of the log — they are wherever the ring's tail happens to land. Dropping
 * that fact silently makes the buffer lie; reporting `dropped` lets the
 * screen say so.
 *
 * This module holds no state of its own: {@link LogBuffer} is an immutable
 * value and every function here returns a new one rather than mutating in
 * place. That is deliberate, not incidental — a caller that owns a mutable
 * slot (a `useRef`, in the React layer this feeds) can reassign it from a
 * stream callback that fires many times in one tick, and only read the result
 * once, after the burst, to hand off to `setState`. Reassigning a ref never
 * touches React state, so nothing in a burst reads a stale closure. That
 * wiring is a React concern for a later task; this module only has to not
 * make it impossible, which an immutable value does for free.
 */

/** One line arriving from a log stream, tagged with where it came from. */
export interface LogLine {
  /** "pod/container", or "" when there is exactly one source to blend into. */
  source: string;
  text: string;
}

/**
 * The ring's state: its capacity, the newest lines it currently holds (oldest
 * first), and how many lines have been pushed out of the front over its
 * lifetime.
 */
export interface LogBuffer {
  readonly capacity: number;
  readonly lines: readonly LogLine[];
  readonly dropped: number;
}

/** A fresh, empty buffer holding at most `capacity` lines. */
export function createLogBuffer(capacity: number): LogBuffer {
  return { capacity: Math.max(1, Math.floor(capacity)), lines: [], dropped: 0 };
}

/**
 * Append `incoming` lines (oldest first) to `buffer`, returning the new
 * state. When the combined total exceeds capacity, the oldest lines are
 * spliced off the head and `dropped` rises by exactly that many.
 *
 * This applies to a single append that is itself larger than the whole
 * capacity too: once combined, nothing distinguishes a line that was already
 * in the buffer from one that just arrived, so a batch of 5,000 lines landing
 * on an empty buffer of capacity 100 reports 4,900 dropped, the same as if those
 * lines had arrived one at a time. That is the only honest count — anything
 * smaller would understate how much of the stream the reader never sees.
 */
export function appendLogLines(buffer: LogBuffer, incoming: readonly LogLine[]): LogBuffer {
  if (incoming.length === 0) return buffer;
  const combined = buffer.lines.length > 0 ? buffer.lines.concat(incoming) : incoming.slice();
  const overflow = combined.length - buffer.capacity;
  if (overflow <= 0) {
    return { capacity: buffer.capacity, lines: combined, dropped: buffer.dropped };
  }
  return {
    capacity: buffer.capacity,
    lines: combined.slice(overflow),
    dropped: buffer.dropped + overflow,
  };
}

/** Empty the buffer and reset its drop count, keeping its capacity. */
export function clearLogBuffer(buffer: LogBuffer): LogBuffer {
  return { capacity: buffer.capacity, lines: [], dropped: 0 };
}
