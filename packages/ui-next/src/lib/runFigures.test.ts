import { describe, it, expect } from "vitest";
import { runFigures, formatAnswering } from "./runFigures";
import type { Turn } from "./agentRun";

const q = (id: number, at: number, extra: Partial<Turn> = {}): Turn => ({ id, role: "user", text: "q", calls: [], at, ...extra });
const a = (id: number, at: number, calls = 0, extra: Partial<Turn> = {}): Turn => ({
  id,
  role: "agent",
  text: "a",
  at,
  ...extra,
  calls: Array.from({ length: calls }, (_, i) => ({ id: `c${id}-${i}`, tool: "k8s.listPods", args: {}, status: "ok" as const })),
});

describe("runFigures (#386)", () => {
  it("counts every tool call across the run", () => {
    expect(runFigures([q(1, 0), a(2, 1000, 2), q(3, 2000), a(4, 3000, 1)], false).calls).toBe(3);
  });

  it("sums the time from each question to its settled answer", () => {
    expect(runFigures([q(1, 0), a(2, 4200), q(3, 10_000), a(4, 17_000)], false).answeringMs).toBe(11_200);
  });

  it("skips a pair whose time was never recorded", () => {
    expect(
      runFigures([q(1, 0, { atRecorded: false }), a(2, 9000, 0, { atRecorded: false }), q(3, 10_000), a(4, 12_000)], false)
        .answeringMs,
    ).toBe(2000);
  });

  it("does not count the answer still in flight", () => {
    expect(runFigures([q(1, 0), a(2, 3000), q(3, 5000), a(4, 5000)], true).answeringMs).toBe(3000);
  });

  it("knows nothing, rather than zero, when nothing could be measured", () => {
    expect(runFigures([q(1, 0)], true).answeringMs).toBeNull();
    expect(runFigures([], false)).toEqual({ calls: 0, answeringMs: null });
  });

  it("counts an error turn as the question's answer", () => {
    expect(runFigures([q(1, 0), { ...a(2, 1500), role: "error" }], false).answeringMs).toBe(1500);
  });
});

describe("formatAnswering", () => {
  it.each([
    [11_200, "11.2s"],
    [400, "0.4s"],
    [125_000, "2m 5s"],
    [120_000, "2m"],
    [3_780_000, "1h 3m"],
  ])("%d ms reads %s", (ms, text) => {
    expect(formatAnswering(ms)).toBe(text);
  });
});
