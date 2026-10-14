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
    expect(summarize(out)).toEqual({ ok: 2, failed: 2, cancelled: 0 });
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
    expect(summarize(out)).toEqual({ ok: 2, failed: 0, cancelled: 0 });
  });
});

/** Resolves when the test says so, so a run can be held mid-flight. */
function gate() {
  let open!: () => void;
  const promise = new Promise<void>((resolve) => {
    open = resolve;
  });
  return { promise, open };
}

describe("runBulk under cancellation", () => {
  it("stops the items that have not started and lets the in-flight ones finish", async () => {
    const started: string[] = [];
    const held = gate();
    const controller = new AbortController();
    const out = runBulk(
      ["a", "b", "c", "d"],
      async (item) => {
        started.push(item);
        await held.promise;
        return { ok: true };
      },
      2,
      { signal: controller.signal },
    );
    await Promise.resolve();
    await Promise.resolve();
    expect(started).toEqual(["a", "b"]);
    controller.abort();
    held.open();
    const outcomes = await out;
    // The two already sent were not recalled: the cluster may already have them.
    expect(started).toEqual(["a", "b"]);
    expect(outcomes).toEqual([
      { item: "a", status: "ok" },
      { item: "b", status: "ok" },
      { item: "c", status: "cancelled" },
      { item: "d", status: "cancelled" },
    ]);
    expect(summarize(outcomes)).toEqual({ ok: 2, failed: 0, cancelled: 2 });
  });

  it("never hands the signal to `op`, so nothing can abort a write in flight", async () => {
    const controller = new AbortController();
    const seen: number[] = [];
    await runBulk(
      ["a"],
      async function (...args: unknown[]) {
        seen.push(args.length);
        return { ok: true };
      },
      1,
      { signal: controller.signal },
    );
    expect(seen).toEqual([1]);
  });

  it("attempts nothing when the signal is already aborted", async () => {
    const op = vi.fn(async () => ({ ok: true }));
    const controller = new AbortController();
    controller.abort();
    const outcomes = await runBulk(["a", "b"], op, 4, { signal: controller.signal });
    expect(op).not.toHaveBeenCalled();
    expect(outcomes.map((o) => o.status)).toEqual(["cancelled", "cancelled"]);
  });
});

describe("runBulk progress", () => {
  it("reports every item from pending through running to its outcome", async () => {
    const seen: string[][] = [];
    const held = gate();
    const out = runBulk(
      ["a", "b"],
      async (item) => {
        if (item === "a") await held.promise;
        return item === "b" ? { error: "denied" } : { ok: true };
      },
      1,
      { onProgress: (items) => seen.push(items.map((i) => i.state)) },
    );
    // The list exists before anything is sent: a reader sees what is queued.
    expect(seen[0]).toEqual(["pending", "pending"]);
    await Promise.resolve();
    expect(seen.at(-1)).toEqual(["running", "pending"]);
    held.open();
    await out;
    expect(seen.at(-1)).toEqual(["ok", "error"]);
  });

  it("carries the reason on the item that was rejected", async () => {
    let last: ReadonlyArray<{ item: string; state: string; error?: string }> = [];
    await runBulk(["a"], async () => ({ error: "admission webhook denied the request" }), 1, {
      onProgress: (items) => {
        last = items.map((i) => ({ ...i }));
      },
    });
    expect(last).toEqual([
      { item: "a", state: "error", error: "admission webhook denied the request" },
    ]);
  });
});
