import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  appendLogLines,
  clearLogBuffer,
  createLogBuffer,
  describeError,
  prependLogLines,
  startLogStream,
  type FriendlyError,
  type LogBuffer,
  type LogFrame,
  type LogLine,
  type LogStatus,
  type LogStream,
  type LogStreamOptions,
  type LogTarget,
  type SpoolPage,
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
 * result once at the end. `onLine` does exactly that: it pushes every line
 * onto a batch in a ref — never dropping one to a stale closure, no matter how
 * many arrive before React gets a turn — and schedules a single commit, for the
 * next frame, that folds the whole batch into the ring with ONE `appendLogLines`
 * and hands the result to render state. However many lines arrive between two
 * frames land as one copy of the ring and one commit, not one of each per line
 * or per tick: each append copies the whole ring, and one at a time, ten
 * thousand lines onto a full ring took ~80ms where one batch took ~1ms. A batch
 * that reaches the ring's capacity before its commit is folded there and then,
 * so a commit held back cannot grow a backlog.
 *
 * **Pause freezes the view, not the stream.** The design's toggle only
 * relabels a button; classic tears the stream down and re-tails on resume,
 * losing whatever happened in the gap. Neither suits watching something
 * fail. Here, `bufferRef` keeps accumulating while paused — nothing further
 * arrives while the reader isn't looking is EVER dropped except by the
 * ring's own capacity — but the rendered `lines` snapshot is not refreshed
 * from it until resume, and `pendingWhilePaused` counts what arrived in the
 * meantime. Resume folds the buffer into view and resets that count; it
 * never touches the connection, so it can never cause a re-tail. (With history
 * on disk it also gives back any room a loaded page made: see `loadEarlier`.) A `hold`
 * from the screen freezes the view the same way, for as long as the reader
 * presses or selects in it, without touching their pause. A hidden document is
 * held too, in the one respect that matters there: lines still fold into the
 * ring (so it stays bounded) but nothing is shown, not even the pending count,
 * until the document is visible again.
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
 *
 * **History on disk (#914).** With `spoolMaxBytes` the cluster's stream keeps
 * every line on disk and sends them in frames that say which line each starts
 * at. The ring then holds one unbroken run of lines, and `dropped` is the
 * number of lines before it, so `loadEarlier` can read the page that ends where
 * the run starts and put it in front: see `onLines` and `loadEarlier`.
 */

/**
 * Where a log stream's lines come from. The cluster's own (`startLogStream`) is
 * the default; an app's pod binding (#567) opens its `logs` app stream with the
 * same callbacks (`startExtensionLogStream`), and a log provider (#569) will too,
 * which is what lets the pod log view pick among them.
 */
export type LogSourceOpener = (
  targets: LogTarget[],
  onLine: (source: string, line: string, truncated: boolean) => void,
  onStatus: (status: LogStatus, source: string) => void,
  options: LogStreamOptions,
) => Promise<LogStream>;

/**
 * What a caller can ask of the hook. `onLines` and `onSpoolFailed` are left out:
 * the hook sets them itself when it asks for history, so a caller's would be ignored (#914).
 */
export interface UseLogStreamOptions extends Omit<LogStreamOptions, "onLines" | "onSpoolFailed"> {
  /** How many lines the ring keeps before dropping the oldest. */
  capacity?: number;
  /**
   * Another source than the cluster's own. `key` names it: the stream reopens
   * when the key changes, never because a new `open` function arrived.
   */
  source?: { key: string; open: LogSourceOpener };
  /**
   * Keep the stream's history on disk, up to this many bytes (#914). Only the
   * cluster's own stream can; another `source` ignores it.
   */
  spoolMaxBytes?: number;
}

/** Connection health, plus the states `startLogStream` itself can't report:
 *  `"connecting"` before the first status or failure, and `"error"` when the
 *  connect promise rejected. */
export type LogStreamStatus = "connecting" | LogStatus | "error";

/** What is known about each of a stream's targets right now. `total` is the
 *  denominator for both counts; targets that have not reported yet are
 *  neither `live` nor `reconnecting`. */
export interface LogTargetCounts {
  readonly completed?: number;
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
  if (counts.total > 0 && (counts.completed ?? 0) >= counts.total) return "completed";
  if (counts.total > 0 && counts.live + (counts.completed ?? 0) >= counts.total) return "live";
  return "connecting";
}

export interface UseLogStreamResult {
  /** The visible lines, oldest first — frozen while `paused` or `held` is true. */
  lines: readonly LogLine[];
  /** How many lines the ring has dropped from the visible buffer. */
  dropped: number;
  status: LogStreamStatus;
  /** How many of this stream's targets are streaming right now. */
  liveTargets: number;
  /** Finite init-container logs that have been fully read. */
  completedTargets: number;
  /** How many are down and retrying — every one of them a gap in the tail. */
  reconnectingTargets: number;
  /** How many targets this stream follows: the denominator for both counts. */
  totalTargets: number;
  /** Set when `status` is `"error"` — why the stream could not be opened. */
  error?: FriendlyError;
  paused: boolean;
  /** Whether the screen is holding the view; see `hold`. */
  held: boolean;
  /** Lines the frozen view is not showing: those since the pause or hold froze it, and any that came just before. */
  pendingWhilePaused: number;
  togglePause: () => void;
  /**
   * Freeze the view (`true`) or let it go (`false`) while the reader presses or selects in the log, so the line
   * under them is not moved or rewritten mid-click or mid-copy. Apart from `paused`, which is the reader's own
   * choice and which this never changes: the view moves only while neither holds it.
   */
  hold: (on: boolean) => void;
  /** Empty the buffer and view without touching the connection. */
  clear: () => void;
  /**
   * Bumped every time a target/since/tailLines change forces a restart after
   * the first connect. A screen watching this rise is how it knows to say
   * scrollback was cleared, rather than the clear passing unremarked.
   */
  restartCount: number;
  /**
   * The stream's history on disk, once its first frame says where it starts:
   * `first` is the oldest line still there, and `failed` why it stopped. With
   * history, `dropped` counts every line before the first one in memory, so
   * `dropped + 1 - first` are on disk and loadable.
   */
  history?: { first: number; failed?: string };
  /** Read the page before the oldest line shown, from the history on disk. */
  loadEarlier: () => Promise<void>;
  loadingEarlier: boolean;
  /** Why the last `loadEarlier` showed nothing, if it did not. */
  loadError?: unknown;
  /** Save the stream's history: all, or the last `sinceSeconds`. Only with history. */
  exportSpool?: (filename: string, sinceSeconds?: number) => Promise<string | null>;
}

const DEFAULT_CAPACITY = 5000;
/** How many earlier lines one "Load earlier" reads back (#914). */
export const EARLIER_PAGE = 5000;
/**
 * The most lines the view holds once earlier pages are loaded.
 * ponytail: a fixed ceiling; raise it once the throttled facets are measured over 50k.
 */
export const MAX_LOADED_LINES = 50_000;

/**
 * Run `fn` before the next frame is drawn: a commit is a render of the whole screen, and the backend can deliver a
 * line per IPC event, many times between two frames. A hidden document draws no frames and would hold every line
 * back, unfolded, until it is shown again, so it, and a host without frames, get a timer of about one frame instead
 * (the hidden one's commit only folds: see `commit`). Returns what takes the request back.
 */
function nextFrame(fn: () => void): () => void {
  if (typeof requestAnimationFrame === "function" && !document.hidden) {
    const frame = requestAnimationFrame(fn);
    return () => cancelAnimationFrame(frame);
  }
  const timer = setTimeout(fn, 16);
  return () => clearTimeout(timer);
}

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
  const spoolMaxBytes = options.spoolMaxBytes;
  const key = targetsKey(targets);
  const sourceKey = options.source?.key ?? "";
  // Read at connect time, so a caller that passes a fresh function each render
  // does not restart the stream; the key does.
  const openRef = useRef(options.source?.open);
  openRef.current = options.source?.open;

  const bufferRef = useRef(createLogBuffer(capacity));
  /** Lines that arrived since the last fold, oldest first: the ring is copied once per batch, not once per line. */
  const batchRef = useRef<LogLine[]>([]);
  const pausedRef = useRef(false);
  const heldRef = useRef(false);
  /** Takes back the commit asked for, while one is. */
  const cancelCommitRef = useRef<(() => void) | undefined>(undefined);
  const firstRunRef = useRef(true);
  /** The open stream, for reading and exporting its history. */
  const streamRef = useRef<LogStream | undefined>(undefined);
  /** Bumped whenever a stream goes (a reconnect, or the screen), so a read that outlives its stream is ignored. */
  const generationRef = useRef(0);
  /** A read of earlier lines is out, so a second request waits for it instead of reading the same page again. */
  const loadingRef = useRef(false);

  const [view, setView] = useState(() => bufferRef.current);
  /** The ring as last shown, for counting what has come since: what a commit reads, `view` may not be yet. */
  const shownRef = useRef(view);
  const [paused, setPaused] = useState(false);
  const [held, setHeld] = useState(false);
  const [pendingWhilePaused, setPendingWhilePaused] = useState(0);
  const [counts, setCounts] = useState<LogTargetCounts>(NO_TARGETS);
  const [error, setError] = useState<FriendlyError | undefined>(undefined);
  const [restartCount, setRestartCount] = useState(0);
  const [history, setHistory] = useState<{ first: number; failed?: string } | undefined>(undefined);
  const [loadingEarlier, setLoadingEarlier] = useState(false);
  const [loadError, setLoadError] = useState<unknown>(undefined);
  const [canExport, setCanExport] = useState(false);

  const fold = useCallback(() => {
    if (batchRef.current.length === 0) return;
    bufferRef.current = appendLogLines(bufferRef.current, batchRef.current);
    batchRef.current = [];
  }, []);

  const show = useCallback((buffer: LogBuffer) => {
    shownRef.current = buffer;
    setPendingWhilePaused(0);
    setView(buffer);
  }, []);

  /** Fold what is waiting and show the whole ring: the end of the last pause or hold. */
  const reveal = useCallback(() => {
    fold();
    // Nobody is looking: the document being shown again is what shows it (below).
    if (document.hidden) return;
    show(bufferRef.current);
  }, [fold, show]);

  const commit = useCallback(() => {
    cancelCommitRef.current = undefined;
    // Folded while paused, held or hidden too: they freeze the view, not the ring.
    fold();
    // A hidden document measures its viewport at 0px, so the screen draws every row of the ring: 50,000 rows a frame
    // for nobody starved the browser until the stream's socket queue overflowed. Nothing is set either, not even the
    // count of what waits: that is a render too, and it is read off the ring when the document is shown again.
    if (document.hidden) return;
    if (pausedRef.current || heldRef.current) {
      // Every line the ring has taken since the view froze, those it has since dropped among them, and those that
      // came before the freeze but were not shown yet: counted off the ring, not as they arrive.
      const taken = (buffer: LogBuffer) => buffer.dropped + buffer.lines.length;
      setPendingWhilePaused(taken(bufferRef.current) - taken(shownRef.current));
      return;
    }
    show(bufferRef.current);
  }, [fold, show]);

  const scheduleCommit = useCallback(() => {
    cancelCommitRef.current ??= nextFrame(commit);
  }, [commit]);

  // A commit still asked for when the screen goes would fold lines into a ring nothing reads.
  useEffect(
    () => () => {
      cancelCommitRef.current?.();
      cancelCommitRef.current = undefined;
    },
    [],
  );

  // The document is shown again: bring the view up to date at once, rather than wait out a frame that a hidden
  // document may have kept from running. Paused or held, `commit` leaves the view as it is and counts what waits.
  useEffect(() => {
    const onVisibility = () => {
      if (document.hidden) return;
      cancelCommitRef.current?.();
      commit();
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => document.removeEventListener("visibilitychange", onVisibility);
  }, [commit]);

  // Stable across restarts: the same ref keeps accumulating, so a target
  // change doesn't need a new callback, only a fresh buffer (below).
  const onLine = useCallback(
    (source: string, text: string, truncated = false) => {
      // Marked only when the host cut it (#747), so a whole line is `{source, text}`.
      const line = truncated ? { source, text, truncated: true as const } : { source, text };
      batchRef.current.push(line);
      // A ring's worth waiting is folded now: whatever holds the commit back, the backlog stays bounded.
      if (batchRef.current.length >= bufferRef.current.capacity) fold();
      scheduleCommit();
    },
    [fold, scheduleCommit],
  );

  /**
   * A frame from a stream with history (#914). The ring always holds one
   * unbroken run of lines, and `dropped` counts every line before it, which
   * is what `loadEarlier` and the banner read. So a frame that starts past the
   * next line expected (the backend kept the ones between only on disk)
   * starts the ring again at the frame.
   */
  const onLines = useCallback(
    (lines: LogLine[], frame: LogFrame) => {
      const buffer = bufferRef.current;
      const expected = buffer.dropped + buffer.lines.length + batchRef.current.length + 1;
      if (frame.seq > expected) {
        // The lines waiting come before the gap and are on disk, and the ring they would be folded into is replaced
        // just below: `dropped` already counts them, so they are let go rather than copied in first.
        batchRef.current = [];
        bufferRef.current = { capacity: bufferRef.current.capacity, lines: [], dropped: frame.seq - 1 };
      }
      const first = frame.first;
      if (first !== undefined) {
        setHistory((h) => (h !== undefined && h.first === first && h.failed === undefined ? h : { first }));
      }
      batchRef.current.push(...lines);
      if (batchRef.current.length >= bufferRef.current.capacity) fold();
      scheduleCommit();
    },
    [fold, scheduleCommit],
  );

  /** Save the open stream's history. One function for the hook's life, so a frame never hands the screen a new one. */
  const exportSpool = useCallback(
    (filename: string, sinceSeconds?: number) =>
      streamRef.current?.exportSpool?.(filename, sinceSeconds) ?? Promise.resolve(null),
    [],
  );

  const togglePause = useCallback(() => {
    const next = !pausedRef.current;
    pausedRef.current = next;
    setPaused(next);
    if (!next) {
      // Pages loaded while paused (#914) took the ring past its capacity, and left there it would stay that big at
      // frame rate. Resuming is the reader going back to the live tail, so the ring is as wide as asked again: the
      // newest lines stay, and the ones let go are on disk, which `dropped` says. A hold's release is no resume.
      const b = bufferRef.current;
      if (b.capacity > capacity) {
        const extra = Math.max(0, b.lines.length - capacity);
        bufferRef.current = { capacity, lines: extra > 0 ? b.lines.slice(extra) : b.lines, dropped: b.dropped + extra };
      }
    }
    // Resuming folds the accumulated buffer into view in one step — never
    // a re-tail, since the connection is untouched by this branch — unless
    // the screen is holding the view, whose release does it instead.
    if (!next && !heldRef.current) reveal();
  }, [reveal, capacity]);

  const hold = useCallback(
    (on: boolean) => {
      if (heldRef.current === on) return;
      heldRef.current = on;
      setHeld(on);
      if (!on && !pausedRef.current) reveal();
    },
    [reveal],
  );

  const clear = useCallback(() => {
    const buffer = bufferRef.current;
    // With history the cleared lines are on disk, not gone (#914): they move
    // into `dropped`, so the banner can still offer them.
    const kept = streamRef.current?.readEarlier !== undefined;
    bufferRef.current = kept
      ? { capacity, lines: [], dropped: buffer.dropped + buffer.lines.length + batchRef.current.length }
      : clearLogBuffer(buffer);
    batchRef.current = [];
    show(bufferRef.current);
  }, [show, capacity]);

  /**
   * Read the page before the oldest line shown and put it in front (#914).
   *
   * Following, the ring grows by a page first, so lines arriving during the
   * read do not push the oldest line shown out from under it, and takes that
   * room back if the read fails or its page does not join, so a stream that
   * keeps failing does not leave the ring at its ceiling. While paused
   * or held, the page goes into the frozen view, and into the ring too if
   * the ring still starts where the view does. On resume the ring goes back to
   * its capacity (the oldest lines it let go are on disk again, in `dropped`)
   * and the view shows it, as ever; a hold's release shrinks nothing. A page that no longer ends where the lines in memory start
   * would leave a hole, so it is not shown, and `loadError` says why. Asked
   * again while a read is out, it does nothing: the same page would come back
   * and no longer join.
   */
  const loadEarlier = useCallback(async () => {
    const stream = streamRef.current;
    if (!stream?.readEarlier || loadingRef.current) return;
    fold();
    const isFrozen = () => pausedRef.current || heldRef.current;
    const from = isFrozen() ? shownRef.current : bufferRef.current;
    const before = from.dropped + 1;
    const room = MAX_LOADED_LINES - from.lines.length;
    if (room <= 0) return;
    const generation = generationRef.current;
    /** Room for `extra` more lines and a page above them, so live lines do not push a loaded page straight out. */
    const roomFor = (b: LogBuffer, extra: number) =>
      Math.min(MAX_LOADED_LINES, b.lines.length + extra + EARLIER_PAGE);
    /** Takes the room back when the read comes to nothing, so failing reads cannot ratchet the ring up to its ceiling. */
    let giveBackRoom = () => {};
    if (!isFrozen()) {
      const b = bufferRef.current;
      const raised = Math.max(b.capacity, roomFor(b, EARLIER_PAGE));
      bufferRef.current = { ...b, capacity: raised };
      giveBackRoom = () => {
        if (generation !== generationRef.current) return;
        fold();
        const now = bufferRef.current;
        // Only this read's own raise is undone: a clear, or a gap in the frames, has since set the ring afresh.
        if (now.capacity !== raised) return;
        // Never below what the ring holds now: the lines that came while the page was read stay.
        bufferRef.current = { ...now, capacity: Math.max(b.capacity, now.lines.length) };
      };
    }
    loadingRef.current = true;
    setLoadingEarlier(true);
    setLoadError(undefined);
    let page: SpoolPage;
    try {
      page = await stream.readEarlier(before, Math.min(EARLIER_PAGE, room));
    } catch (e) {
      // A read that outlived its stream leaves the flag to whoever reset it: it may be another stream's now.
      if (generation === generationRef.current) {
        loadingRef.current = false;
        giveBackRoom();
        setLoadError(e);
        setLoadingEarlier(false);
      }
      return;
    }
    if (generation !== generationRef.current) return;
    loadingRef.current = false;
    setLoadingEarlier(false);
    setHistory((h) => (h === undefined || h.first === page.first ? h : { ...h, first: page.first }));
    if (page.lines.length === 0) {
      giveBackRoom();
      return;
    }
    const end = page.seq + page.lines.length;
    const joins = (b: LogBuffer) => b.dropped + 1 === end;
    // Asked again: the reader may have paused, or let go, while the page was read.
    if (isFrozen()) {
      if (!joins(shownRef.current)) {
        giveBackRoom();
        setLoadError(new Error("Those lines moved on while they were read. Load them again."));
        return;
      }
      if (joins(bufferRef.current)) {
        bufferRef.current = prependLogLines(
          bufferRef.current,
          page.lines,
          roomFor(bufferRef.current, page.lines.length),
        );
      } else {
        giveBackRoom();
      }
      shownRef.current = prependLogLines(shownRef.current, page.lines, shownRef.current.capacity);
      setView(shownRef.current);
      return;
    }
    fold();
    if (!joins(bufferRef.current)) {
      giveBackRoom();
      setLoadError(
        new Error("The stream moved past those lines while they were read. Pause it, then load them again."),
      );
      return;
    }
    bufferRef.current = prependLogLines(bufferRef.current, page.lines, roomFor(bufferRef.current, page.lines.length));
    show(bufferRef.current);
  }, [fold, show]);

  useEffect(() => {
    let cancelled = false;
    let stopFn: (() => void) | undefined;

    bufferRef.current = createLogBuffer(capacity);
    batchRef.current = [];
    show(bufferRef.current);
    // A restart knows nothing about the new targets until they report; the
    // old targets' states are not evidence about these ones.
    const sources = new Set(targets.map((t) => t.label ?? ""));
    setCounts({ live: 0, reconnecting: 0, total: sources.size });
    setError(undefined);
    streamRef.current = undefined;
    setHistory(undefined);
    setLoadError(undefined);
    setLoadingEarlier(false);
    setCanExport(false);

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
    const guardedOnLine = (source: string, text: string, truncated: boolean) => {
      if (cancelled) return;
      onLine(source, text, truncated);
    };

    // History is the cluster's own stream's to keep (#914); another source
    // ignores the request, so it is not made.
    const streamOptions: LogStreamOptions = { timestamps, sinceSeconds, tailLines };
    if (spoolMaxBytes !== undefined && !openRef.current) {
      streamOptions.spoolMaxBytes = spoolMaxBytes;
      streamOptions.onLines = (lines, frame) => {
        if (!cancelled) onLines(lines, frame);
      };
      streamOptions.onSpoolFailed = (reason) => {
        if (!cancelled) setHistory((h) => ({ first: h?.first ?? 1, failed: reason }));
      };
    }

    const open: LogSourceOpener =
      openRef.current ??
      ((targets, line, status, options) => startLogStream(context, namespace, targets, line, status, options));
    open(
      streamTargets,
      guardedOnLine,
      (s, source) => {
        if (cancelled) return;
        seen.set(source, s);
        let live = 0;
        let reconnecting = 0;
        let completed = 0;
        for (const state of seen.values()) {
          if (state === "live") live += 1;
          else if (state === "completed") completed += 1;
          else reconnecting += 1;
        }
        setCounts({ live, reconnecting, completed, total });
      },
      streamOptions,
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
        streamRef.current = stream;
        setCanExport(stream.exportSpool !== undefined);
      },
      (e: unknown) => {
        if (cancelled) return;
        setError(describeError(e));
      },
    );

    return () => {
      cancelled = true;
      // A read still out for this stream is no longer wanted, on a restart or once the screen is gone.
      generationRef.current += 1;
      loadingRef.current = false;
      streamRef.current = undefined;
      stopFn?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    context,
    namespace,
    key,
    sourceKey,
    sinceSeconds,
    tailLines,
    timestamps,
    capacity,
    spoolMaxBytes,
    onLine,
    onLines,
    show,
  ]);

  return useMemo(
    () => ({
      lines: view.lines,
      dropped: view.dropped,
      // The failure to OPEN a stream is the hook's own, not a target's, and
      // outranks whatever the targets last said.
      status: error !== undefined ? "error" : aggregateLogStatus(counts),
      liveTargets: counts.live,
      reconnectingTargets: counts.reconnecting,
      completedTargets: counts.completed ?? 0,
      totalTargets: counts.total,
      error,
      paused,
      held,
      pendingWhilePaused,
      togglePause,
      hold,
      clear,
      restartCount,
      history,
      loadEarlier,
      loadingEarlier,
      loadError,
      exportSpool: canExport ? exportSpool : undefined,
    }),
    [
      view,
      counts,
      error,
      paused,
      held,
      pendingWhilePaused,
      togglePause,
      hold,
      clear,
      restartCount,
      history,
      loadEarlier,
      loadingEarlier,
      loadError,
      canExport,
      exportSpool,
    ],
  );
}
