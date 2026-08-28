/**
 * Run one operation over many items, collecting a per-item outcome. A failing
 * item never aborts the rest (partial failures are expected and reported), and
 * concurrency is bounded so a 50-item bulk delete doesn't hammer the apiserver.
 */

/** The outcome of a bulk operation on a single item. */
export interface BulkOutcome<T> {
  item: T;
  status: "ok" | "error";
  error?: string;
}

/** A per-item result from any of the action wrappers (`{ ok }` / `{ deleted }`
 *  / `{ error }`). Success is the absence of an `error`. */
export type ActionOutcome = { ok?: boolean; deleted?: boolean; error?: string };

/**
 * Apply `op` to every item with at most `concurrency` in flight, returning an
 * outcome per item in the original order. Never throws and never short-circuits:
 * each item's failure is captured, the others continue.
 */
export async function runBulk<T>(
  items: readonly T[],
  op: (item: T) => Promise<ActionOutcome>,
  concurrency = 8,
): Promise<BulkOutcome<T>[]> {
  const results: BulkOutcome<T>[] = new Array(items.length);
  let next = 0;

  const worker = async () => {
    for (let i = next++; i < items.length; i = next++) {
      const item = items[i];
      try {
        const r = await op(item);
        results[i] = r.error ? { item, status: "error", error: r.error } : { item, status: "ok" };
      } catch (e) {
        results[i] = { item, status: "error", error: String(e) };
      }
    }
  };

  const workers = Math.max(1, Math.min(concurrency, items.length));
  await Promise.all(Array.from({ length: workers }, worker));
  return results;
}

/** Summary counts for a completed bulk run. */
export function summarize<T>(outcomes: BulkOutcome<T>[]): { ok: number; failed: number } {
  let ok = 0;
  let failed = 0;
  for (const o of outcomes) {
    if (o.status === "ok") ok++;
    else failed++;
  }
  return { ok, failed };
}
