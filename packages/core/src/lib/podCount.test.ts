import { describe, it, expect, vi } from "vitest";
import { podCount } from "./podCount";

describe("podCount", () => {
  it("passes the context through and returns running/total", async () => {
    const invoke = vi.fn().mockResolvedValue({ running: 1284, total: 1310 });
    const outcome = await podCount("prod-eu", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.podCount", { context: "prod-eu" });
    expect(outcome.error).toBeUndefined();
    expect(outcome.counts).toEqual({ running: 1284, total: 1310 });
  });

  it("passes through an empty cluster's zero counts", async () => {
    const invoke = vi.fn().mockResolvedValue({ running: 0, total: 0 });
    const outcome = await podCount("empty-cluster", invoke);
    expect(outcome.error).toBeUndefined();
    expect(outcome.counts).toEqual({ running: 0, total: 0 });
  });

  it("reports a timeout as an error, never as a zero count", async () => {
    const outcome = await podCount("prod-eu", () => Promise.reject(new Error("pod count timed out")));
    expect(outcome.counts).toBeUndefined();
    expect(outcome.error).toContain("timed out");
  });

  it("normalises any transport failure into an error outcome", async () => {
    const outcome = await podCount("prod-eu", () => Promise.reject(new Error("ipc unavailable")));
    expect(outcome.counts).toBeUndefined();
    expect(outcome.error).toContain("ipc unavailable");
  });
});
