import { Terminal } from "@xterm/xterm";
import {
  deletePod,
  describeError,
  notify,
  startLocalTerminal,
  startPodExec,
  type TerminalConnection,
} from "@srelens/core";

/**
 * The terminal sessions this window is running — what is live, the emulator
 * each one is writing into, and the cleanup each one owes.
 *
 * **Why this store is in `ui-next` and not in core**, where port-forwards keep
 * theirs: it holds the xterm instance. An emulator created by the pane would
 * die with the pane, and a reader who closed the Terminals tab and came back
 * would find an attached shell with no scrollback — a shell whose last command
 * it cannot show them. Sessions outliving the screen means nothing unless the
 * transcript outlives it too. core holds no DOM (`no-react.test.ts` walks its
 * import graph), so the owner has to live here, next to the emulator, holding
 * core's session handle beside it.
 *
 * **The whole data path is wired here.** The store writes the backend's output
 * into the emulator, and wires the emulator's input and resize back to the PTY.
 * A component that attaches the emulator to the DOM therefore does not have to
 * hold a session handle at all — {@link terminalFor} is the only thing it
 * needs, and an unmount that forgets to unwire something cannot cut a live
 * shell's keystrokes.
 *
 * **`terminalFor` hands out an instance the caller must not dispose.** The
 * store disposes it, in {@link endSession}, and only there.
 *
 * Shaped after `packages/core/src/lib/forward.ts`: module-level state, a
 * listener set, and a snapshot that keeps its reference until something in it
 * actually changed, so `useSyncExternalStore` has a stable value to compare.
 */

/** What kind of shell a session is. `node` is a pod exec into the privileged
 *  debug pod srelens created for a node — the store deletes that pod as soon
 *  as the session is over, whether the far end went on its own or the reader
 *  dismissed the row (see {@link takeDebugPod}), which is the cleanup a pod
 *  exec and a local shell have nothing to do. */
export type SessionKind = "pod" | "node" | "local";

/**
 * A session's live state.
 *
 * `attached` and `idle` are both RUNNING — the difference is whether the shell
 * has said anything lately (see {@link SESSION_IDLE_AFTER_MS}), which is what
 * the rail's state dot is reporting. Only `closed` means the far end is gone.
 *
 * No word or tone for these lives here. core has no verdict for a shell's
 * state — `k8sStatus`/`k8sHealth` speak about Kubernetes resources and
 * `logConnectionStatus` about a log stream's connection — and this module
 * holds state, not vocabulary.
 */
export type SessionState = "attached" | "idle" | "closed";

/** One session, as a rail row reads it. */
export interface TerminalSessionRow {
  id: number;
  kind: SessionKind;
  /** "checkout-api-5c8b7f2d9-mk3wl · api" — the pod and the container it
   *  attached to, or the local shell's own name. */
  title: string;
  context: string;
  /** Empty for a local shell, which is not in a namespace. */
  namespace: string;
  state: SessionState;
  /**
   * Why it ended, when it ended badly. Already through `describeError`: a
   * session's reason is read straight off the row by whatever renders it, so
   * a raw Rust string turned into a sentence here rather than at each reader.
   */
  error?: string;
  /** Epoch millis when this session was opened. */
  startedAt: number;
  /**
   * Epoch millis of the last output — what the rail's idle time counts from.
   * Stamped at start, so a shell that has never printed a prompt still ages
   * from somewhere real.
   *
   * Rounded down to the second, and that is load-bearing rather than tidy: a
   * shell tailing a log emits many chunks a second, and a row rebuilt on each
   * one would hand `useSyncExternalStore` a new snapshot on every byte. At
   * second resolution a busy session rebuilds its row once a second — the
   * same rate the forwards store accepts for its byte counter — and an idle
   * time measured in seconds loses nothing by it.
   */
  lastOutputAt: number;
}

/** How long a running session must stay quiet before it reads as `idle`.
 *  A shell with a prompt sitting on it is doing nothing; a shell that printed
 *  a line a second ago is being used. */
export const SESSION_IDLE_AFTER_MS = 60_000;

/** What a pod (or node) shell needs to open. */
export interface PodSessionRequest {
  context: string;
  namespace: string;
  pod: string;
  /** The container to attach to; the pod's default when omitted. */
  container?: string;
  /** Overrides the shell the backend would pick — a node shell's `nsenter`. */
  command?: string[];
  /** `node` for a shell that reached a node through a debug pod. */
  kind?: "pod" | "node";
  /** Overrides the title the pod and container would compose. */
  title?: string;
}

/** What a local shell needs to open. */
export interface LocalSessionRequest {
  context: string;
  /** Extra kubeconfigs to put on the shell's KUBECONFIG. */
  extraKubeconfigs?: string[];
  title?: string;
}

let sessions: TerminalSessionRow[] = [];
const listeners = new Set<() => void>();
/** The emulators this store owns, one per session, alive until the reader
 *  removes the row. */
const emulators = new Map<number, Terminal>();
/** The far end of each session, for as long as there is one to talk to. */
const handles = new Map<number, TerminalConnection>();
/** Everything wired to a session's emulator, unwired when the row goes. */
const unwires = new Map<number, () => void>();
const idleTimers = new Map<number, ReturnType<typeof setTimeout>>();
/**
 * The privileged debug pod a `kind: "node"` session is exec'd into — the
 * object `k8s.createNodeDebugPod` left on the cluster, and the store's own to
 * delete. Absent for a pod exec or a local shell, neither of which created
 * anything: deleting a pod the reader is merely looking at would be the wrong
 * kind of cleanup.
 *
 * Entries come out through {@link takeDebugPod}, the moment either cleanup path
 * acts on them — {@link close} when the far end goes, {@link endSession} when
 * the reader dismisses the row — and that take is what keeps the delete to
 * once: whichever runs second finds nothing left to clean up.
 */
const nodeDebugPods = new Map<number, { context: string; namespace: string; pod: string }>();

/** Ids are the store's own: a pod exec and a local PTY number themselves
 *  independently on the backend, so their ids collide. */
let seq = 0;

function emit() {
  for (const listener of listeners) listener();
}

/** Subscribe to store changes (for `useSyncExternalStore`). */
export function subscribeSessions(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/**
 * Every session this window knows about — running and closed.
 *
 * A stable reference until something in it changes: `useSyncExternalStore`
 * compares snapshots by identity and re-renders forever when handed a fresh
 * array each read.
 */
export function getSessions(): TerminalSessionRow[] {
  return sessions;
}

/**
 * This session's emulator — the live one, with its scrollback.
 *
 * **The caller must not dispose it.** It is the store's, it survives the
 * screen that shows it, and disposing it on unmount undoes the reason this
 * store exists.
 */
export function terminalFor(id: number): Terminal | undefined {
  return emulators.get(id);
}

/** Open a shell inside a pod (or, with `kind: "node"`, inside a node's debug
 *  pod) and track it. Never throws: a session that could not be opened is a
 *  `closed` row carrying the reason, because a failure the reader can read is
 *  worth more than one they have to catch. */
export async function startPodSession(req: PodSessionRequest): Promise<number> {
  const kind = req.kind ?? "pod";
  const id = register({
    kind,
    title: req.title ?? titleOf(req.pod, req.container),
    context: req.context,
    namespace: req.namespace,
  });
  // `req.pod` for a node session IS the debug pod: `k8s.createNodeDebugPod`
  // made it, this exec runs `nsenter` inside it, and nothing else knows its
  // name. Recorded before the connect, same as the row itself, so a session
  // the reader ends while it is still opening (see `connect`) still cleans up.
  if (kind === "node") {
    nodeDebugPods.set(id, { context: req.context, namespace: req.namespace, pod: req.pod });
  }
  await connect(id, (onData, onExit, size) =>
    startPodExec(
      req.context,
      req.namespace,
      req.pod,
      onData,
      onExit,
      req.container,
      req.command,
      size,
    ),
  );
  return id;
}

/** Open a shell on the user's own machine, scoped to a context, and track it.
 *  Never throws, for the same reason {@link startPodSession} does not. */
export async function startLocalSession(req: LocalSessionRequest): Promise<number> {
  const id = register({
    kind: "local",
    title: req.title ?? "Local shell",
    context: req.context,
    // A local shell is not in a namespace, and saying so with an empty string
    // is honest where naming the cluster's current one would not be.
    namespace: "",
  });
  await connect(id, (onData, onExit, size) =>
    startLocalTerminal(req.context, req.extraKubeconfigs ?? [], onData, () => onExit(null), size),
  );
  return id;
}

/**
 * The reader is done with this session: close the far end, drop the row, and
 * dispose the emulator.
 *
 * The one place a row leaves the screen. A session that ends on its own stays
 * listed as `closed` with its reason — #349's vanishing port-forward is what
 * happens otherwise, a reader carrying on as though a dead thing were fine.
 * Removing it is the reader's own act, and they do not need it reported back.
 */
export function endSession(id: number): void {
  disconnect(id);
  emulators.get(id)?.dispose();
  emulators.delete(id);
  const debugPod = takeDebugPod(id);
  if (debugPod) void deleteDebugPod(debugPod);
  commit(sessions.filter((s) => s.id !== id));
}

/**
 * Claim this session's debug pod, if it still has one to clean up.
 *
 * The whole point is the take: the entry leaves the map SYNCHRONOUSLY, before
 * any delete is issued, so of the two paths that clean up — {@link close} when
 * the far end goes, {@link endSession} when the reader dismisses the row — the
 * second one to run finds nothing and does nothing. Either order, and a second
 * call on the same id is a no-op rather than a delete aimed at a pod that is
 * already gone.
 *
 * `undefined` for a pod exec or a local shell, neither of which created
 * anything, and for a node session already cleaned up.
 */
function takeDebugPod(id: number): { context: string; namespace: string; pod: string } | undefined {
  const target = nodeDebugPods.get(id);
  nodeDebugPods.delete(id);
  return target;
}

/**
 * Delete the debug pod a finished node session leaves behind.
 *
 * Fire-and-forget from both callers: neither waits on it, so a failure has
 * nowhere to land on the row. Nor should it land there — a row that closed
 * says why the SHELL died, and overwriting that with a cleanup failure would
 * answer a question the reader never asked. It goes to `notify` instead,
 * described rather than raw: the pod may already be gone, or the reader may
 * lack permission to delete it, and either way `deletePod` itself never throws,
 * so a terminal that has already closed is never stranded waiting on this.
 */
async function deleteDebugPod(target: { context: string; namespace: string; pod: string }): Promise<void> {
  const out = await deletePod(target.context, target.namespace, target.pod);
  if (out.error) {
    notify.error(`Couldn't delete debug pod ${target.pod}`, describeError(out.error).detail);
  }
}

/** Reset the module-level store between tests. */
export function __resetSessionsForTests(): void {
  for (const id of [...emulators.keys()]) endSession(id);
  for (const id of [...handles.keys()]) disconnect(id);
  nodeDebugPods.clear();
  sessions = [];
  listeners.clear();
  seq = 0;
}

/** "pod · container", or just the pod when it has only the one. */
function titleOf(pod: string, container?: string): string {
  return container ? `${pod} · ${container}` : pod;
}

/**
 * Put the row and its emulator in place, before anything is connected.
 *
 * Deliberately ahead of the backend call: `startPodExec` resolves only after a
 * round trip, and output can arrive the moment it does. An emulator built
 * after the await would be built after the first prompt had nowhere to land.
 */
function register(row: Pick<TerminalSessionRow, "kind" | "title" | "context" | "namespace">): number {
  const id = ++seq;
  const now = Date.now();
  const stamp = wholeSecond(now);
  // No theme and no font here: those are the pane's, and come from tokens
  // where there is a DOM to read them from. `convertEol` matches the classic
  // pane — a backend that sends a bare newline still starts the next line at
  // column zero.
  emulators.set(id, new Terminal({ convertEol: true, scrollback: 10_000 }));
  sessions = [
    ...sessions,
    { id, ...row, state: "attached", startedAt: now, lastOutputAt: stamp },
  ];
  scheduleIdle(id);
  emit();
  return id;
}

/**
 * Open the far end and wire it to the emulator in both directions.
 *
 * The failure path leaves the row standing as `closed` rather than throwing:
 * a shell that RBAC refused is exactly the thing a reader needs to see said,
 * and `describeError` is what says it.
 */
async function connect(
  id: number,
  open: (
    onData: (chunk: string) => void,
    onExit: (error: string | null) => void,
    size: { cols: number; rows: number },
  ) => Promise<TerminalConnection>,
): Promise<void> {
  const term = emulators.get(id);
  if (!term) return;
  let handle: TerminalConnection;
  try {
    handle = await open(
      (chunk) => receive(id, chunk),
      (error) => close(id, error),
      // The PTY starts the size the emulator already is, so the first prompt
      // wraps where the reader will see it. The pane refits once it is on
      // screen, and `onResize` below carries that through.
      { cols: term.cols, rows: term.rows },
    );
  } catch (e) {
    // Raw, not described: `close` describes exactly once, and a sentence run
    // through `describeError` twice is classified on its own wording.
    close(id, e);
    return;
  }
  // A session the reader ended while the connect was still in flight has no
  // row left to attach to; close what just opened rather than leaking a PTY.
  if (!emulators.has(id)) {
    handle.close();
    return;
  }
  handles.set(id, handle);
  const data = term.onData((input) => handle.send(input));
  const resize = term.onResize(({ cols, rows }) => handle.resize(cols, rows));
  unwires.set(id, () => {
    data.dispose();
    resize.dispose();
  });
}

/** A chunk from the far end: into the emulator, and the session is awake. */
function receive(id: number, chunk: string) {
  emulators.get(id)?.write(chunk);
  markActive(id);
}

/**
 * Note that this session just spoke.
 *
 * Output arrives constantly, and the snapshot must survive it: a store that
 * rebuilt its array for every chunk would wake every `useSyncExternalStore`
 * subscriber on every byte. Only a session that was `idle`, or whose last
 * output was in an earlier second, changes anything here — and a session
 * whose far end is gone cannot be woken at all.
 */
function markActive(id: number) {
  const stamp = wholeSecond(Date.now());
  const next = sessions.map((s) => {
    if (s.id !== id || s.state === "closed") return s;
    if (s.state === "attached" && s.lastOutputAt === stamp) return s;
    return { ...s, state: "attached" as const, lastOutputAt: stamp };
  });
  scheduleIdle(id);
  commit(next);
}

/** An epoch stamp at second resolution. See {@link TerminalSessionRow.lastOutputAt}. */
function wholeSecond(millis: number): number {
  return Math.floor(millis / 1000) * 1000;
}

/** Start (or restart) this session's quiet clock. */
function scheduleIdle(id: number) {
  clearTimeout(idleTimers.get(id));
  idleTimers.set(
    id,
    setTimeout(() => markIdle(id), SESSION_IDLE_AFTER_MS),
  );
}

/** The session has been quiet long enough to read as idle. Still running. */
function markIdle(id: number) {
  idleTimers.delete(id);
  commit(sessions.map((s) => (s.id === id && s.state === "attached" ? { ...s, state: "idle" as const } : s)));
}

/**
 * The far end is gone. The row STAYS, marked `closed`, carrying why — and a
 * node session's debug pod does NOT.
 *
 * `reason` is whatever the backend gave — a clean exit gives nothing, and a
 * row that already recorded a reason keeps it rather than being overwritten by
 * a second, emptier notice of the same death. Nothing is rebuilt when nothing
 * changed: the store must not wake its subscribers for a closure it already
 * knows about.
 */
function close(id: number, reason: unknown) {
  disconnect(id);
  // The debug pod goes NOW, not when the reader gets round to dismissing the
  // row. A node shell that exits normally, or whose exec connection drops,
  // used to leave a privileged pod — host PID, network and IPC namespaces —
  // sleeping on the node indefinitely, and closing the app without dismissing
  // the row left it there for good. `takeDebugPod` is what keeps this to one
  // delete between here and `endSession`, either order.
  const debugPod = takeDebugPod(id);
  if (debugPod) void deleteDebugPod(debugPod);
  const error = describedReason(reason);
  commit(
    sessions.map((s) => {
      if (s.id !== id) return s;
      const kept = error ?? s.error;
      if (s.state === "closed" && s.error === kept) return s;
      return { ...s, state: "closed" as const, error: kept };
    }),
  );
}

/** Stop talking to the far end: close the session, unwire the emulator, and
 *  stop its clock. The emulator itself stays — its scrollback is the whole
 *  reason a closed row is still worth opening. Idempotent. */
function disconnect(id: number) {
  clearTimeout(idleTimers.get(id));
  idleTimers.delete(id);
  unwires.get(id)?.();
  unwires.delete(id);
  handles.get(id)?.close();
  handles.delete(id);
}

/** A reason worth showing, said the way a reader can use it — a backend
 *  string, or a thrown value from a session that never opened. `describeError`
 *  strips the wrappers either arrives in and classifies what is left; nothing
 *  at all, or an empty reason, is not a sentence and stays nothing. */
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
function commit(next: TerminalSessionRow[]) {
  if (next.length === sessions.length && !next.some((s, i) => s !== sessions[i])) return;
  sessions = next;
  emit();
}
