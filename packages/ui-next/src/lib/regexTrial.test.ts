import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cachedVerdict, clearTrialCache, runTrial, trialRegex, type Verdict, type WorkerLike } from "./regexTrial";

/**
 * A worker the test drives. `answer` is what it does with the message it is
 * given: nothing (a regex that never finishes), a "done" (one that does), or a
 * crash. Whatever it does, the test can see what was posted and that it was
 * terminated.
 */
function fakeWorker(answer: (w: WorkerLike) => void = () => {}) {
  const posted: unknown[] = [];
  const worker = {
    onmessage: null,
    onerror: null,
    terminate: vi.fn(),
    postMessage(m: unknown) {
      posted.push(m);
      // As a real worker does: the reply comes after the caller has returned.
      queueMicrotask(() => answer(worker));
    },
  } as WorkerLike;
  return { worker, posted };
}

/** What the real worker says first: it has the job and is about to run the pattern. */
const starts = (w: WorkerLike) => w.onmessage?.({ data: "started" });
/** And what it says when the pattern has been run over every text. */
const finishes = (w: WorkerLike) => {
  starts(w);
  w.onmessage?.({ data: "done" });
};

beforeEach(() => clearTrialCache());
afterEach(() => vi.useRealTimers());

describe("runTrial", () => {
  it("tests every text against the pattern, with its flags", () => {
    const test = vi.spyOn(RegExp.prototype, "test");
    try {
      runTrial("a+b", "i", ["xaab", "zz"]);
      expect(test.mock.contexts.map((re) => (re as RegExp).flags)).toEqual(["i", "i"]);
      expect(test.mock.calls.map(([text]) => text)).toEqual(["xaab", "zz"]);
    } finally {
      test.mockRestore();
    }
  });

  it("returns on an empty list of texts", () => {
    expect(() => runTrial("a+b", "i", [])).not.toThrow();
  });
});

describe("trialRegex", () => {
  it("resolves ok when the worker answers, after posting it the pattern and the texts, and ends the worker", async () => {
    const { worker, posted } = fakeWorker(finishes);
    await expect(trialRegex("a+b", "i", ["xaab"], { worker: () => worker })).resolves.toBe("ok");
    expect(posted).toEqual([{ source: "a+b", flags: "i", texts: ["xaab"] }]);
    expect(worker.terminate).toHaveBeenCalledTimes(1);
  });

  it("resolves slow 500 ms after the worker starts when it never finishes, and ends the worker", async () => {
    vi.useFakeTimers();
    const { worker } = fakeWorker(starts);
    let verdict: Verdict | undefined;
    void trialRegex("(a+)+$", "i", ["aaaa"], { worker: () => worker }).then((v) => (verdict = v));
    await vi.advanceTimersByTimeAsync(499);
    expect(verdict).toBeUndefined();
    expect(worker.terminate).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(verdict).toBe("slow");
    expect(worker.terminate).toHaveBeenCalledTimes(1);
  });

  it("counts the budget from the worker's start, so a slow start-up is not held against the pattern", async () => {
    vi.useFakeTimers();
    const { worker } = fakeWorker();
    let verdict: Verdict | undefined;
    void trialRegex("a+b", "i", ["xaab"], { worker: () => worker }).then((v) => (verdict = v));
    await vi.advanceTimersByTimeAsync(400);
    worker.onmessage?.({ data: "started" });
    // 400 ms into the call is within the budget; so is 800 ms, which is only 400 ms after the start.
    await vi.advanceTimersByTimeAsync(400);
    expect(verdict).toBeUndefined();
    expect(worker.terminate).not.toHaveBeenCalled();
    worker.onmessage?.({ data: "done" });
    await vi.advanceTimersByTimeAsync(0);
    expect(verdict).toBe("ok");
    expect(worker.terminate).toHaveBeenCalledTimes(1);
  });

  it("resolves ok, and ends the worker, when it has not started within 5 seconds", async () => {
    vi.useFakeTimers();
    const { worker } = fakeWorker();
    let verdict: Verdict | undefined;
    void trialRegex("a+b", "i", ["xaab"], { worker: () => worker }).then((v) => (verdict = v));
    // Past the 500 ms budget, which does not run until the worker has started.
    await vi.advanceTimersByTimeAsync(4999);
    expect(verdict).toBeUndefined();
    expect(worker.terminate).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(verdict).toBe("ok");
    expect(worker.terminate).toHaveBeenCalledTimes(1);
    expect(cachedVerdict("a+b", "i")).toBe("ok");
  });

  it("starts no budget for a start that comes after it gave up waiting", async () => {
    vi.useFakeTimers();
    const { worker } = fakeWorker();
    const trial = trialRegex("a+b", "i", ["xaab"], { worker: () => worker });
    await vi.advanceTimersByTimeAsync(5000);
    await expect(trial).resolves.toBe("ok");
    worker.onmessage?.({ data: "started" });
    expect(vi.getTimerCount()).toBe(0);
    expect(cachedVerdict("a+b", "i")).toBe("ok");
  });

  it("takes its budget from the options", async () => {
    vi.useFakeTimers();
    const { worker } = fakeWorker(starts);
    let verdict: Verdict | undefined;
    void trialRegex("(a+)+$", "i", [], { budgetMs: 50, worker: () => worker }).then((v) => (verdict = v));
    await vi.advanceTimersByTimeAsync(50);
    expect(verdict).toBe("slow");
  });

  it("resolves ok when the worker cannot be made", async () => {
    const factory = vi.fn(() => {
      throw new Error("Worker is not defined");
    });
    await expect(trialRegex("a+b", "i", ["xaab"], { worker: factory })).resolves.toBe("ok");
    expect(factory).toHaveBeenCalledTimes(1);
  });

  it("resolves ok when the worker fails, and ends it", async () => {
    const { worker } = fakeWorker((w) => w.onerror?.(new Error("boom")));
    await expect(trialRegex("a+b", "i", ["xaab"], { worker: () => worker })).resolves.toBe("ok");
    expect(worker.terminate).toHaveBeenCalledTimes(1);
  });

  it("answers a pattern it has tried from memory, without a second worker", async () => {
    const first = fakeWorker(finishes);
    await trialRegex("a+b", "i", ["xaab"], { worker: () => first.worker });
    const factory = vi.fn(() => fakeWorker(finishes).worker);
    await expect(trialRegex("a+b", "i", ["other"], { worker: factory })).resolves.toBe("ok");
    expect(factory).not.toHaveBeenCalled();
    expect(cachedVerdict("a+b", "i")).toBe("ok");
  });

  it("remembers a slow pattern too, so a restored query does not wait the budget out again", async () => {
    vi.useFakeTimers();
    const { worker } = fakeWorker(starts);
    const trial = trialRegex("(a+)+$", "i", ["aaaa"], { worker: () => worker });
    await vi.advanceTimersByTimeAsync(500);
    await expect(trial).resolves.toBe("slow");
    const factory = vi.fn(() => fakeWorker().worker);
    await expect(trialRegex("(a+)+$", "i", ["aaaa"], { worker: factory })).resolves.toBe("slow");
    expect(factory).not.toHaveBeenCalled();
    expect(cachedVerdict("(a+)+$", "i")).toBe("slow");
  });

  it("keys the memory on the flags as well as the pattern", async () => {
    const { worker } = fakeWorker(finishes);
    await trialRegex("a+b", "i", ["xaab"], { worker: () => worker });
    expect(cachedVerdict("a+b", "i")).toBe("ok");
    expect(cachedVerdict("a+b", "")).toBeUndefined();
    expect(cachedVerdict("a+c", "i")).toBeUndefined();
  });

  it("is not moved off slow by an answer that comes after the budget", async () => {
    vi.useFakeTimers();
    const { worker } = fakeWorker(starts);
    const trial = trialRegex("(a+)+$", "i", ["aaaa"], { worker: () => worker });
    await vi.advanceTimersByTimeAsync(500);
    await expect(trial).resolves.toBe("slow");
    worker.onmessage?.({ data: "done" });
    worker.onerror?.(new Error("late"));
    expect(cachedVerdict("(a+)+$", "i")).toBe("slow");
    expect(worker.terminate).toHaveBeenCalledTimes(1);
  });
});
