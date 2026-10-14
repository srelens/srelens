/**
 * Trial-runs a user's `/regex/` over the lines it would be tested on, in a
 * worker, before the Logs screen applies it (#898).
 *
 * A regex runs on the main thread (`matchLine`, the facet counts, the
 * highlighting), and a pattern that backtracks catastrophically over a matching
 * line freezes the whole app. A worker can be thrown away; the main thread
 * cannot. So a pattern is first run over the buffer in a worker, and one that
 * has not finished within the budget is never given to the main thread.
 */

/** The part of a `Worker` the trial uses, so a test can stand one in. */
export interface WorkerLike {
  postMessage(message: unknown): void;
  terminate(): void;
  onmessage: ((event: { data: unknown }) => void) | null;
  onerror: ((event: unknown) => void) | null;
}

export type Verdict = "ok" | "slow";

/** Half a second, worker start-up included: past it a filter is a freeze, not a wait. */
const BUDGET_MS = 500;

/** What the pattern was told, by `${flags}/${source}`. A verdict holds for the session. */
const verdicts = new Map<string, Verdict>();
const keyOf = (source: string, flags: string) => `${flags}/${source}`;

const realWorker = (): WorkerLike =>
  new Worker(new URL("./regexTrial.worker.ts", import.meta.url), { type: "module" }) as unknown as WorkerLike;

/** What the worker does with a trial: the same `test` calls the main thread would make. */
export function runTrial(source: string, flags: string, texts: readonly string[]): void {
  const re = new RegExp(source, flags);
  for (const text of texts) re.test(text);
}

/**
 * Whether the pattern finishes over `texts` within the budget. The worker is
 * ended however the trial goes. A worker that cannot start, or that fails, is
 * "ok": the guard must not hold a query back in a host with no workers.
 */
export function trialRegex(
  source: string,
  flags: string,
  texts: readonly string[],
  { budgetMs = BUDGET_MS, worker = realWorker }: { budgetMs?: number; worker?: () => WorkerLike } = {},
): Promise<Verdict> {
  const known = verdicts.get(keyOf(source, flags));
  if (known !== undefined) return Promise.resolve(known);
  return new Promise((resolve) => {
    let w: WorkerLike | undefined;
    let settled = false;
    const settle = (verdict: Verdict) => {
      // Once: an answer that lands after the budget must not turn "slow" into "ok" in the memory.
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      w?.terminate();
      verdicts.set(keyOf(source, flags), verdict);
      resolve(verdict);
    };
    const timer = setTimeout(() => settle("slow"), budgetMs);
    try {
      w = worker();
      w.onmessage = () => settle("ok");
      w.onerror = () => settle("ok");
      w.postMessage({ source, flags, texts });
    } catch {
      settle("ok");
    }
  });
}

/** The verdict already reached for this pattern, if any. */
export function cachedVerdict(source: string, flags: string): Verdict | undefined {
  return verdicts.get(keyOf(source, flags));
}

/** Forgets every verdict. For tests: the real cache lasts the session. */
export function clearTrialCache(): void {
  verdicts.clear();
}
