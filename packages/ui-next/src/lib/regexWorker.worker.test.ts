import { afterEach, describe, expect, it, vi } from "vitest";

/**
 * The worker file is run here as a script in jsdom, where `self` is the window:
 * it sets `self.onmessage`, and the test is the host that sends it a batch. What
 * it says back, and when, is what the oracle times the budget from.
 */
type Handler = (event: { data: unknown }) => void;
const scope = self as unknown as { onmessage: Handler | null };

afterEach(() => {
  vi.restoreAllMocks();
  scope.onmessage = null;
});

describe("the regex worker", () => {
  it("says it has a batch before it runs the pattern, then answers it under the batch's id", async () => {
    await import("./regexWorker.worker");
    // Recorded only while the batch runs: loading a module and an assertion test strings too.
    const events: unknown[] = [];
    const post = vi.spyOn(self, "postMessage").mockImplementation(((message: unknown) => {
      events.push(message);
    }) as never);
    const real = RegExp.prototype.test;
    const test = vi.spyOn(RegExp.prototype, "test").mockImplementation(function (this: RegExp, text: string) {
      events.push(`tested ${text}`);
      return real.call(this, text);
    });
    try {
      scope.onmessage?.({ data: { id: 7, source: "a+b", flags: "i", items: [{ test: "xaab", shown: "x AAB" }, { test: "zz" }] } });
    } finally {
      post.mockRestore();
      test.mockRestore();
    }
    expect(events).toEqual([
      { id: 7, started: true },
      "tested xaab",
      "tested zz",
      { id: 7, answers: [{ hit: true, ranges: [[2, 5]] }, { hit: false }] },
    ]);
  });
});
