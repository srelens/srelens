/**
 * A stand-in for `regexWorker`, for tests: jsdom has no `Worker`. Mock the
 * module with this one (`vi.mock(".../regexWorker", () => import(".../regexWorker.fake"))`)
 * and every oracle the code makes lands in `oracles`, keeping each `ask`
 * exactly as it was made. It answers only when the test says so, with the
 * worker's own code, so a test sees the screen wait on it.
 */
import { answer, type RegexAnswer, type RegexItem } from "./regexMatch";
import type { OracleState, RegexOracle } from "./regexWorker";

export const oracles: FakeOracle[] = [];
/** Patterns found slow, by `${flags}/${source}`, as the real module's session memory holds them. */
export const knownSlow = new Set<string>();
let answering = false;

export function resetFakeOracles(): void {
  oracles.length = 0;
  knownSlow.clear();
}

export function isKnownSlow(source: string, flags: string): boolean {
  return knownSlow.has(`${flags}/${source}`);
}

export function createRegexOracle(source: string, flags: string): RegexOracle {
  const oracle = new FakeOracle(source, flags);
  oracles.push(oracle);
  return oracle;
}

/** Whether a fake worker is running a pattern right now: what a spy on the main thread's regexes leaves out. */
export function workerIsRunning(): boolean {
  return answering;
}

/** The one oracle for `source` that is not disposed. */
export function oracleFor(source: string): FakeOracle {
  const found = oracles.filter((o) => o.source === source && !o.disposed);
  if (found.length !== 1) throw new Error(`${found.length} live oracles for ${source}`);
  return found[0];
}

export class FakeOracle implements RegexOracle {
  readonly asks: RegexItem[][] = [];
  readonly answers = new Map<string, RegexAnswer>();
  state: OracleState;
  disposed = false;
  private waiting: (() => void)[] = [];

  constructor(
    readonly source: string,
    readonly flags: string,
  ) {
    this.state = isKnownSlow(source, flags) ? "slow" : "ready";
  }

  ask(items: readonly RegexItem[]): Promise<void> {
    if (this.state !== "ready" || this.disposed) return Promise.resolve();
    this.asks.push([...items]);
    return new Promise((resolve) => this.waiting.push(resolve));
  }

  get(test: string): RegexAnswer | undefined {
    return this.answers.get(test);
  }

  get size(): number {
    return this.answers.size;
  }

  prune(keep: ReadonlySet<string>): void {
    for (const test of this.answers.keys()) if (!keep.has(test)) this.answers.delete(test);
  }

  dispose(): void {
    this.disposed = true;
    this.release();
  }

  /** The worker answers every item asked so far: with the worker's code, or with `given` for every item. */
  reply(given?: RegexAnswer): void {
    answering = true;
    try {
      for (const items of this.asks) {
        const answers = given === undefined ? answer(this.source, this.flags, items) : items.map(() => given);
        answers.forEach((a, i) => this.answers.set(items[i].test, a));
      }
    } finally {
      answering = false;
    }
    this.release();
  }

  /** A batch ran past its budget, or no worker could run: final, as the real oracle's states are. */
  fail(to: "slow" | "unavailable"): void {
    this.state = to;
    if (to === "slow") knownSlow.add(`${this.flags}/${this.source}`);
    this.release();
  }

  private release() {
    for (const resolve of this.waiting.splice(0)) resolve();
  }
}
