import { afterEach, describe, expect, it, vi } from "vitest";

/**
 * The worker file is run here as a script in jsdom, where `self` is the window:
 * it sets `self.onmessage`, and the test is the host that sends it a job. What it
 * says back, and when, is what `trialRegex` times the budget from.
 */
type Handler = (event: { data: unknown }) => void;
const scope = self as unknown as { onmessage: Handler | null };

afterEach(() => {
  vi.restoreAllMocks();
  scope.onmessage = null;
});

describe("the regex trial worker", () => {
  it("says it has started before it runs the pattern, and that it is done after", async () => {
    await import("./regexTrial.worker");
    // Recorded only while the job runs: loading a module and an assertion test strings too.
    const events: string[] = [];
    const post = vi.spyOn(self, "postMessage").mockImplementation(((message: unknown) => {
      events.push(`posted ${String(message)}`);
    }) as never);
    const real = RegExp.prototype.test;
    const test = vi.spyOn(RegExp.prototype, "test").mockImplementation(function (this: RegExp, text: string) {
      events.push(`tested ${text}`);
      return real.call(this, text);
    });
    try {
      scope.onmessage?.({ data: { source: "a+b", flags: "i", texts: ["xaab", "zz"] } });
    } finally {
      post.mockRestore();
      test.mockRestore();
    }
    expect(events).toEqual(["posted started", "tested xaab", "tested zz", "posted done"]);
  });
});
