/**
 * The main thread's side of a user's `/regex/` (#898): one worker per regex
 * clause, which answers every line the clause is tested on, so the screen only
 * reads answers and never runs the pattern itself.
 *
 * A pattern that backtracks catastrophically over one line freezes the thread
 * it runs on. A trial over the lines in the buffer can't vouch for the next
 * line, so the main thread never runs a user regex at all: a worker can be
 * thrown away, and the main thread cannot.
 */
import type { RegexAnswer, RegexItem } from "./regexMatch";

/** The part of a `Worker` the oracle uses, so a test can stand one in. */
export interface WorkerLike {
  postMessage(message: unknown): void;
  terminate(): void;
  onmessage: ((event: { data: unknown }) => void) | null;
  onerror: ((event: unknown) => void) | null;
}

/** `slow`: a batch ran past its budget. `unavailable`: no worker could answer at all. Both are final. */
export type OracleState = "ready" | "slow" | "unavailable";

export interface RegexOracle {
  /** Ask for the items not yet answered; resolves when they are, or when the oracle goes slow or unavailable. */
  ask(items: readonly RegexItem[]): Promise<void>;
  /** The answer for a tested string, or undefined while it is pending. */
  get(test: string): RegexAnswer | undefined;
  readonly state: OracleState;
  /** How many strings have an answer kept. */
  readonly size: number;
  /** Forgets the answers for every string but these: the lines still in the buffer. */
  prune(keep: ReadonlySet<string>): void;
  /** Ends the worker. */
  dispose(): void;
}

/** Half a second of the pattern running on one batch, from the worker's "started": past it a filter is a freeze. */
const BUDGET_MS = 500;

/**
 * How long a worker may take to say it has a batch. The budget does not run
 * until it does, so a slow start-up (a cold machine, a busy one) is not taken
 * for a slow pattern. A worker that has not started by now is as good as one
 * that cannot, and no regex is run without one.
 */
const STARTUP_MS = 5000;

/** Patterns that ran past the budget, by `${flags}/${source}`: slow for the rest of the session. */
const slowPatterns = new Set<string>();
const keyOf = (source: string, flags: string) => `${flags}/${source}`;

export function isKnownSlow(source: string, flags: string): boolean {
  return slowPatterns.has(keyOf(source, flags));
}

/** Forgets every slow pattern. For tests: the real memory lasts the session. */
export function clearSlowPatterns(): void {
  slowPatterns.clear();
}

const realWorker = (): WorkerLike =>
  new Worker(new URL("./regexWorker.worker.ts", import.meta.url), { type: "module" }) as unknown as WorkerLike;

interface Batch {
  readonly id: number;
  readonly items: readonly RegexItem[];
  readonly done: () => void;
}

type Reply = { readonly id: number; readonly started: true } | { readonly id: number; readonly answers: RegexAnswer[] };

export function createRegexOracle(
  source: string,
  flags: string,
  { worker = realWorker }: { worker?: () => WorkerLike } = {},
): RegexOracle {
  const answers = new Map<string, RegexAnswer>();
  /** Strings sent or waiting to be, so one asked twice goes to the worker once. */
  const pending = new Set<string>();
  const queue: Batch[] = [];
  let state: OracleState = isKnownSlow(source, flags) ? "slow" : "ready";
  let disposed = false;
  let w: WorkerLike | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let ids = 0;
  /** Resolves once the last batch queued so far is answered. */
  let last = Promise.resolve();

  const live = () => state === "ready" && !disposed;

  /** Ends the worker, and lets everything waiting on it go: nothing more will be answered. */
  function halt() {
    clearTimeout(timer);
    w?.terminate();
    w = undefined;
    pending.clear();
    for (const batch of queue.splice(0)) batch.done();
  }

  function fail(to: "slow" | "unavailable") {
    if (!live()) return;
    state = to;
    if (to === "slow") slowPatterns.add(keyOf(source, flags));
    halt();
  }

  function receive(reply: Reply) {
    const batch = queue[0];
    if (!live() || batch === undefined || reply.id !== batch.id) return;
    clearTimeout(timer);
    if ("started" in reply) {
      // From here it is the pattern being timed, not the worker coming up.
      timer = setTimeout(() => fail("slow"), BUDGET_MS);
      return;
    }
    batch.items.forEach((item, i) => {
      answers.set(item.test, reply.answers[i]);
      pending.delete(item.test);
    });
    queue.shift();
    batch.done();
    send();
  }

  function send() {
    const batch = queue[0];
    if (batch === undefined) return;
    try {
      if (w === undefined) {
        w = worker();
        w.onmessage = ({ data }) => receive(data as Reply);
        w.onerror = () => fail("unavailable");
      }
      w.postMessage({ id: batch.id, source, flags, items: batch.items });
    } catch {
      return fail("unavailable");
    }
    timer = setTimeout(() => fail("unavailable"), STARTUP_MS);
  }

  return {
    ask(items) {
      if (!live()) return Promise.resolve();
      const fresh: RegexItem[] = [];
      for (const item of items) {
        if (answers.has(item.test) || pending.has(item.test)) continue;
        pending.add(item.test);
        fresh.push(item);
      }
      if (fresh.length > 0) {
        last = new Promise((done) => queue.push({ id: (ids += 1), items: fresh, done }));
        if (queue.length === 1) send();
      }
      return last;
    },
    get: (test) => answers.get(test),
    get state() {
      return state;
    },
    get size() {
      return answers.size;
    },
    prune(keep) {
      for (const test of answers.keys()) if (!keep.has(test)) answers.delete(test);
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      halt();
    },
  };
}
