import { describe, it, expect, vi } from "vitest";
import { countResources, COUNT_MAX_NAMESPACES, type CountKindRef, type KindCountResult } from "./countResources";

/** What the backend answers for `kinds`: each counted as its index. */
const answer = (kinds: CountKindRef[]): { counts: KindCountResult[] } => ({
  counts: kinds.map((k, i) => ({ kind: k.kind, group: k.group ?? "", count: i })),
});

describe("countResources", () => {
  it("passes context, kinds and namespaces through", async () => {
    const invoke = vi.fn().mockResolvedValue({ counts: [{ kind: "Pod", group: "", count: 38 }] });
    const outcome = await countResources("prod", [{ kind: "Pod" }], ["a"], invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.countResources", { context: "prod", kinds: [{ kind: "Pod" }], namespaces: ["a"] });
    expect(outcome).toEqual({ counts: [{ kind: "Pod", group: "", count: 38 }] });
  });

  it("normalises a rejection into an error, never counts", async () => {
    const outcome = await countResources("prod", [{ kind: "Pod" }], [], () => Promise.reject(new Error("the cluster did not answer")));
    expect(outcome.counts).toBeUndefined();
    expect(outcome.error).toContain("did not answer");
  });

  /**
   * 256, well under the backend's 1024: each call has one time budget and
   * runs 8 requests at a time, and 1024 requests at 80 ms each would need
   * ~10 s — the kinds at the end of a full chunk would time out every cycle.
   */
  it("splits kinds so each call stays within 256 requests, one call after another", async () => {
    const order: string[] = [];
    const invoke = vi.fn(async (_id: string, input: { kinds: CountKindRef[] }) => {
      order.push(`start ${input.kinds.length}`);
      await new Promise((r) => setTimeout(r, 5));
      order.push(`end ${input.kinds.length}`);
      return { counts: input.kinds.map((k) => ({ kind: k.kind, group: "", count: Number(k.kind.slice(1)) })) };
    });
    const kinds = Array.from({ length: 600 }, (_, i) => ({ kind: `K${i}` }));
    const outcome = await countResources("prod", kinds, ["a", "b"], invoke as never);
    expect(invoke.mock.calls.map((c) => (c[1] as { kinds: unknown[] }).kinds.length)).toEqual([128, 128, 128, 128, 88]);
    expect(order).toEqual(["start 128", "end 128", "start 128", "end 128", "start 128", "end 128", "start 128", "end 128", "start 88", "end 88"]);
    expect(outcome.counts?.map((c) => c.count)).toEqual(kinds.map((_, i) => i));
  });

  it("a failed chunk fails the whole outcome", async () => {
    const invoke = vi
      .fn()
      .mockImplementationOnce(async (_id: string, input: { kinds: CountKindRef[] }) => answer(input.kinds))
      .mockRejectedValueOnce(new Error("timed out"));
    const kinds = Array.from({ length: 1025 }, (_, i) => ({ kind: `K${i}` }));
    const outcome = await countResources("prod", kinds, [], invoke);
    expect(outcome.counts).toBeUndefined();
    expect(outcome.error).toContain("timed out");
  });

  it("over 128 namespaces, namespaced kinds get no number and no request", async () => {
    const invoke = vi.fn(async (_id: string, input: { kinds: CountKindRef[] }) => ({
      counts: input.kinds.map((k) => ({ kind: k.kind, group: "", count: 3 })),
    }));
    const namespaces = Array.from({ length: COUNT_MAX_NAMESPACES + 1 }, (_, i) => `ns${i}`);
    const outcome = await countResources(
      "prod",
      [{ kind: "Pod", namespaced: true }, { kind: "Node", namespaced: false }],
      namespaces,
      invoke as never,
    );
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("k8s.countResources", {
      context: "prod",
      kinds: [{ kind: "Node", namespaced: false }],
      namespaces: [],
    });
    expect(outcome.counts?.[0]).toMatchObject({ kind: "Pod", group: "", count: null });
    expect(typeof outcome.counts?.[0].error).toBe("string");
    expect(outcome.counts?.[1]).toEqual({ kind: "Node", group: "", count: 3 });
  });

  it("no kinds, no call", async () => {
    const invoke = vi.fn();
    expect(await countResources("prod", [], [], invoke)).toEqual({ counts: [] });
    expect(invoke).not.toHaveBeenCalled();
  });
});
