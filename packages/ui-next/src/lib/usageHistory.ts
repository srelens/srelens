/**
 * The last few minutes of each pod's CPU and memory, kept so a list can draw
 * where a figure has been and not only where it is (#864).
 *
 * metrics-server answers with one reading and no past. The past here is the
 * readings this window has itself been given, kept as they arrive: it starts
 * when a pods list is first opened for a cluster and fills from there. A graph
 * that has just started is one point, drawn as a flat line, and that is the
 * truth — nothing is invented to fill the ten minutes before the list opened.
 *
 * Module-level, like the session and forward stores: it outlives the screen,
 * so leaving the Pods list and coming back does not start the graph again.
 */

/** How much past is kept and drawn. */
export const USAGE_WINDOW_MS = 10 * 60 * 1000;

/**
 * Readings closer together than this are one reading. Two tabs on the same
 * list poll on their own clocks, and a point per poll per tab would draw the
 * same ten minutes twice as densely for no more information.
 */
const MIN_SPACING_MS = 5_000;

export interface UsageSample {
  /** When the reading was taken, in epoch milliseconds. */
  at: number;
  cpu: number;
  memory: number;
}

/** Cluster, then the pod's own key within it. */
const history = new Map<string, Map<string, UsageSample[]>>();

/**
 * `samples` with everything outside the window dropped: older than ten
 * minutes, and — after a clock has been put back — stamped later than now. A
 * reading "from the future" would otherwise sit at the end of the line as the
 * current one until the clock caught up with it.
 */
function inWindow(samples: UsageSample[], now: number): UsageSample[] {
  const from = now - USAGE_WINDOW_MS;
  if (samples.every((s) => s.at >= from && s.at <= now)) return samples;
  return samples.filter((s) => s.at >= from && s.at <= now);
}

/**
 * Keep a batch of readings taken together — one poll of one scope.
 *
 * Returns each pod's history including the new reading, oldest first, as
 * fresh arrays: a row that is handed one re-renders, which is what a new
 * point is for.
 *
 * A pod absent from the batch is NOT forgotten here: a batch is one namespace
 * of several, and the pods of the others are simply not in it. A pod's past
 * goes when its last reading has aged out of the window, which a deleted pod's
 * does within ten minutes, swept as later batches arrive.
 *
 * Nor is a pod told apart here from one of the same name created after it: a
 * reading carries a name and no identity. The row knows when its pod was
 * created, and draws only what is newer than that — see `PodUsageCell`.
 */
export function recordUsage(
  context: string,
  readings: ReadonlyArray<{ key: string; cpu: number; memory: number }>,
  now: number = Date.now(),
): Map<string, UsageSample[]> {
  let pods = history.get(context);
  if (!pods) history.set(context, (pods = new Map()));
  const out = new Map<string, UsageSample[]>();
  for (const { key, cpu, memory } of readings) {
    const kept = inWindow(pods.get(key) ?? [], now);
    const last = kept[kept.length - 1];
    // `kept` holds nothing later than `now`, so the new reading is always the
    // newest and the points stay in order.
    const next: UsageSample[] =
      last && now - last.at < MIN_SPACING_MS
        ? [...kept.slice(0, -1), { at: now, cpu, memory }]
        : [...kept, { at: now, cpu, memory }];
    pods.set(key, next);
    out.set(key, next);
  }
  // The sweep: whatever has had no reading inside the window is a pod that is
  // gone, or a scope nobody is looking at any more. Every cluster, not only
  // the one just read — a cluster the reader has left is never read again,
  // and would otherwise keep its last ten minutes for as long as the window
  // stays open.
  for (const [cluster, held] of history) {
    for (const [key, samples] of held) {
      if (inWindow(samples, now).length === 0) held.delete(key);
    }
    if (held.size === 0) history.delete(cluster);
  }
  return out;
}

/** What is held for one pod, oldest first; empty when nothing is. */
export function usageHistory(context: string, key: string, now: number = Date.now()): UsageSample[] {
  return inWindow(history.get(context)?.get(key) ?? [], now);
}

/** How much is held, so a test can see that what should be forgotten is. */
export function __usageHistorySizeForTests(): { clusters: number; pods: number } {
  let pods = 0;
  for (const held of history.values()) pods += held.size;
  return { clusters: history.size, pods };
}

/** Forget everything, so a test starts with no past. */
export function __resetUsageHistoryForTests(): void {
  history.clear();
}
