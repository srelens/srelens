import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { answer, type RegexItem } from "./regexMatch";
import { clearSlowPatterns, createRegexOracle, isKnownSlow, type WorkerLike } from "./regexWorker";

interface Batch {
  readonly id: number;
  readonly source: string;
  readonly flags: string;
  readonly items: readonly RegexItem[];
}

/**
 * A worker the test drives: it says nothing until told to. `start` is it taking
 * up the batch it was last sent, and `finish` is it answering that batch as the
 * real worker does.
 */
function fakeWorker() {
  const posted: Batch[] = [];
  const worker: WorkerLike = {
    onmessage: null,
    onerror: null,
    terminate: vi.fn(),
    postMessage: (message) => posted.push(message as Batch),
  };
  const say = (data: unknown) => worker.onmessage?.({ data });
  const last = () => posted[posted.length - 1];
  return {
    worker,
    posted,
    start: () => say({ id: last().id, started: true }),
    finish: () => say({ id: last().id, answers: answer(last().source, last().flags, last().items) }),
  };
}

/** Whether a promise has settled by now. */
async function settled(promise: Promise<unknown>): Promise<boolean> {
  let done = false;
  void promise.then(() => (done = true));
  await vi.advanceTimersByTimeAsync(0);
  return done;
}

beforeEach(() => {
  clearSlowPatterns();
  vi.useFakeTimers();
});
afterEach(() => vi.useRealTimers());

describe("createRegexOracle", () => {
  it("answers what it is asked through one worker, and keeps the answers", async () => {
    const fake = fakeWorker();
    const factory = vi.fn(() => fake.worker);
    const oracle = createRegexOracle("a+b", "i", { worker: factory });
    const asked = oracle.ask([{ test: "xaab", shown: "x AAB" }, { test: "zz" }]);
    expect(oracle.get("xaab")).toBeUndefined();
    expect(fake.posted).toEqual([{ id: 1, source: "a+b", flags: "i", items: [{ test: "xaab", shown: "x AAB" }, { test: "zz" }] }]);
    fake.start();
    fake.finish();
    expect(await settled(asked)).toBe(true);
    expect(oracle.get("xaab")).toEqual({ hit: true, ranges: [[2, 5]] });
    expect(oracle.get("zz")).toEqual({ hit: false });
    expect(oracle.state).toBe("ready");
    expect(factory).toHaveBeenCalledTimes(1);
    expect(fake.worker.terminate).not.toHaveBeenCalled();
  });

  it("starts no worker until it is asked something", () => {
    const factory = vi.fn(() => fakeWorker().worker);
    const oracle = createRegexOracle("a+b", "i", { worker: factory });
    void oracle.ask([]);
    expect(factory).not.toHaveBeenCalled();
  });

  it("sends a batch at a time, each with only the strings not yet answered or on their way", async () => {
    const fake = fakeWorker();
    const oracle = createRegexOracle("a+b", "i", { worker: () => fake.worker });
    const first = oracle.ask([{ test: "aab" }, { test: "zz" }]);
    const second = oracle.ask([{ test: "aab" }, { test: "new" }, { test: "new" }]);
    // The second waits on the first: one batch is in the worker at a time.
    expect(fake.posted).toHaveLength(1);
    fake.start();
    fake.finish();
    expect(await settled(first)).toBe(true);
    expect(await settled(second)).toBe(false);
    expect(fake.posted.map((b) => b.items)).toEqual([[{ test: "aab" }, { test: "zz" }], [{ test: "new" }]]);
    fake.start();
    fake.finish();
    expect(await settled(second)).toBe(true);
    // Nothing new: nothing is sent, and it resolves at once.
    expect(await settled(oracle.ask([{ test: "zz" }]))).toBe(true);
    expect(fake.posted).toHaveLength(2);
  });

  it("sends what was asked while a batch ran as one batch, so a backlog it can't keep up with runs past the budget", async () => {
    const fake = fakeWorker();
    const oracle = createRegexOracle("a", "i", { worker: () => fake.worker });
    void oracle.ask([{ test: "1" }]);
    const later = [oracle.ask([{ test: "2" }]), oracle.ask([{ test: "3" }]), oracle.ask([{ test: "4" }])];
    fake.start();
    fake.finish();
    await vi.advanceTimersByTimeAsync(0);
    expect(fake.posted.map((b) => b.items.map((i) => i.test))).toEqual([["1"], ["2", "3", "4"]]);
    fake.start();
    fake.finish();
    for (const asked of later) expect(await settled(asked)).toBe(true);
    expect(["2", "3", "4"].map((t) => oracle.get(t))).toEqual([{ hit: false }, { hit: false }, { hit: false }]);
  });

  it("goes slow 500 ms after the worker starts a batch, ends the worker, and remembers the pattern", async () => {
    const fake = fakeWorker();
    const oracle = createRegexOracle("(a+)+$", "i", { worker: () => fake.worker });
    const asked = oracle.ask([{ test: "aaaa!" }]);
    const waiting = oracle.ask([{ test: "later" }]);
    fake.start();
    await vi.advanceTimersByTimeAsync(499);
    expect(oracle.state).toBe("ready");
    expect(fake.worker.terminate).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(oracle.state).toBe("slow");
    expect(fake.worker.terminate).toHaveBeenCalledTimes(1);
    expect(isKnownSlow("(a+)+$", "i")).toBe(true);
    expect(isKnownSlow("(a+)+$", "")).toBe(false);
    // Nothing waits on an oracle that will never answer.
    expect(await settled(asked)).toBe(true);
    expect(await settled(waiting)).toBe(true);
  });

  it("times each batch from the worker's start, so a slow start-up is not held against the pattern", async () => {
    const fake = fakeWorker();
    const oracle = createRegexOracle("a+b", "i", { worker: () => fake.worker });
    void oracle.ask([{ test: "aab" }]);
    await vi.advanceTimersByTimeAsync(4000);
    fake.start();
    await vi.advanceTimersByTimeAsync(499);
    fake.finish();
    await vi.advanceTimersByTimeAsync(1000);
    expect(oracle.state).toBe("ready");
    expect(oracle.get("aab")).toEqual({ hit: true });
  });

  it("is unavailable, not slow, when the worker has not started within 5 seconds, and ends it", async () => {
    const fake = fakeWorker();
    const oracle = createRegexOracle("a+b", "i", { worker: () => fake.worker });
    const asked = oracle.ask([{ test: "aab" }]);
    await vi.advanceTimersByTimeAsync(4999);
    expect(oracle.state).toBe("ready");
    await vi.advanceTimersByTimeAsync(1);
    expect(oracle.state).toBe("unavailable");
    expect(fake.worker.terminate).toHaveBeenCalledTimes(1);
    expect(isKnownSlow("a+b", "i")).toBe(false);
    expect(await settled(asked)).toBe(true);
  });

  it("is unavailable when the worker cannot be made", async () => {
    const oracle = createRegexOracle("a+b", "i", {
      worker: () => {
        throw new Error("Worker is not defined");
      },
    });
    expect(await settled(oracle.ask([{ test: "aab" }]))).toBe(true);
    expect(oracle.state).toBe("unavailable");
    expect(isKnownSlow("a+b", "i")).toBe(false);
  });

  it("is unavailable when the worker fails, and ends it", async () => {
    const fake = fakeWorker();
    const oracle = createRegexOracle("a+b", "i", { worker: () => fake.worker });
    const asked = oracle.ask([{ test: "aab" }]);
    fake.worker.onerror?.(new Error("boom"));
    expect(oracle.state).toBe("unavailable");
    expect(fake.worker.terminate).toHaveBeenCalledTimes(1);
    expect(await settled(asked)).toBe(true);
  });

  it("is slow from the start for a pattern already found slow, and makes no worker", async () => {
    const fake = fakeWorker();
    void createRegexOracle("(a+)+$", "i", { worker: () => fake.worker }).ask([{ test: "aaaa!" }]);
    fake.start();
    await vi.advanceTimersByTimeAsync(500);
    const factory = vi.fn(() => fakeWorker().worker);
    const again = createRegexOracle("(a+)+$", "i", { worker: factory });
    expect(again.state).toBe("slow");
    expect(await settled(again.ask([{ test: "aaaa!" }]))).toBe(true);
    expect(factory).not.toHaveBeenCalled();
  });

  it("ends the worker when disposed, and takes no answer after that", async () => {
    const fake = fakeWorker();
    const oracle = createRegexOracle("a+b", "i", { worker: () => fake.worker });
    const asked = oracle.ask([{ test: "aab" }]);
    fake.start();
    oracle.dispose();
    expect(fake.worker.terminate).toHaveBeenCalledTimes(1);
    expect(await settled(asked)).toBe(true);
    fake.finish();
    expect(oracle.get("aab")).toBeUndefined();
    await vi.advanceTimersByTimeAsync(1000);
    expect(oracle.state).toBe("ready");
  });

  it("takes no answer that comes after it went slow", async () => {
    const fake = fakeWorker();
    const oracle = createRegexOracle("(a+)+$", "i", { worker: () => fake.worker });
    void oracle.ask([{ test: "aaaa!" }]);
    fake.start();
    await vi.advanceTimersByTimeAsync(500);
    fake.finish();
    fake.worker.onerror?.(new Error("late"));
    expect(oracle.get("aaaa!")).toBeUndefined();
    expect(oracle.state).toBe("slow");
  });

  it("forgets every answer but the strings it is told to keep", async () => {
    const fake = fakeWorker();
    const oracle = createRegexOracle("a+b", "i", { worker: () => fake.worker });
    void oracle.ask([{ test: "aab" }, { test: "zz" }]);
    fake.start();
    fake.finish();
    expect(oracle.size).toBe(2);
    oracle.prune(new Set(["zz"]));
    expect(oracle.size).toBe(1);
    expect(oracle.get("aab")).toBeUndefined();
    expect(oracle.get("zz")).toEqual({ hit: false });
  });
});
