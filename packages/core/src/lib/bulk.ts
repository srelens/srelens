/**
 * Run one operation over many items, collecting a per-item outcome. A failing
 * item never aborts the rest (partial failures are expected and reported), and
 * concurrency is bounded so a 50-item bulk delete doesn't hammer the apiserver.
 */

/** The outcome of a bulk operation on a single item. */
export interface BulkOutcome<T> {
  item: T;
  /** `cancelled` is an item the run never sent, not one the cluster refused. */
  status: "ok" | "error" | "cancelled";
  error?: string;
}

/** A per-item result from any of the action wrappers (`{ ok }` / `{ deleted }`
 *  / `{ error }`). Success is the absence of an `error`. */
export type ActionOutcome = { ok?: boolean; deleted?: boolean; error?: string };

/** Where one item has got to, for a progress list a reader is watching. */
export type BulkState = "pending" | "running" | "ok" | "error" | "cancelled";

/** One line of a live progress list. */
export interface BulkProgress<T> {
  item: T;
  state: BulkState;
  error?: string;
}

export interface BulkRunOptions<T> {
  /**
   * Stops the items that have NOT STARTED.
   *
   * It is never passed to `op` and never interrupts a call already in flight:
   * a write the host has sent is a write the cluster may already have
   * accepted, and cancelling the client's wait for it would leave the reader
   * told nothing happened while the controller acts. An item already running
   * therefore finishes and reports `ok` or `error` like any other; only the
   * queue behind it is dropped, as `cancelled`.
   */
  signal?: AbortSignal;
  /**
   * Called with EVERY item's state, in the items' own order, each time one
   * moves — starting with all of them `pending`, before anything is sent, so
   * a reader sees the queue rather than an empty panel that fills in later.
   *
   * The array and its entries are fresh on each call, so a React caller can
   * store them without copying and without the list mutating underneath it.
   */
  onProgress?: (items: readonly BulkProgress<T>[]) => void;
}

/**
 * Apply `op` to every item with at most `concurrency` in flight, returning an
 * outcome per item in the original order. Never throws and never short-circuits:
 * each item's failure is captured, the others continue.
 */
export async function runBulk<T>(
  items: readonly T[],
  op: (item: T) => Promise<ActionOutcome>,
  concurrency = 8,
  options: BulkRunOptions<T> = {},
): Promise<BulkOutcome<T>[]> {
  const { signal, onProgress } = options;
  const results: BulkOutcome<T>[] = new Array(items.length);
  const states: BulkState[] = items.map(() => "pending");
  const errors: Array<string | undefined> = items.map(() => undefined);
  let next = 0;

  const report = () =>
    onProgress?.(items.map((item, i) => ({ item, state: states[i], ...(errors[i] === undefined ? {} : { error: errors[i] }) })));
  report();

  const settle = (i: number, outcome: BulkOutcome<T>) => {
    results[i] = outcome;
    states[i] = outcome.status;
    errors[i] = outcome.error;
    report();
  };

  const worker = async () => {
    for (let i = next++; i < items.length; i = next++) {
      const item = items[i];
      if (signal?.aborted) {
        settle(i, { item, status: "cancelled" });
        continue;
      }
      states[i] = "running";
      report();
      try {
        // One argument, deliberately: `op` is handed no signal, so nothing it
        // calls can abort a request the cluster may already have accepted.
        const r = await op(item);
        settle(i, r.error ? { item, status: "error", error: r.error } : { item, status: "ok" });
      } catch (e) {
        settle(i, { item, status: "error", error: String(e) });
      }
    }
  };

  const workers = Math.max(1, Math.min(concurrency, items.length));
  await Promise.all(Array.from({ length: workers }, worker));
  return results;
}

/**
 * Counts for a completed bulk run.
 *
 * `cancelled` is its own count rather than folded into `failed`: an item the
 * run never sent is not an item the cluster refused, and a caller that reports
 * it as a failure is making a claim about the cluster that nothing backs.
 */
export function summarize<T>(outcomes: BulkOutcome<T>[]): { ok: number; failed: number; cancelled: number } {
  let ok = 0;
  let failed = 0;
  let cancelled = 0;
  for (const o of outcomes) {
    if (o.status === "ok") ok++;
    else if (o.status === "cancelled") cancelled++;
    else failed++;
  }
  return { ok, failed, cancelled };
}
