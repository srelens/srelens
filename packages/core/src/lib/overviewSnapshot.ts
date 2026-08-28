// Typed wrappers for the overview-snapshot commands (backend:
// `overview_snapshot.rs`, issue #148) — disk persistence for the cluster
// overview so a cold start paints the last known counts instantly.
//
// The backend treats `stats` as opaque JSON: this module owns the shape.
// Every wrapper degrades to a no-op on failure or off desktop — the commands
// only exist under Tauri, so in web mode they short-circuit client-side
// instead of sending authenticated HTTP requests the server would 404 — and
// a broken cache must never break the overview itself.
import { isTauri } from "../transport/platform";
import { invokeCommand } from "../transport/transport";

/** Cluster overview counts, as shown on the dashboard tiles. */
export interface OverviewStats {
  nodes: { total: number; ready: number };
  pods: { total: number; running: number; pending: number; other: number };
  deployments: number;
  services: number;
  namespaces: number;
  events: { total: number; normal: number; warnings: number; recentWarnings: string[] };
}

/** A point-in-time overview, persisted so a cold start can render instantly. */
export interface OverviewSnapshot {
  stats: OverviewStats;
  updatedAt: number;
}

/** A command invoker — injectable for testing. */
type Invoker = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

function isNumbers(value: unknown, keys: string[]): boolean {
  if (!value || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  return keys.every((key) => typeof record[key] === "number");
}

/** True only for a complete `OverviewStats` — the overview dereferences every
 * nested field straight in render, so a snapshot from an older schema (or a
 * half-corrupted file) must read as a cache miss, not crash the dashboard. */
function isOverviewStats(value: unknown): value is OverviewStats {
  if (!value || typeof value !== "object") return false;
  const stats = value as Record<string, unknown>;
  const events = stats.events as Record<string, unknown> | null | undefined;
  return (
    isNumbers(stats.nodes, ["total", "ready"]) &&
    isNumbers(stats.pods, ["total", "running", "pending", "other"]) &&
    isNumbers(stats, ["deployments", "services", "namespaces"]) &&
    isNumbers(events, ["total", "normal", "warnings"]) &&
    Array.isArray(events?.recentWarnings) &&
    events.recentWarnings.every((warning) => typeof warning === "string")
  );
}

/** Last persisted snapshot for a context, or null if absent or unavailable. */
export async function loadPersistedOverview(
  context: string,
  invoke: Invoker = invokeCommand,
): Promise<OverviewSnapshot | null> {
  if (!isTauri()) return null;
  try {
    const out = await invoke<OverviewSnapshot | null>("overview_snapshot_load", { context });
    if (!out || typeof out !== "object") return null;
    if (!isOverviewStats(out.stats)) return null;
    if (typeof out.updatedAt !== "number") return null;
    // The backend's i64 outranges JS dates; formatUpdatedAt would throw on an
    // unrepresentable timestamp, so it reads as a cache miss instead.
    if (Number.isNaN(new Date(out.updatedAt).getTime())) return null;
    return out;
  } catch {
    return null;
  }
}

/** Persist a context's snapshot for the next cold start (best-effort). */
export async function persistOverview(
  context: string,
  snapshot: OverviewSnapshot,
  invoke: Invoker = invokeCommand,
): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("overview_snapshot_save", { context, snapshot });
  } catch {
    // storage failure — the overview works without the cache
  }
}

/** Drop the persisted snapshot for one context, or all of them (best-effort). */
export async function clearPersistedOverview(
  context?: string,
  invoke: Invoker = invokeCommand,
): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("overview_snapshot_clear", { context: context ?? null });
  } catch {
    // storage failure — nothing to clear
  }
}
