import { beforeEach, describe, expect, it } from "vitest";
import { USAGE_WINDOW_MS, __resetUsageHistoryForTests, recordUsage, usageHistory } from "./usageHistory";

const T0 = Date.UTC(2026, 9, 9, 10, 0, 0);
const S = 1000;
const one = (cpu: number, memory = 100) => [{ key: "shop/web-0", cpu, memory }];

beforeEach(() => __resetUsageHistoryForTests());

describe("recordUsage", () => {
  it("starts a pod's history with its first reading, and nothing before it", () => {
    const out = recordUsage("dev", one(120), T0);
    expect(out.get("shop/web-0")).toEqual([{ at: T0, cpu: 120, memory: 100 }]);
  });

  it("adds each later reading to the end, oldest first", () => {
    recordUsage("dev", one(120), T0);
    recordUsage("dev", one(150), T0 + 10 * S);
    const out = recordUsage("dev", one(90), T0 + 20 * S);
    expect(out.get("shop/web-0")?.map((s) => s.cpu)).toEqual([120, 150, 90]);
  });

  it("hands back a new array each time, so a row given it draws the new point", () => {
    const first = recordUsage("dev", one(120), T0).get("shop/web-0");
    const second = recordUsage("dev", one(150), T0 + 10 * S).get("shop/web-0");
    expect(second).not.toBe(first);
    expect(first).toHaveLength(1);
  });

  it("keeps ten minutes and lets the rest go", () => {
    recordUsage("dev", one(1), T0);
    recordUsage("dev", one(2), T0 + 60 * S);
    const out = recordUsage("dev", one(3), T0 + USAGE_WINDOW_MS + 30 * S);
    // The first is past the window; the second, a minute later, is not.
    expect(out.get("shop/web-0")?.map((s) => s.cpu)).toEqual([2, 3]);
  });

  it("keeps a reading taken exactly ten minutes ago", () => {
    recordUsage("dev", one(1), T0);
    expect(recordUsage("dev", one(2), T0 + USAGE_WINDOW_MS).get("shop/web-0")).toHaveLength(2);
  });

  it("takes two readings moments apart as one, the later replacing the earlier", () => {
    // Two tabs on one list, each polling on its own clock.
    recordUsage("dev", one(120), T0);
    const out = recordUsage("dev", one(125), T0 + 2 * S);
    expect(out.get("shop/web-0")).toEqual([{ at: T0 + 2 * S, cpu: 125, memory: 100 }]);
  });

  it("keeps pods apart, and clusters apart", () => {
    recordUsage("dev", [{ key: "shop/web-0", cpu: 1, memory: 1 }, { key: "shop/web-1", cpu: 2, memory: 2 }], T0);
    recordUsage("prod", [{ key: "shop/web-0", cpu: 9, memory: 9 }], T0);
    expect(usageHistory("dev", "shop/web-0", T0).map((s) => s.cpu)).toEqual([1]);
    expect(usageHistory("dev", "shop/web-1", T0).map((s) => s.cpu)).toEqual([2]);
    expect(usageHistory("prod", "shop/web-0", T0).map((s) => s.cpu)).toEqual([9]);
  });

  it("does not forget a pod for being absent from one batch", () => {
    // A batch is one namespace of several; the others' pods are not in it.
    recordUsage("dev", [{ key: "shop/web-0", cpu: 1, memory: 1 }], T0);
    recordUsage("dev", [{ key: "pay/api-0", cpu: 5, memory: 5 }], T0 + 10 * S);
    expect(usageHistory("dev", "shop/web-0", T0 + 10 * S)).toHaveLength(1);
  });

  it("forgets a pod whose last reading has aged out, as a deleted pod's does", () => {
    recordUsage("dev", [{ key: "shop/gone-0", cpu: 1, memory: 1 }], T0);
    recordUsage("dev", [{ key: "shop/web-0", cpu: 2, memory: 2 }], T0 + USAGE_WINDOW_MS + S);
    expect(usageHistory("dev", "shop/gone-0", T0 + USAGE_WINDOW_MS + S)).toEqual([]);
    // And a pod of the same name created later starts afresh.
    const out = recordUsage("dev", [{ key: "shop/gone-0", cpu: 7, memory: 7 }], T0 + USAGE_WINDOW_MS + 20 * S);
    expect(out.get("shop/gone-0")).toHaveLength(1);
  });

  it("never leaves points out of order when the clock is put back", () => {
    recordUsage("dev", one(1), T0);
    recordUsage("dev", one(2), T0 + 30 * S);
    const out = recordUsage("dev", one(3), T0 + 28 * S).get("shop/web-0")!;
    expect(out.map((s) => s.at)).toEqual([...out.map((s) => s.at)].sort((a, b) => a - b));
    expect(out[out.length - 1].cpu).toBe(3);
  });
});

describe("usageHistory", () => {
  it("is empty for a pod nothing has been read for", () => {
    expect(usageHistory("dev", "shop/web-0", T0)).toEqual([]);
  });

  it("shows only what is inside the window at the moment it is asked", () => {
    recordUsage("dev", one(1), T0);
    expect(usageHistory("dev", "shop/web-0", T0 + USAGE_WINDOW_MS + S)).toEqual([]);
  });
});
