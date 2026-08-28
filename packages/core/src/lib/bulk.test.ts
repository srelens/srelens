import { describe, it, expect, vi } from "vitest";
import { runBulk, summarize } from "./bulk";

describe("runBulk", () => {
  it("returns a per-item outcome in original order", async () => {
    const out = await runBulk([1, 2, 3], async (n) => (n === 2 ? { error: "boom" } : { ok: true }));
    expect(out).toEqual([
      { item: 1, status: "ok" },
      { item: 2, status: "error", error: "boom" },
      { item: 3, status: "ok" },
    ]);
  });

  it("does not abort the rest when one item fails or throws", async () => {
    const seen: number[] = [];
    const out = await runBulk([1, 2, 3, 4], async (n) => {
      seen.push(n);
      if (n === 1) throw new Error("thrown");
      if (n === 3) return { error: "returned" };
      return { deleted: true };
    });
    expect(seen.sort()).toEqual([1, 2, 3, 4]); // every item attempted
    expect(summarize(out)).toEqual({ ok: 2, failed: 2 });
    expect(out[0]).toEqual({ item: 1, status: "error", error: "Error: thrown" });
  });

  it("bounds concurrency to at most `concurrency` in flight", async () => {
    let inFlight = 0;
    let peak = 0;
    const op = vi.fn(async () => {
      inFlight++;
      peak = Math.max(peak, inFlight);
      await new Promise((r) => setTimeout(r, 5));
      inFlight--;
      return { ok: true };
    });
    await runBulk(Array.from({ length: 20 }, (_, i) => i), op, 4);
    expect(op).toHaveBeenCalledTimes(20);
    expect(peak).toBeLessThanOrEqual(4);
  });

  it("handles an empty list", async () => {
    const op = vi.fn();
    expect(await runBulk([], op)).toEqual([]);
    expect(op).not.toHaveBeenCalled();
  });

  it("treats `{ deleted: true }` and `{ ok: true }` as success (no error only)", async () => {
    const out = await runBulk(["a", "b"], async (s) => (s === "a" ? { deleted: true } : { ok: true }));
    expect(summarize(out)).toEqual({ ok: 2, failed: 0 });
  });
});
