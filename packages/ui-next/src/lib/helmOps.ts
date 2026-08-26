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
 * partway through, and there is no honest way to take it back — which is why
 * {@link dismissHelmOp} declines to remove a `running` row: a row that
 * disappeared under a dismiss button would read as a cancel, and it is not
 * one.
 *
 * **`close()` is not a quiet unsubscribe. It kills helm.** The handle core
 * hands back invokes `helm_op_close`, which aborts the tokio task owning the
 * `tokio::process::Child` — and that child is spawned `kill_on_drop(true)`, so
 * dropping the aborted future SIGKILLs helm wherever it had reached. A release
 * left half-upgraded, its Secret mid-write, and no output saying why, because
 * the process that would have said it is gone. `crates/streams/src/helm.rs`
 * calls it "best-effort abort" and means exactly that.
 *
 * **So the rule this module keeps: never close a handle whose row is still
 * `running`.** Every call obeys it today — {@link settle} closes after the
 * process has already exited, {@link dismissHelmOp} declines a `running` row
 * outright, the late-handle guard in {@link startHelmOperation} closes only a
 * row that has already settled, and `__resetHelmOpsForTests` runs under vitest
 * alone. Nothing in a running app reaches {@link forget} while an operation is
 * in flight, and that is a property to keep rather than one to rely on
 * accidentally: a new caller of `forget`, or a new export of any kind, must
 * check the row's state and leave a `running` one watching. There is no "just
 * stop listening" to reach for here — from this side, stopping listening IS
 * the kill. `helmOps.test.ts` pins both halves: that no live entry point
 * closes a running handle, and that the export list is the one that claim was
 * traced across.
 *
 * (The backend's `HelmManager::shutdown_all` aborts the same way when a user's
 * environment is dropped. That is out of this module's hands and deliberate
 * there; it is not licence to do it from here.)
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
  // The row already ended, or is gone, while the start was still in flight.
  // Core registers both stream listeners BEFORE it invokes, so an operation
  // that fails instantly — `helm uninstall` of a release that is not there —
  // can reach `settle` ahead of this await; `forget` ran then and found no
  // handle to close, so storing this one would park a subscription nothing
  // ever closes. (A row that has left entirely is the test reset's doing:
  // `dismissHelmOp` declines every `running` row, and the row is `running` for
  // this whole window.) Closing here is safe for the one reason closing is
  // ever safe in this module: the guard runs only when the row has already
  // settled, so the helm process this would kill has already exited. A
  // `running` row keeps its handle and falls through.
  const row = ops.find((o) => o.id === id);
  if (!row || row.state !== "running") {
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

/**
 * Reset the module-level store between tests.
 *
 * The one place that closes a `running` operation's handle, and therefore the
 * one exception to this module's rule. It is safe only because it never runs
 * outside vitest, where the handle is a double and no helm process exists to
 * kill. Do not call it from app code, and do not copy its shape into a
 * "clear everything" the shell could reach.
 */
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
 * The first line of any second lands at once — including the second right
 * after a timer-driven flush, which is charged to the second it closed, not
 * the one it woke in — so an operation that is printing keeps looking like it
 * is. The rest of each second's lines wait together. Nothing is lost by
 * waiting: {@link scheduleFlush} lands the tail when the output stops, and
 * {@link settle} lands it immediately when the operation ends.
 */
function receive(id: number, line: string) {
  const buffered = pending.get(id);
  if (buffered) buffered.push(line);
  else pending.set(id, [line]);
  if (lastFlush.get(id) === wholeSecond(Date.now())) scheduleFlush(id);
  else flush(id);
}

/**
 * Land the tail of a mid-second burst once the second turns.
 *
 * One timer per operation, keyed by id — a shared timer would let one
 * operation's flush cancel another's and strand its buffered lines with
 * nothing left to land them. A later line in the same second joins the buffer
 * this timer will flush.
 *
 * The second being closed is captured here rather than read back when the
 * timer fires: the timer fires ON the boundary, and a flush that stamped the
 * second it woke up in would spend a budget nothing had used yet, leaving the
 * first line of the new second invisible for another whole second.
 */
function scheduleFlush(id: number) {
  if (flushTimers.has(id)) return;
  const now = Date.now();
  const second = wholeSecond(now);
  flushTimers.set(
    id,
    setTimeout(() => flush(id, second), second + 1000 - now),
  );
}

/**
 * Fold whatever this operation has buffered into the snapshot, and record the
 * second that fold is charged to.
 *
 * `second` is the second the flushed lines belong to. It defaults to the
 * current one — right for a flush a line or an ending drove — and
 * {@link scheduleFlush} passes the older, closing second instead.
 */
function flush(id: number, second: number = wholeSecond(Date.now())) {
  clearTimeout(flushTimers.get(id));
  flushTimers.delete(id);
  const buffered = pending.get(id);
  pending.delete(id);
  lastFlush.set(id, second);
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
  // Safe to close here, and only here by default: `onExit` fired, so the helm
  // process is gone and there is nothing left for the abort to kill.
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
 * Let go of this operation: drop its buffers and close its stream handle.
 * Idempotent.
 *
 * **Callers must have established that the operation is no longer `running`.**
 * `close()` kills the helm process — see the rule in this module's header —
 * so this is a function for an operation that has already ended, not a way to
 * stop caring about one that has not. The three callers that reach a live row
 * each check first, and the fourth is the test reset.
 */
function forget(id: number) {
  clearTimeout(flushTimers.get(id));
  flushTimers.delete(id);
  pending.delete(id);
  // Ids never repeat, so a stamp left behind here is one dead number kept for
  // the life of the window per operation ever started.
  lastFlush.delete(id);
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
