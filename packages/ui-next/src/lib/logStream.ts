import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  appendLogLines,
  clearLogBuffer,
  createLogBuffer,
  describeError,
  startLogStream,
  type FriendlyError,
  type LogLine,
  type LogStatus,
  type LogStreamOptions,
  type LogTarget,
} from "@srelens/core";

/**
 * The hook the Logs screen sits on: it owns the buffer, the connection
 * status, and pause/clear controls, so the screen only has to render what
 * this returns.
 *
 * **Whose job is subject resolution.** `resolveLogSubject` (`./logSubject`)
 * is a one-shot async lookup, not a subscription — the screen calls it,
 * settles on a concrete `LogTarget[]`, and hands that array to this hook.
 * This hook never resolves a subject itself: its only input is targets that
 * are already known to be complete, because `startLogStream` cannot be
 * un-opened once a container's lines have been silently dropped by an
 * incomplete list.
 *
 * **Why the buffer lives in a ref, not `useState`.** `LogBuffer` is an
 * immutable value with pure functions on it (`packages/core/src/lib/logBuffer.ts`),
 * built so a caller holding it in a mutable slot can reassign that slot from
 * a stream callback that fires many times in one tick, and only read the
 * result once at the end. `onLine` does exactly that: it mutates
 * `bufferRef.current` synchronously on every call — never dropping a line to
 * a stale closure, no matter how many arrive before React gets a turn — and
 * schedules a single microtask to fold the ref into render state once the
 * burst has finished. A hundred lines fired in one tick land as one commit,
 * not a race between a hundred `setState` closures.
 *
 * **Pause freezes the view, not the stream.** The design's toggle only
 * relabels a button; classic tears the stream down and re-tails on resume,
 * losing whatever happened in the gap. Neither suits watching something
 * fail. Here, `bufferRef` keeps accumulating while paused — nothing further
 * arrives while the reader isn't looking is EVER dropped except by the
 * ring's own capacity — but the rendered `lines` snapshot is not refreshed
 * from it until resume, and `pendingWhilePaused` counts what arrived in the
 * meantime. Resume folds the buffer into view and resets that count; it
 * never touches the connection, so it can never cause a re-tail.
 *
 * **The unmount race `startLogStream` opens.** `startLogStream` is async: it
 * awaits an `invokeCommand` before its promise resolves. If the component
 * unmounts in that window, a naive `useEffect` would store the resolved
 * `{ stop }` into a ref that nothing reads again — a live subscription with
 * no owner left to stop it. The effect's cleanup instead sets a local
 * `cancelled` flag; the promise's resolve handler checks it and calls
 * `stream.stop()` itself when the mount lost the race, instead of stashing
 * the handle for a cleanup that already ran.
 *
 * **Restarts are unavoidable and must not be silent.** Changing targets, the
 * since window or the tail length has to reopen the stream — there is no
 * live way to add a target the backend didn't tail from the start — and that
 * costs the reader their scrollback. `restartCount` increments on every such
 * restart after the first connect (never on the initial mount, which has
 * nothing to lose, and never on pause/resume or a manual `clear()`, neither
 * of which reopens the stream) so the screen can say "scrollback cleared"
 * instead of quietly emptying the pane.
 *
 * **The indicator answers "am I seeing everything?".** A stream fans out over
 * many pods and each one connects, drops and reconnects on its own schedule.
 * Every status event names its target (`source`, the same tag that target's
 * lines carry), so this hook keeps each target's latest state and derives the
 * aggregate from all of them at once — see {@link aggregateLogStatus}. The
 * word alone cannot say how much of a fan-out is down, so `liveTargets` /
 * `reconnectingTargets` / `totalTargets` ride alongside it: a screen showing
 * "2 of 3 following" tells a reader during an incident exactly which of the
 * two questions — a blip, or the whole tail — they are looking at. Pair the
 * word with `logConnectionStatus` from `@srelens/core` for its label and
 * tone; this module deliberately owns no vocabulary of its own.
 */

export interface UseLogStreamOptions extends LogStreamOptions {
  /** How many lines the ring keeps before dropping the oldest. */
  capacity?: number;
}

/** Connection health, plus the states `startLogStream` itself can't report:
 *  `"connecting"` before the first status or failure, and `"error"` when the
 *  connect promise rejected. */
export type LogStreamStatus = "connecting" | LogStatus | "error";

/** What is known about each of a stream's targets right now. `total` is the
 *  denominator for both counts; targets that have not reported yet are
 *  neither `live` nor `reconnecting`. */
export interface LogTargetCounts {
  readonly live: number;
  readonly reconnecting: number;
  readonly total: number;
}

/**
 * The one word for a whole stream, from what is known about its targets.
 *
 * "Am I seeing everything?" is the question the indicator answers, so a
 * single target being down is enough to say no: `reconnecting` the moment ANY
 * target reports it, `live` only once EVERY target has reported streaming,
 * and `connecting` while some target has yet to say either way. A screen that
 * wants to distinguish "one of ten down" from "all ten down" reads the counts
 * beside it — the word alone was never able to carry that, and the streak
 * counter this replaces bought a steadier word by delaying the truth: on a
 * fifty-pod workload a target that stayed down took fifty backoff cycles
 * (~100s) to surface, and one that flapped never surfaced at all, because any
 * other target's success reset the streak.
 *
 * Never returns `"error"`: that is the hook's own state, not any target's.
 */
export function aggregateLogStatus(counts: LogTargetCounts): "connecting" | LogStatus {
  if (counts.reconnecting > 0) return "reconnecting";
  if (counts.total > 0 && counts.live >= counts.total) return "live";
  return "connecting";
}

export interface UseLogStreamResult {
  /** The visible lines, oldest first — frozen while `paused` is true. */
  lines: readonly LogLine[];
  /** How many lines the ring has dropped from the visible buffer. */
  dropped: number;
  status: LogStreamStatus;
  /** How many of this stream's targets are streaming right now. */
  liveTargets: number;
  /** How many are down and retrying — every one of them a gap in the tail. */
  reconnectingTargets: number;
  /** How many targets this stream follows: the denominator for both counts. */
  totalTargets: number;
  /** Set when `status` is `"error"` — why the stream could not be opened. */
  error?: FriendlyError;
  paused: boolean;
  /** Lines that have arrived since pausing, not yet folded into `lines`. */
  pendingWhilePaused: number;
  togglePause: () => void;
  /** Empty the buffer and view without touching the connection. */
  clear: () => void;
  /**
   * Bumped every time a target/since/tailLines change forces a restart after
   * the first connect. A screen watching this rise is how it knows to say
   * scrollback was cleared, rather than the clear passing unremarked.
   */
  restartCount: number;
}

const DEFAULT_CAPACITY = 5000;

/** Before the connection effect has run there is nothing known about anything. */
const NO_TARGETS: LogTargetCounts = { live: 0, reconnecting: 0, total: 0 };

/** A stable key for a target list, so the connection effect restarts on what
 *  a target list actually MEANS rather than on a new array identity a
 *  caller might pass every render. */
function targetsKey(targets: readonly LogTarget[]): string {
  return targets.map((t) => `${t.pod}|${t.container ?? ""}|${t.label ?? ""}`).join(",");
}

export function useLogStream(
  context: string,
  namespace: string,
  targets: readonly LogTarget[],
  options: UseLogStreamOptions = {},
): UseLogStreamResult {
  const capacity = options.capacity ?? DEFAULT_CAPACITY;
  const timestamps = options.timestamps ?? false;
  const sinceSeconds = options.sinceSeconds;
  const tailLines = options.tailLines;
  const key = targetsKey(targets);

  const bufferRef = useRef(createLogBuffer(capacity));
  const pausedRef = useRef(false);
  const pendingRef = useRef(0);
  const flushScheduledRef = useRef(false);
  const firstRunRef = useRef(true);

  const [view, setView] = useState(() => bufferRef.current);
  const [paused, setPaused] = useState(false);
  const [pendingWhilePaused, setPendingWhilePaused] = useState(0);
  const [counts, setCounts] = useState<LogTargetCounts>(NO_TARGETS);
  const [error, setError] = useState<FriendlyError | undefined>(undefined);
  const [restartCount, setRestartCount] = useState(0);

  const commit = useCallback(() => {
    flushScheduledRef.current = false;
    if (pausedRef.current) return;
    setView(bufferRef.current);
  }, []);

  const scheduleCommit = useCallback(() => {
    if (flushScheduledRef.current) return;
    flushScheduledRef.current = true;
    void Promise.resolve().then(commit);
  }, [commit]);

  // Stable across restarts: the same ref keeps accumulating, so a target
  // change doesn't need a new callback, only a fresh buffer (below).
  const onLine = useCallback(
    (source: string, text: string) => {
      bufferRef.current = appendLogLines(bufferRef.current, [{ source, text }]);
      if (pausedRef.current) {
        pendingRef.current += 1;
        setPendingWhilePaused(pendingRef.current);
      } else {
        scheduleCommit();
      }
    },
    [scheduleCommit],
  );

  const togglePause = useCallback(() => {
    const next = !pausedRef.current;
    pausedRef.current = next;
    setPaused(next);
    if (!next) {
      // Resuming folds the accumulated buffer into view in one step — never
      // a re-tail, since the connection is untouched by this branch.
      pendingRef.current = 0;
      setPendingWhilePaused(0);
      setView(bufferRef.current);
    }
  }, []);

  const clear = useCallback(() => {
    bufferRef.current = clearLogBuffer(bufferRef.current);
    pendingRef.current = 0;
    setPendingWhilePaused(0);
    setView(bufferRef.current);
  }, []);

  useEffect(() => {
    let cancelled = false;
    let stopFn: (() => void) | undefined;

    bufferRef.current = createLogBuffer(capacity);
    pendingRef.current = 0;
    setPendingWhilePaused(0);
    setView(bufferRef.current);
    // A restart knows nothing about the new targets until they report; the
    // old targets' states are not evidence about these ones.
    const sources = new Set(targets.map((t) => t.label ?? ""));
    setCounts({ live: 0, reconnecting: 0, total: sources.size });
    setError(undefined);

    if (targets.length === 0) {
      // F1: a quiet not-yet, not a failure. `startLogStream` throws on an
      // empty target list — a plain string `describeError` cannot classify —
      // but there is nothing wrong here: the caller (typically a screen mid
      // subject-resolution) just doesn't have a target list yet. Never call
      // it with one; leave `status` at "connecting" and stop, without ever
      // touching `firstRunRef`, so the eventual real first connect — once
      // targets do arrive — still counts as the initial connect rather than
      // a "restart" announcing scrollback lost that was never there to lose.
      return;
    }

    // `firstRunRef` flips only on an actual connect attempt, never on the
    // empty-targets skip above.
    if (!firstRunRef.current) {
      // F6 (inert today): this repo runs no StrictMode, so this effect body
      // runs once per real mount or dependency change. Under StrictMode,
      // React double-invokes it (mount, synthetic cleanup, mount again), and
      // that second synthetic run would find `firstRunRef.current` already
      // false and count a phantom restart here — a `restartCount` lie. Not
      // fixing it now: there is no StrictMode anywhere in this codebase to
      // make it reachable, and restructuring working code around a
      // hypothetical would just be a different way to get it wrong.
      setRestartCount((c) => c + 1);
    }
    firstRunRef.current = false;

    const streamTargets: LogTarget[] = targets.map((t) => ({ ...t }));

    // F3: `status` disagreeing across targets. `onStatus` fires once per
    // underlying pod/container connection transition — core's resilient log
    // loop (crates/kube/src/logs.rs) emits one "reconnecting" per failed
    // connect attempt and one "live" per success — and each event names the
    // target it came from, the same `source` tag that target's lines carry.
    // So the aggregate is not a guess: keep each source's LATEST state and
    // count them. One target retrying every two seconds is one entry moving
    // between two values, not a stream of events racing each other to be the
    // last writer.
    //
    // Keyed by source because that is the identity the backend reports under.
    // Two targets sharing a label are one source to everything downstream,
    // their lines included, so `total` counts distinct sources — counting the
    // array instead would leave the aggregate permanently short of its own
    // denominator for a target that can never report separately.
    const total = sources.size;
    const seen = new Map<string, LogStatus>();

    // Effect-scoped, unlike `onLine` itself: a dependency change (since,
    // container, tail length, ...) cancels THIS run before its connect
    // promise settles, and the old stream's initial tail lines can still
    // arrive on the channel in the gap before its `.then()` gets around to
    // calling `stop()`. Without this guard those lines land, through the
    // shared `onLine`, in the buffer the new effect already cleared.
    const guardedOnLine = (source: string, text: string) => {
      if (cancelled) return;
      onLine(source, text);
    };

    startLogStream(
      context,
      namespace,
      streamTargets,
      guardedOnLine,
      (s, source) => {
        if (cancelled) return;
        seen.set(source, s);
        let live = 0;
        let reconnecting = 0;
        for (const state of seen.values()) {
          if (state === "live") live += 1;
          else reconnecting += 1;
        }
        setCounts({ live, reconnecting, total });
      },
      { timestamps, sinceSeconds, tailLines },
    ).then(
      (stream) => {
        // The dangerous window: this component (or this connection's
        // dependencies) went away between the call and the promise
        // resolving. Nothing else is left to stop the subscription that was
        // just created, so this branch stops it itself instead of storing a
        // handle a cleanup that already ran will never read.
        if (cancelled) {
          stream.stop();
          return;
        }
        stopFn = stream.stop;
      },
      (e: unknown) => {
        if (cancelled) return;
        setError(describeError(e));
      },
    );

    return () => {
      cancelled = true;
      stopFn?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [context, namespace, key, sinceSeconds, tailLines, timestamps, capacity, onLine]);

  return useMemo(
    () => ({
      lines: view.lines,
      dropped: view.dropped,
      // The failure to OPEN a stream is the hook's own, not a target's, and
      // outranks whatever the targets last said.
      status: error !== undefined ? "error" : aggregateLogStatus(counts),
      liveTargets: counts.live,
      reconnectingTargets: counts.reconnecting,
      totalTargets: counts.total,
      error,
      paused,
      pendingWhilePaused,
      togglePause,
      clear,
      restartCount,
    }),
    [view, counts, error, paused, pendingWhilePaused, togglePause, clear, restartCount],
  );
}
