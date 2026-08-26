import { describeError, startHelmOp } from "@srelens/core";

/**
 * The helm operations this window has started — what each one is doing, every
 * line it has printed, and how it ended.
 *
 * **Why a store and not a modal.** `helm upgrade --wait` runs for minutes, and
 * a dialog held open for the duration stops the reader looking at the pods it
 * is restarting. The dialog submits and closes; this owns what happens next,
 * so the release list, the strip and the pane can all read the same operation
 * without any of them having to be on screen when it finishes.
 *
 * **Nothing here cancels anything, and no name in this file may suggest it.**
 * A shell dies with its process and a tunnel dies with its socket, so their
 * stores can honestly offer to end one. A `helm upgrade` is a cluster mutation
 * that continues whether srelens watches or not: core's `close()` unsubscribes
 * this window from the stream and touches the release not at all. That is why
 * {@link dismissHelmOp} declines to remove a `running` row — a row that
 * disappeared under a dismiss button would read as a cancel, and it is not
 * one.
 *
 * **No status word or tone lives here.** This module holds state; the words
 * for it come from core, the same way every other screen in this migration
 * gets them.
 *
 * Shaped after `sessions.ts`, which is shaped after `packages/core/src/lib/
 * forward.ts`: module-level state, a listener set, and a snapshot that keeps
 * its reference until something in it actually changed, so
 * `useSyncExternalStore` has a stable value to compare.
 */

/** Which of the four operations a row is. */
export type HelmOpKind = "install" | "upgrade" | "rollback" | "uninstall";

/**
 * How an operation stands.
 *
 * `running` until the backend says otherwise — including after the reader has
 * closed every view of it, because the cluster is still changing. `done` and
 * `failed` are both final, and both stay listed until dismissed.
 */
export type HelmOpState = "running" | "done" | "failed";

/** One operation, as the pane and the strip read it. */
export interface HelmOpRow {
  id: number;
  kind: HelmOpKind;
  release: string;
  namespace: string;
  context: string;
  state: HelmOpState;
  /**
   * Every line the operation has printed, oldest first.
   *
   * Lines are coalesced into the snapshot at second resolution — see
   * {@link receive}. A finished operation's output is complete regardless:
   * {@link settle} flushes whatever the current second was still holding.
   */
  output: string[];
  /**
   * Why it failed. Already through `describeError`: a failed upgrade's reason
   * is read straight off the row by whatever renders it, so a raw Rust string
   * is turned into a sentence here rather than at each reader.
   */
  error?: string;
  /** Epoch millis when the operation was started. */
  startedAt: number;
}

/** What starting an operation needs. `args` is the helm argv exactly as core
 *  will run it — this store composes no commands, it only owns the one it was
 *  handed. */
export interface HelmOpRequest {
  kind: HelmOpKind;
  release: string;
  namespace: string;
  context: string;
  args: string[];
  /** Extra kubeconfigs to put on helm's KUBECONFIG. */
  extraKubeconfigs?: string[];
  /** A values.yaml body, for the operations that take one. */
  values?: string;
}

let ops: HelmOpRow[] = [];
const listeners = new Set<() => void>();
/** The stream subscription for each operation still being watched. */
const handles = new Map<number, { close: () => void }>();
/** Lines that have arrived but are not in the snapshot yet — see
 *  {@link receive}. */
const pending = new Map<number, string[]>();
/** The whole second each operation's output was last folded into the snapshot
 *  at. A line landing in a later second may rebuild the row; one landing in
 *  the same second waits. */
const lastFlush = new Map<number, number>();
/** The timer that lands a buffered tail when the output stops mid-second, so
 *  the last thing a quiet operation said is never held back. */
const flushTimers = new Map<number, ReturnType<typeof setTimeout>>();

/** Ids are the store's own, not the backend's channel numbers. */
let seq = 0;

function emit() {
  for (const listener of listeners) listener();
}

/** Subscribe to store changes (for `useSyncExternalStore`). */
export function subscribeHelmOps(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/**
 * Every operation this window has started — running and finished.
 *
 * A stable reference until something in it changes: `useSyncExternalStore`
 * compares snapshots by identity and re-renders forever when handed a fresh
 * array each read, and a streamed operation prints often enough to make that
 * a hang rather than a waste.
 */
export function getHelmOps(): HelmOpRow[] {
  return ops;
}

/**
 * Start one of the four operations and watch it.
 *
 * Never throws: an operation the backend refused to start is a `failed` row
 * carrying the reason, because a failure the reader can read is worth more
 * than one they have to catch. Returns the row's id either way.
 */
export async function startHelmOperation(req: HelmOpRequest): Promise<number> {
  const id = register(req);
  let handle: { close: () => void };
  try {
    handle = await startHelmOp(
      req.context,
      req.args,
      (line) => receive(id, line),
      (err) => settle(id, err),
      req.extraKubeconfigs ?? [],
      req.values ?? "",
    );
  } catch (e) {
    // Raw, not described: `settle` describes exactly once, and a sentence run
    // through `describeError` twice is classified on its own wording.
    settle(id, e);
    return id;
  }
  // Dismissed while the start was still in flight — there is no row left to
  // report into, so stop watching what just opened rather than leaking a
  // subscription. The operation itself carries on; nothing here could stop it.
  if (!ops.some((o) => o.id === id)) {
    handle.close();
    return id;
  }
  handles.set(id, handle);
  return id;
}

/**
 * The reader is done with a finished operation: drop the row.
 *
 * The one place a row leaves the screen, and only for a `done` or `failed`
 * one. An operation that ended on its own stays listed with its output —
 * #349's vanishing port-forward is what happens otherwise, a reader carrying
 * on as though a dead thing were fine, and a failed upgrade's output is the
 * only place its reason exists.
 *
 * A `running` row is left exactly where it is. See this module's note: there
 * is no cancel to offer, so there is no removal to honour either.
 */
export function dismissHelmOp(id: number): void {
  const row = ops.find((o) => o.id === id);
  if (!row || row.state === "running") return;
  forget(id);
  commit(ops.filter((o) => o.id !== id));
}

/** Reset the module-level store between tests. */
export function __resetHelmOpsForTests(): void {
  for (const id of [...handles.keys()]) forget(id);
  for (const id of [...flushTimers.keys()]) forget(id);
  ops = [];
  listeners.clear();
  pending.clear();
  lastFlush.clear();
  seq = 0;
}

/**
 * Put the row in place, before anything is started.
 *
 * Deliberately ahead of the backend call: `startHelmOp` resolves only after a
 * round trip, and output can arrive the moment it does. A row built after the
 * await would be built after the first line had nowhere to land.
 */
function register(req: HelmOpRequest): number {
  const id = ++seq;
  const now = Date.now();
  ops = [
    ...ops,
    {
      id,
      kind: req.kind,
      release: req.release,
      namespace: req.namespace,
      context: req.context,
      state: "running",
      output: [],
      startedAt: now,
    },
  ];
  emit();
  return id;
}

/**
 * A line from the operation.
 *
 * Buffered rather than committed, and folded into the snapshot at most once a
 * second. `helm upgrade --wait` narrates itself continuously while it waits on
 * a rollout, and a store that rebuilt its array for every line would wake
 * every `useSyncExternalStore` subscriber several times a second — the same
 * problem millisecond stamps caused in `sessions.ts`, arriving here through
 * the output array instead of a timestamp.
 *
 * The first line of any second lands at once, so an operation that has just
 * started printing is visibly printing; the rest of that second's lines wait
 * together. Nothing is lost by waiting: {@link scheduleFlush} lands the tail
 * when the output stops, and {@link settle} lands it immediately when the
 * operation ends.
 */
function receive(id: number, line: string) {
  const buffered = pending.get(id);
  if (buffered) buffered.push(line);
  else pending.set(id, [line]);
  if (lastFlush.get(id) === wholeSecond(Date.now())) scheduleFlush(id);
  else flush(id);
}

/** Land the tail of a mid-second burst once the second turns. Only ever one
 *  timer per operation: a later line in the same second joins the buffer the
 *  pending timer will flush. */
function scheduleFlush(id: number) {
  if (flushTimers.has(id)) return;
  const now = Date.now();
  flushTimers.set(
    id,
    setTimeout(() => flush(id), wholeSecond(now) + 1000 - now),
  );
}

/** Fold whatever this operation has buffered into the snapshot. */
function flush(id: number) {
  clearTimeout(flushTimers.get(id));
  flushTimers.delete(id);
  const buffered = pending.get(id);
  pending.delete(id);
  lastFlush.set(id, wholeSecond(Date.now()));
  if (!buffered || buffered.length === 0) return;
  commit(ops.map((o) => (o.id === id ? { ...o, output: [...o.output, ...buffered] } : o)));
}

/**
 * The operation is over. The row STAYS, `done` or `failed`, carrying its
 * output and — when it failed — why.
 *
 * `reason` is whatever the backend gave; a clean exit gives nothing. A row
 * that has already ended is left alone rather than rebuilt, so the store does
 * not wake its subscribers for an ending it already knows about.
 */
function settle(id: number, reason: unknown) {
  flush(id);
  forget(id);
  const error = describedReason(reason);
  commit(
    ops.map((o) => {
      if (o.id !== id) return o;
      const state = error ? "failed" : "done";
      if (o.state === state && o.error === error) return o;
      return { ...o, state, error };
    }),
  );
}

/**
 * Stop watching this operation and drop its buffers. Idempotent.
 *
 * Watching is all this ends. `close()` unsubscribes from the stream; the helm
 * process and the cluster mutation behind it are entirely unaffected.
 */
function forget(id: number) {
  clearTimeout(flushTimers.get(id));
  flushTimers.delete(id);
  pending.delete(id);
  handles.get(id)?.close();
  handles.delete(id);
}

/** An epoch stamp at second resolution. See {@link receive}. */
function wholeSecond(millis: number): number {
  return Math.floor(millis / 1000) * 1000;
}

/** A reason worth showing, said the way a reader can use it — a backend
 *  string, or a thrown value from an operation that never started.
 *  `describeError` strips the wrappers either arrives in and classifies what
 *  is left; nothing at all, or an empty reason, is not a sentence and stays
 *  nothing, which is what a clean exit gives. */
function describedReason(reason: unknown): string | undefined {
  if (reason === null || reason === undefined) return undefined;
  if (typeof reason === "string" && reason.trim() === "") return undefined;
  return describeError(reason).detail;
}

/**
 * The one way the snapshot changes: take a rebuilt list only if some row in it
 * actually changed, or a row left it. Identity is the snapshot's contract, and
 * a no-op assignment breaks it — but so does a length change that every
 * surviving row happens to match, which is what dropping the LAST row looks
 * like to a positional comparison alone.
 */
function commit(next: HelmOpRow[]) {
  if (next.length === ops.length && !next.some((o, i) => o !== ops[i])) return;
  ops = next;
  emit();
}
