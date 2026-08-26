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
export function resetProbes(): void { infos = {}; probes = {}; emit(); }
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
 * Link state is derived, not invented: `connecting` while the call is out,
 * then `connected` from `reachable`, `error` with the backend's message, or
 * `disconnected` when it is unreachable and says nothing more.
 *
 * `now` times the round trip for {@link Probe.latencyMs} and defaults to
 * `Date.now`; tests inject a fake clock that actually advances, because one
 * that never moves would let a broken timing calculation (e.g. reading the
 * clock once and subtracting it from itself) pass unnoticed.
 *
 * `connect` is not expected to reject — `connectCluster` already catches
 * transport failures into `{ reachable: false, error }` — but an injected
 * one in a test might, so a rejection is folded into the same unreachable
 * shape rather than left as an unhandled promise.
 */
export async function probeCluster(
  ctx: ClusterContext,
  connect: typeof connectCluster = connectCluster,
  now: () => number = Date.now,
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
