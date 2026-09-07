import { describe, expect, it } from "vitest";
import {
  TAINT_COLUMN_HINT,
  formatTaint,
  orderTaints,
  parseTaints,
  taintBadgeLabel,
  taintBadgeText,
  taintSortValue,
  taintTally,
  taintTallyText,
  taintTimeAddedText,
  taintTooltip,
  type NodeTaint,
} from "./taints";

const taint = (key: string, effect: string, value = "", timeAdded?: string): NodeTaint => ({
  key,
  value,
  effect,
  ...(timeAdded ? { timeAdded } : {}),
});

const CONTROL_PLANE = taint("node-role.kubernetes.io/control-plane", "NoSchedule");
const MEMORY_PRESSURE = taint("node.kubernetes.io/memory-pressure", "NoSchedule");
const SPOT = taint("spot", "PreferNoSchedule", "true");
const PAYMENTS = taint("team", "NoExecute", "payments", "2026-09-02T08:15:00Z");

describe("formatTaint", () => {
  it("writes kubectl's own key=value:effect", () => {
    expect(formatTaint(PAYMENTS)).toBe("team=payments:NoExecute");
  });

  it("keeps the empty value visible, because a valueless taint is written that way", () => {
    expect(formatTaint(CONTROL_PLANE)).toBe("node-role.kubernetes.io/control-plane=:NoSchedule");
  });
});

describe("orderTaints", () => {
  it("reads worst-first: NoExecute, then NoSchedule, then PreferNoSchedule", () => {
    const ordered = orderTaints([SPOT, CONTROL_PLANE, PAYMENTS]);
    expect(ordered.map((t) => t.effect)).toEqual(["NoExecute", "NoSchedule", "PreferNoSchedule"]);
  });

  it("holds the API server's order within one effect, so a node always reads the same way", () => {
    const ordered = orderTaints([CONTROL_PLANE, MEMORY_PRESSURE]);
    expect(ordered.map((t) => t.key)).toEqual([CONTROL_PLANE.key, MEMORY_PRESSURE.key]);
  });

  it("does not drop an effect it has never heard of — it sorts it last", () => {
    const future = taint("future", "NoExecuteSoon");
    expect(orderTaints([future, PAYMENTS]).map((t) => t.key)).toEqual(["team", "future"]);
  });

  it("leaves its argument alone", () => {
    const input = [SPOT, PAYMENTS];
    orderTaints(input);
    expect(input.map((t) => t.effect)).toEqual(["PreferNoSchedule", "NoExecute"]);
  });
});

describe("taintTooltip", () => {
  it("is empty for a node with none, so a badge that isn't drawn has nothing to say", () => {
    expect(taintTooltip([])).toBe("");
  });

  it("is one line for one taint", () => {
    expect(taintTooltip([CONTROL_PLANE])).toBe("node-role.kubernetes.io/control-plane=:NoSchedule");
  });

  it("is one line per taint, worst effect first", () => {
    expect(taintTooltip([CONTROL_PLANE, SPOT, PAYMENTS]).split("\n")).toEqual([
      "team=payments:NoExecute",
      "node-role.kubernetes.io/control-plane=:NoSchedule",
      "spot=true:PreferNoSchedule",
    ]);
  });
});

describe("taintTally", () => {
  it("counts zero of everything for a node with no taints", () => {
    expect(taintTally([])).toEqual({ noSchedule: 0, preferNoSchedule: 0, noExecute: 0 });
    expect(taintTallyText([])).toBe("0 / 0 / 0");
  });

  it("counts each effect into its own bucket", () => {
    expect(taintTally([CONTROL_PLANE, MEMORY_PRESSURE, SPOT, PAYMENTS])).toEqual({
      noSchedule: 2,
      preferNoSchedule: 1,
      noExecute: 1,
    });
  });

  it("reads NoSchedule / PreferNoSchedule / NoExecute, the order the header hint names", () => {
    expect(taintTallyText([CONTROL_PLANE, MEMORY_PRESSURE, SPOT, PAYMENTS])).toBe("2 / 1 / 1");
    expect(TAINT_COLUMN_HINT).toBe("NoSchedule / PreferNoSchedule / NoExecute");
  });

  it("ignores an effect it does not know rather than miscounting it as one it does", () => {
    expect(taintTally([taint("future", "Unknown")])).toEqual({
      noSchedule: 0,
      preferNoSchedule: 0,
      noExecute: 0,
    });
  });
});

describe("taintTimeAddedText", () => {
  it("gives the time Kubernetes stamped on a NoExecute taint", () => {
    expect(taintTimeAddedText(PAYMENTS)).toBe("added 2026-09-02T08:15:00Z");
  });

  it("says there is no time in words, never as an em dash", () => {
    // The classic design's `KV` drops any row whose value is "—", so an em
    // dash here would delete the taint from that page rather than style it.
    expect(taintTimeAddedText(CONTROL_PLANE)).toBe("time not recorded");
    expect(taintTimeAddedText(CONTROL_PLANE)).not.toBe("—");
  });
});

describe("taintBadgeLabel", () => {
  it("spells the count out, because a screen reader reads '· 2' as punctuation", () => {
    expect(taintBadgeLabel(2)).toBe("2 taints");
  });

  it("is singular for one", () => {
    expect(taintBadgeLabel(1)).toBe("1 taint");
  });

  it("puts the count on the badge's own text too", () => {
    expect(taintBadgeText(3)).toBe("Tainted · 3");
  });
});

describe("taintSortValue", () => {
  it("sorts a node with no taints as 0, not out of the order", () => {
    expect(taintSortValue([])).toBe(0);
    expect(taintSortValue(undefined)).toBe(0);
  });

  it("orders by count, most-constrained first when sorted descending", () => {
    const nodes = [
      { name: "three", taints: [CONTROL_PLANE, SPOT, PAYMENTS] },
      { name: "none", taints: [] as NodeTaint[] },
      { name: "one", taints: [CONTROL_PLANE] },
    ];
    const byTaints = [...nodes].sort((a, b) => taintSortValue(b.taints) - taintSortValue(a.taints));
    expect(byTaints.map((n) => n.name)).toEqual(["three", "one", "none"]);
  });

  it("never lets the effect mix outrank the count", () => {
    // One NoExecute taint is more disruptive than two PreferNoSchedule ones,
    // and still sorts below them: the column's promise is the count.
    expect(taintSortValue([PAYMENTS])).toBeLessThan(taintSortValue([SPOT, SPOT]));
  });

  it("breaks a tie on the more disruptive effect", () => {
    expect(taintSortValue([PAYMENTS, PAYMENTS])).toBeGreaterThan(taintSortValue([SPOT, SPOT]));
  });
});

describe("parseTaints", () => {
  it("reads spec.taints off a live object", () => {
    expect(
      parseTaints({
        taints: [{ key: "team", value: "payments", effect: "NoExecute", timeAdded: "2026-09-02T08:15:00Z" }],
      }),
    ).toEqual([PAYMENTS]);
  });

  it("keeps the cordon taint the list leaves out — this is the drill-down", () => {
    const parsed = parseTaints({
      taints: [
        { key: "node.kubernetes.io/unschedulable", effect: "NoSchedule" },
        { key: "dedicated", effect: "NoSchedule" },
      ],
    });
    expect(parsed.map((t) => t.key)).toEqual(["node.kubernetes.io/unschedulable", "dedicated"]);
  });

  it("reports none for a spec that is not there, or carries no taints", () => {
    expect(parseTaints(undefined)).toEqual([]);
    expect(parseTaints({})).toEqual([]);
    expect(parseTaints({ taints: "several" })).toEqual([]);
    expect(parseTaints("nonsense")).toEqual([]);
  });

  it("drops an entry with nothing to identify it, and keeps the rest", () => {
    const parsed = parseTaints({ taints: [null, { effect: "NoSchedule" }, { key: "real", effect: "NoExecute" }] });
    expect(parsed).toEqual([{ key: "real", value: "", effect: "NoExecute" }]);
  });

  it("omits timeAdded rather than carrying an empty string for it", () => {
    expect(parseTaints({ taints: [{ key: "k", effect: "NoSchedule" }] })[0]).not.toHaveProperty("timeAdded");
  });
});
