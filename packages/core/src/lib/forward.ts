import { invokeCommand, on } from "../transport/transport";
import { isTauri } from "../transport/platform";
import { describeError } from "./errors";
import { notify } from "./notify";

/** A live port-forward: a local port piped to a Pod or Service. */
export interface ActiveForward {
  id: number;
  context: string;
  namespace: string;
  /** "Pod" or "Service". */
  kind: string;
  name: string;
  remotePort: number;
  localPort: number;
  /**
   * Live state, driven by `forward:status:<id>` events from the backend and
   * by `forward:closed:<id>`.
   *
   * `failed` is the ENDED state: the tunnel exhausted its retries, or its
   * serve loop stopped. `reconnecting` is not — that one is still coming
   * back. See {@link isForwardEnded}, which is the only thing allowed to
   * decide which of the two a row is.
   */
  status: "active" | "reconnecting" | "failed";
  /**
   * Why the tunnel is in trouble, exactly as the backend said it — the raw
   * `error` off a `forward:status` event or the reason on `forward:closed`.
   * `undefined` while nothing has gone wrong, and cleared again when a
   * tunnel comes back, so a live row cannot carry a stale excuse.
   *
   * Raw on purpose: it is a backend string, and the surface that shows it
   * runs it through `describeError` rather than printing a Rust struct at
   * the reader.
   */
  error?: string;
  /** Bytes moved since this forward started, as the backend counts them. A
   *  running total, not a delta: `forward:traffic:<id>` carries the whole
   *  number each time. */
  bytesMoved: number;
  /** Epoch millis, stamped by the backend when the forward was created — for
   *  every forward, including ones this session started, so an age means the
   *  same thing in every row rather than "since I noticed it" in some. */
  startedAt: number;
}

export interface ForwardRequest {
  context: string;
  namespace: string;
  kind: string;
  name: string;
  remotePort: number;
  /** Preferred local port; omitted/0 lets the OS pick a free one. */
  localPort?: number;
}

/** One live forward as `list_forwards` reports it (Rust `ForwardEntry`). */
interface ForwardEntry {
  id: number;
  context: string;
  namespace: string;
  kind: string;
  name: string;
  remotePort: number;
  localPort: number;
  startedAt: number;
  bytes: number;
  /** True once the manager's own task has given up on this tunnel. */
  ended?: boolean;
  /** Why it gave up, when the loop had a reason to give. */
  error?: string | null;
}

// Module-level store so active forwards survive component remounts and are
// shared between the per-resource "Forward" action and the status-bar list.
let forwards: ActiveForward[] = [];
const listeners = new Set<() => void>();
const closers = new Map<number, () => void>();

/**
 * Ids the READER has taken off the screen — a tunnel they stopped, or a dead
 * row they dismissed. Both of those also tell the backend to forget the
 * forward, so this set has exactly one job left: a `list_forwards` that was
 * already in flight when the removal landed must not put the row back.
 *
 * It used to carry more, and it could not. A forward that gives up stays in
 * `ForwardManager`'s map until `stop` is called, and this store used to drop
 * such a row and remember the id so a rehydrate could not resurrect it — but
 * this is module-level JavaScript that a browser reload wipes, and the
 * `forward:closed` event that filled it fired before the reloaded page
 * existed. A reload therefore adopted the dead tunnel as `active`. That case
 * is answered where it can be: `list_forwards` now reports `ended` and the
 * reason with it, and {@link fromEntry} reads them.
 */
const dropped = new Set<number>();

function emit() {
  for (const l of listeners) l();
}

/** Subscribe to store changes (for `useSyncExternalStore`). */
export function subscribeForwards(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Current forwards, live and dead (stable reference until the next change). */
export function getForwards(): ActiveForward[] {
  return forwards;
}

/**
 * Has this tunnel given up?
 *
 * The one place the difference is decided, because more than one surface has
 * to agree about it: the forwards table counts its live rows, the status bar
 * counts port-forwards for the whole window, and a dead row's action is a
 * dismissal rather than a stop. A tunnel that is `reconnecting` is still
 * alive and still counts; only one that has stopped trying is dead.
 */
export function isForwardEnded(forward: Pick<ActiveForward, "status">): boolean {
  return forward.status === "failed";
}

/** Start a port-forward and track it. A backend loop that ends leaves the row
 *  behind, marked as ended — see {@link endForward}. */
export async function startPortForward(req: ForwardRequest): Promise<ActiveForward> {
  const info = await invokeCommand<{ id: number; localPort: number; startedAt: number }>(
    "start_port_forward",
    {
      context: req.context,
      namespace: req.namespace,
      kind: req.kind,
      name: req.name,
      remotePort: req.remotePort,
      localPort: req.localPort ?? null,
    },
  );
  // The start response carries its own startedAt now — the same stamp
  // `list_forwards` would report for this forward, taken once on the
  // backend rather than read back with a second call. No follow-up
  // `list_forwards` here: a tunnel that started fine no longer fails on a
  // read that has nothing to do with whether it's running.
  const fwd: ActiveForward = {
    id: info.id,
    context: req.context,
    namespace: req.namespace,
    kind: req.kind,
    name: req.name,
    remotePort: req.remotePort,
    localPort: info.localPort,
    status: "active",
    bytesMoved: 0,
    startedAt: info.startedAt,
  };
  forwards = [...forwards, fwd];
  watchForward(info.id);
  emit();
  return fwd;
}

/**
 * Stop a forward and drop it from the store.
 *
 * Also how a reader DISMISSES a dead row, and deliberately so. A forward that
 * gave up on its own stays in `ForwardManager`'s map — and in its listing —
 * until `stop` is called, so a dismissal that only deleted the row here would
 * be undone by the next reload. There is nothing left to abort in that case;
 * `stop` is what makes the backend forget the tunnel.
 */
export async function stopPortForward(id: number): Promise<void> {
  await invokeCommand("stop_port_forward", { id });
  removeForward(id);
}

/**
 * RECONCILE the store against the backend's own listing — adopt what it does
 * not know, and correct what it holds. This store is module-level and dies with
 * a browser reload; `ForwardManager` does not, so without this a web user
 * reloads into an empty table while their tunnels keep running.
 *
 * **Why correcting matters, and why this is the only place that can.** A row's
 * live state comes from `forward:status:<id>` and `forward:closed:<id>`, and
 * `watchForward(info.id)` can only be called AFTER `start_port_forward`
 * resolves — the id is server-assigned and the backend spawns the task before
 * returning — with `on()` registering asynchronously on top of that. In web
 * mode `wsClient` reconnects with a backoff up to 10s and events emitted during
 * the outage are never delivered at all. So a tunnel that exhausts its retries
 * inside that window emits `failed` and then `closed` into nothing, and its row
 * stays `status: "active"`, `error: undefined`, green, over a tunnel that
 * cannot carry a byte.
 *
 * This used to skip every id already in `known`, so {@link fromEntry} — the
 * only reader of the authoritative `ended`/`error`/`bytes` off `list_forwards`
 * — never ran for such a row, and NOTHING could ever correct it.
 *
 * **What is corrected, and what deliberately is not.** The listing says whether
 * the manager's task has GIVEN UP (`ended`) and how many bytes have moved. It
 * says nothing about the flap state: `ended: false` covers both `active` and
 * `reconnecting`. So only the ended direction is reconciled — pushing a live
 * entry back to `active` would wipe a genuine `reconnecting` off a row, trading
 * live information for a listing that cannot see it.
 *
 * The corrections go through {@link endForward} and {@link setForwardBytes},
 * which already no-op when nothing changed, so the row-identity property this
 * function has always protected survives: a rehydrate on mount that finds
 * everything as it left it leaves every row the same object and wakes no
 * subscriber. A forward the reader dropped on purpose stays dropped, corrections
 * included. Resolves even when the listing fails: that failure is reported to
 * the reader, not thrown at a mount effect.
 *
 * NOT called after `watchForward` in `startPortForward`, though the gap is
 * widest right there. That would put a `list_forwards` back on the start path,
 * which was removed on purpose so a tunnel that started fine can no longer fail
 * on a read that has nothing to do with whether it is running — a property with
 * its own test. The forwards screen rehydrates on mount, which is when a reader
 * is looking at these rows.
 */
export async function rehydrateForwards(): Promise<void> {
  let entries: ForwardEntry[];
  try {
    entries = await listForwards();
  } catch (e) {
    notify.error("Couldn't list active port forwards", describeError(e).detail);
    return;
  }
  const known = new Set(forwards.map((f) => f.id));
  const added: ActiveForward[] = [];
  for (const e of entries) {
    // A row the reader stopped or dismissed stays gone, and is not corrected
    // back onto the screen either.
    if (dropped.has(e.id)) continue;
    if (!known.has(e.id)) {
      added.push(fromEntry(e));
      continue;
    }
    setForwardBytes(e.id, e.bytes);
    if (e.ended === true) endForward(e.id, reasonOf(e.error));
  }
  if (added.length === 0) return;
  forwards = [...forwards, ...added];
  // Only the live ones: a tunnel the backend already reports as ended has no
  // further events to send, and its row is already saying so.
  for (const f of added) if (!isForwardEnded(f)) watchForward(f.id);
  emit();
}

/** Where a live port-forward is reachable from the current UI: the bound
 *  localhost port on desktop, or the same-origin `/pf/<id>/` reverse proxy on
 *  web (the container's loopback port isn't reachable from the browser). */
export function forwardUrl(info: { id: number; localPort: number }): string {
  return isTauri() ? `http://localhost:${info.localPort}` : `/pf/${info.id}/`;
}

/** The human-readable, copy-pasteable address of a live forward: the bound
 *  localhost port on desktop, or the absolute same-origin `/pf/<id>/` proxy URL
 *  on web (the container's loopback port isn't reachable from the browser). */
export function forwardAddress(info: { id: number; localPort: number }): string {
  return isTauri()
    ? `localhost:${info.localPort}`
    : `${window.location.origin}/pf/${info.id}/`;
}

async function listForwards(): Promise<ForwardEntry[]> {
  const res = await invokeCommand<{ forwards?: ForwardEntry[] }>("list_forwards");
  return res?.forwards ?? [];
}

/**
 * A backend entry as a store row.
 *
 * The listing says whether the manager's own task is still trying, so a
 * forward it reports as `ended` becomes a dead row here with the reason
 * attached — which is the whole of what a page that reloaded after the
 * `forward:closed` event has to go on. Anything else starts `active`; a
 * `forward:status` event corrects that the moment the tunnel flaps.
 */
function fromEntry(e: ForwardEntry): ActiveForward {
  return {
    id: e.id,
    context: e.context,
    namespace: e.namespace,
    kind: e.kind,
    name: e.name,
    remotePort: e.remotePort,
    localPort: e.localPort,
    status: e.ended === true ? "failed" : "active",
    error: reasonOf(e.error),
    bytesMoved: e.bytes,
    startedAt: e.startedAt,
  };
}

/** A reason worth keeping, or nothing. The backend says `null` for "no reason
 *  to give", and an empty string is not a sentence either. */
function reasonOf(error: unknown): string | undefined {
  return typeof error === "string" && error !== "" ? error : undefined;
}

/** Listen for one forward's closure, status and traffic. Idempotent, so a
 *  rehydrate that re-meets a known forward doesn't subscribe twice. */
function watchForward(id: number) {
  if (closers.has(id)) return;
  // The payload is the loop's final error, or null for a clean end.
  const unsubClosed = on(`forward:closed:${id}`, (payload) => endForward(id, reasonOf(payload)));
  const unsubStatus = on(`forward:status:${id}`, (payload) => {
    const event = payload as { state?: unknown; error?: unknown } | null;
    const state = event?.state;
    if (state === "active" || state === "reconnecting" || state === "failed") {
      setForwardStatus(id, state, reasonOf(event?.error));
    }
  });
  const unsubTraffic = on(`forward:traffic:${id}`, (payload) => {
    const bytes = (payload as { bytes?: unknown } | null)?.bytes;
    if (typeof bytes === "number" && Number.isFinite(bytes)) setForwardBytes(id, bytes);
  });
  closers.set(id, () => {
    unsubClosed();
    unsubStatus();
    unsubTraffic();
  });
}

/** Record a forward's live state and the reason that came with it. The reason
 *  is replaced rather than merged, so a tunnel that comes back does not keep
 *  the excuse from the attempt before. */
function setForwardStatus(id: number, status: ActiveForward["status"], error?: string) {
  const next = forwards.map((f) =>
    f.id === id && (f.status !== status || f.error !== error) ? { ...f, status, error } : f,
  );
  if (next.some((f, i) => f !== forwards[i])) {
    forwards = next;
    emit();
  }
}

/**
 * The backend has closed this tunnel. The row STAYS, marked as ended.
 *
 * A tunnel the reader stopped is gone from the screen at once, because they
 * did it and do not need it reported back. One that died underneath them is
 * news: this used to delete the row, so a forward that had been up for
 * fifteen minutes vanished with no trace and the reader went on depending on
 * a tunnel that was dead.
 *
 * `reason` is the closure's own; a reason already recorded by the `failed`
 * status that preceded it survives a closure that carries none.
 *
 * Nothing is rebuilt when nothing changed, and that guard is reached on the
 * ordinary path rather than a defensive one: the backend's give-up sequence
 * is a `failed` status carrying the reason and THEN a closure saying the same
 * thing, so the second event routinely tells this store nothing it does not
 * hold. Rebuilding the row for it would wake every subscriber to
 * `getForwards`, which is how `useSyncExternalStore` ends up looping.
 */
function endForward(id: number, reason: string | undefined) {
  // Nothing further will be reported about a tunnel that has stopped.
  closers.get(id)?.();
  closers.delete(id);
  const next = forwards.map((f) => {
    if (f.id !== id) return f;
    const error = reason ?? f.error;
    if (f.status === "failed" && f.error === error) return f;
    return { ...f, status: "failed" as const, error };
  });
  if (next.some((f, i) => f !== forwards[i])) {
    forwards = next;
    emit();
  }
}

/** Record a forward's running byte total. The event fires about once a second,
 *  so an unchanged total must leave both the row and the array identity alone —
 *  otherwise `useSyncExternalStore` wakes every subscriber every second. */
function setForwardBytes(id: number, bytesMoved: number) {
  const next = forwards.map((f) =>
    f.id === id && f.bytesMoved !== bytesMoved ? { ...f, bytesMoved } : f,
  );
  if (next.some((f, i) => f !== forwards[i])) {
    forwards = next;
    emit();
  }
}

function removeForward(id: number) {
  closers.get(id)?.();
  closers.delete(id);
  dropped.add(id);
  const next = forwards.filter((f) => f.id !== id);
  if (next.length !== forwards.length) {
    forwards = next;
    emit();
  }
}

/** Reset the module-level store between tests. */
export function __resetForwardStoreForTests(): void {
  for (const close of closers.values()) close();
  closers.clear();
  dropped.clear();
  forwards = [];
}
