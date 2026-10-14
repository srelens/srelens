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

/** Half a second of the pattern running, from the worker's "started": past it a filter is a freeze, not a wait. */
const BUDGET_MS = 500;

/**
 * How long a worker may take to say "started". The budget does not run until it does, so a slow start-up (a cold
 * machine, a busy one) is not taken for a slow pattern, and a "slow" is not remembered for the session on that account.
 * A worker that has not started by now is as good as one that cannot: the query is let through.
 */
const STARTUP_MS = 5000;

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
 * Whether the pattern finishes over `texts` within the budget, counted from the
 * worker's "started" (it posts that when it has the job, before it runs the
 * pattern, and "done" after). The worker is ended however the trial goes. A
 * worker that cannot start, that does not start in time, or that fails, is "ok":
 * the guard must not hold a query back in a host with no working workers.
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
    let budget: ReturnType<typeof setTimeout> | undefined;
    const settle = (verdict: Verdict) => {
      // Once: an answer that lands after the budget must not turn "slow" into "ok" in the memory.
      if (settled) return;
      settled = true;
      clearTimeout(startup);
      clearTimeout(budget);
      w?.terminate();
      verdicts.set(keyOf(source, flags), verdict);
      resolve(verdict);
    };
    const startup = setTimeout(() => settle("ok"), STARTUP_MS);
    try {
      w = worker();
      w.onmessage = ({ data }) => {
        if (settled) return;
        if (data !== "started") return settle("ok");
        // From here it is the pattern being timed, not the worker coming up.
        clearTimeout(startup);
        budget ??= setTimeout(() => settle("slow"), budgetMs);
      };
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
