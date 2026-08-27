import { useSyncExternalStore } from "react";
import { connectCluster, describeError, type ClusterContext, type ClusterInfo } from "@srelens/core";
import { setLink } from "./workspace";

let infos: Record<string, ClusterInfo> = {};

/**
 * A cluster's reachability probe, for readouts (the connections screen) that
 * want a latency and a tri-state status rather than the raw `ClusterInfo` the
 * rest of the shell keys off of. Kept as its own store rather than folded into
 * `infos`, so `getInfo`/`useInfo`/`useInfos` and their callers — the rail, the
 * status strip, Overview, Toolbox — stay untouched.
 */
export type ProbeState = "unread" | "reachable" | "unreachable";

export interface Probe {
  state: ProbeState;
  /**
   * Round trip to the API server, timed around the `connect` call — no
   * capability returns this, so it is measured here. It is a network
   * duration, not a cluster health signal. Absent unless `state` is
   * "reachable": a call that did not complete has no duration to report, and
   * `0` would read as instant, which is the opposite of the truth.
   */
  latencyMs?: number;
  /** Through `describeError`, never the backend's raw string. Present only
   *  when `state` is "unreachable" and the backend said something — an
   *  unreachable cluster that said nothing (the link state's "disconnected")
   *  gets no invented message either. */
  error?: string;
  version?: string;
}

const UNREAD: Probe = { state: "unread" };

let probes: Record<string, Probe> = {};

function deriveProbe(info: ClusterInfo, latencyMs: number): Probe {
  if (info.reachable) {
    return { state: "reachable", latencyMs, ...(info.version ? { version: info.version } : {}) };
  }
  return { state: "unreachable", ...(info.error ? { error: describeError(info.error).detail } : {}) };
}

const listeners = new Set<() => void>();
const emit = () => { for (const l of listeners) l(); };

export function getInfo(stableId: string): ClusterInfo | undefined { return infos[stableId]; }
export function getProbe(stableId: string): Probe { return probes[stableId] ?? UNREAD; }
export function resetProbes(): void { infos = {}; probes = {}; reading.clear(); emit(); }
function subscribe(l: () => void) { listeners.add(l); return () => listeners.delete(l); }
export function useProbe(stableId: string | null): Probe {
  return useSyncExternalStore(subscribe, () => (stableId ? (probes[stableId] ?? UNREAD) : UNREAD), () => UNREAD);
}

/** Every cluster's probe at once, for the connections screen's list — same
 *  shape and same reasoning as {@link useInfos}. */
export function useProbes(): Record<string, Probe> {
  return useSyncExternalStore(subscribe, () => probes, () => probes);
}
export function useInfo(stableId: string | null): ClusterInfo | undefined {
  return useSyncExternalStore(subscribe, () => (stableId ? infos[stableId] : undefined), () => undefined);
}

/**
 * Every cluster's info at once, for a caller with a list rather than an id.
 *
 * `useInfo` cannot serve that: a hook per cluster is a hook count that changes
 * with the list, and asking it about `null` to get the subscription alone is a
 * subscription that never fires, because its snapshot is `undefined` whatever
 * happens and `useSyncExternalStore` bails on an unchanged one (#325 review).
 *
 * The whole record is a sound snapshot because `infos` is replaced rather than
 * mutated on every write, so its identity changes exactly when its contents do.
 */
export function useInfos(): Record<string, ClusterInfo> {
  return useSyncExternalStore(subscribe, () => infos, () => infos);
}

/**
 * The read in flight per cluster — **the one place two callers can both see.**
 *
 * Two surfaces ask for a reading of the same cluster and neither can see the
 * other: `shell/Window.tsx` probes the workspace's clusters whenever its
 * contexts change, skipping a cluster that already has an answer
 * (`if (getInfo(id)) continue`), and the connections screen probes every
 * context it lists and re-reads them all on `Refresh all`. `getInfo` is
 * populated only AFTER the round trip completes, so for the whole length of a
 * slow or timing-out cluster both callers see "no answer yet" and both start a
 * read.
 *
 * That is two unordered writes of one cluster's reading, and this store is
 * exactly where they cannot be undone: whichever finishes last wins, so a
 * 30-second timeout from the first read lands on top of the second read's
 * `12 ms` and the row says `unreachable` about a cluster that answered.
 *
 * A guard held by either caller could not fix it — a component ref is invisible
 * to the other, and both call THIS function. So the rule lives here: **one
 * cluster, one read in flight**, and a second caller joins the first rather
 * than racing it. Joining also means the second caller's `await` resolves when
 * the reading is actually in — which is what lets a caller do more work once a
 * cluster has answered (the connections screen fetches its facts) rather than
 * walking away from a cluster it declined to re-probe.
 *
 * Keyed on `stableId`, and deliberately not on the injected `connect`: two
 * concurrent reads of one cluster with different transports is not a thing any
 * caller does, and the tests that inject one await it before the next.
 * {@link resetProbes} clears this along with the answers, so a test that leaves
 * a read hanging does not leave the next one joined to it.
 */
const reading = new Map<string, Promise<void>>();

/**
 * Read one cluster: connect to it, time the round trip, and record what came
 * back — or join the read already out for it (see {@link reading}).
 *
 * Link state is derived, not invented: `connecting` while the call is out,
 * then `connected` from `reachable`, `error` with the backend's message, or
 * `disconnected` when it is unreachable and says nothing more.
 *
 * `now` times the round trip for {@link Probe.latencyMs} and defaults to
 * `Date.now`; tests inject a fake clock that actually advances, because one
 * that never moves would let a broken timing calculation (e.g. reading the
 * clock once and subtracting it from itself) pass unnoticed. Both it and
 * `connect` belong to whichever call STARTED the read — a joining caller gets
 * the reading the first one is taking, which is the point of joining.
 *
 * `connect` is not expected to reject — `connectCluster` already catches
 * transport failures into `{ reachable: false, error }` — but an injected
 * one in a test might, so a rejection is folded into the same unreachable
 * shape rather than left as an unhandled promise. Not `async` itself, so a
 * joining caller is handed the running promise rather than a fresh one wrapped
 * around it.
 */
export function probeCluster(
  ctx: ClusterContext,
  connect: typeof connectCluster = connectCluster,
  now: () => number = Date.now,
): Promise<void> {
  const running = reading.get(ctx.stableId);
  if (running) return running;
  const run = read(ctx, connect, now).finally(() => {
    reading.delete(ctx.stableId);
  });
  reading.set(ctx.stableId, run);
  return run;
}

/** One reading, start to finish. See {@link probeCluster} for who may start it. */
async function read(
  ctx: ClusterContext,
  connect: typeof connectCluster,
  now: () => number,
): Promise<void> {
  setLink(ctx.stableId, "connecting");
  const started = now();
  let info: ClusterInfo;
  try {
    info = await connect(ctx.name);
  } catch (e) {
    info = { context: ctx.name, reachable: false, error: String(e) };
  }
  const elapsedMs = now() - started;
  infos = { ...infos, [ctx.stableId]: info };
  probes = { ...probes, [ctx.stableId]: deriveProbe(info, elapsedMs) };
  emit();
  if (info.reachable) setLink(ctx.stableId, "connected");
  else if (info.error) setLink(ctx.stableId, "error", info.error);
  else setLink(ctx.stableId, "disconnected");
}
