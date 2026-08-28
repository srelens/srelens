import { describe, it, expect, vi } from "vitest";
import { podOverview } from "./podOverview";

const OUT = {
  total: 4,
  byNode: [
    { node: "n1", pods: 2 },
    { node: "n2", pods: 1 },
  ],
  unsettled: [
    {
      name: "aa-worker-0",
      namespace: "checkout",
      phase: "Running",
      ready: "0/1",
      restarts: 4,
      node: "n3",
      age: "3d",
      image: "acme/api:1",
      waitingReason: "CrashLoopBackOff",
    },
  ],
  truncated: false,
};

describe("podOverview", () => {
  it("asks the backend for one context's pod facts", async () => {
    const invoke = vi.fn().mockResolvedValue(OUT);
    const outcome = await podOverview("prod-eu", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.podOverview", { context: "prod-eu" });
    expect(outcome.pods?.total).toBe(4);
    expect(outcome.pods?.unsettled[0].waitingReason).toBe("CrashLoopBackOff");
    expect(outcome.error).toBeUndefined();
  });

  it("keeps the per-node counts as the backend grouped them", async () => {
    const outcome = await podOverview("prod-eu", vi.fn().mockResolvedValue(OUT));
    expect(outcome.pods?.byNode).toEqual([
      { node: "n1", pods: 2 },
      { node: "n2", pods: 1 },
    ]);
  });

  it("reads an empty cluster as an answer, because it is one", async () => {
    const empty = { total: 0, byNode: [], unsettled: [], truncated: false };
    const outcome = await podOverview("empty", vi.fn().mockResolvedValue(empty));
    expect(outcome.pods).toEqual(empty);
    expect(outcome.error).toBeUndefined();
  });

  it("returns the reason rather than a cluster with no pods when the call fails", async () => {
    const outcome = await podOverview("prod-eu", () =>
      Promise.reject(new Error("pod overview timed out")),
    );
    // A cluster that did not answer has not told us it has no pods: nothing
    // downstream may read this as `total: 0`.
    expect(outcome.pods).toBeUndefined();
    expect(outcome.error).toContain("pod overview timed out");
  });

  it("carries the backend's truncation rather than presenting a short list as whole", async () => {
    const short = { ...OUT, truncated: true };
    const outcome = await podOverview("prod-eu", vi.fn().mockResolvedValue(short));
    expect(outcome.pods?.truncated).toBe(true);
  });
});
