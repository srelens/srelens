import { describe, it, expect } from "vitest";
import { nodeUsage, clusterCapacity, podUsage } from "./k8sCapacity";
import type { NodeSummary, NodeMetric } from "./manifest";

function node(overrides: Partial<NodeSummary> = {}): NodeSummary {
  return {
    name: "node-1",
    status: "Ready",
    unschedulable: false,
    taints: 0,
    taintDetails: [],
    version: "v1.30.0",
    roles: "worker",
    age: "10d",
    allocatableCpuMillicores: 4000,
    allocatableMemoryMiB: 16000,
    allocatablePods: 110,
    instanceType: "c3-standard-4",
    ...overrides,
  };
}

function metric(overrides: Partial<NodeMetric> = {}): NodeMetric {
  return {
    name: "node-1",
    cpuMillicores: 2000,
    memoryMiB: 8000,
    ...overrides,
  };
}

describe("nodeUsage — the ordinary case", () => {
  it("divides usage by allocatable, as a percentage, unrounded", () => {
    const usage = nodeUsage(node(), metric(), 31);
    expect(usage.cpuPercent).toBe(50);
    expect(usage.memoryPercent).toBe(50);
    expect(usage.pods).toEqual({ used: 31, allocatable: 110 });
  });

  it("does not round — a third of allocatable stays a real fraction", () => {
    const usage = nodeUsage(
      node({ allocatableCpuMillicores: 3000 }),
      metric({ cpuMillicores: 1000 }),
      undefined,
    );
    expect(usage.cpuPercent).toBeCloseTo(33.333333, 5);
  });
});

describe("nodeUsage — no metric for the node", () => {
  it("is null, not zero — absence of a reading is not an idle reading", () => {
    const usage = nodeUsage(node(), undefined, undefined);
    expect(usage.cpuPercent).toBeNull();
    expect(usage.memoryPercent).toBeNull();
  });
});

describe("nodeUsage — allocatable of zero", () => {
  it("is null, not a division by zero", () => {
    const usage = nodeUsage(
      node({ allocatableCpuMillicores: 0, allocatableMemoryMiB: 0 }),
      metric(),
      undefined,
    );
    expect(usage.cpuPercent).toBeNull();
    expect(usage.memoryPercent).toBeNull();
  });
});

describe("nodeUsage — usage above allocatable", () => {
  it("reports honestly above 100, never clamped", () => {
    const usage = nodeUsage(
      node({ allocatableCpuMillicores: 1000, allocatableMemoryMiB: 1000 }),
      metric({ cpuMillicores: 1400, memoryMiB: 1900 }),
      undefined,
    );
    expect(usage.cpuPercent).toBe(140);
    expect(usage.memoryPercent).toBe(190);
  });
});

describe("nodeUsage — pods", () => {
  it("is null when the pod count for the node is unknown", () => {
    const usage = nodeUsage(node(), metric(), undefined);
    expect(usage.pods).toBeNull();
  });

  it("carries the raw used/allocatable pair when a count is known", () => {
    const usage = nodeUsage(node({ allocatablePods: 50 }), metric(), 31);
    expect(usage.pods).toEqual({ used: 31, allocatable: 50 });
  });
});

describe("clusterCapacity — the ordinary case", () => {
  it("sums usage and allocatable across every node reporting a metric, and says all of them reported", () => {
    const nodes = [
      node({ name: "a", allocatableCpuMillicores: 4000, allocatableMemoryMiB: 16000 }),
      node({ name: "b", allocatableCpuMillicores: 4000, allocatableMemoryMiB: 16000 }),
    ];
    const metrics = [
      metric({ name: "a", cpuMillicores: 1000, memoryMiB: 4000 }),
      metric({ name: "b", cpuMillicores: 3000, memoryMiB: 12000 }),
    ];
    const capacity = clusterCapacity(nodes, metrics);
    expect(capacity.cpu).toEqual({ usedMillicores: 4000, allocatableMillicores: 8000 });
    expect(capacity.memory).toEqual({ usedMiB: 16000, allocatableMiB: 32000 });
    expect(capacity.nodesReporting).toBe(2);
    expect(capacity.nodesTotal).toBe(2);
  });
});

describe("clusterCapacity — no node has a metric", () => {
  it("is null — the same absence rule as a single node, not a zero total — but still says how many nodes there are", () => {
    const nodes = [node({ name: "a" }), node({ name: "b" })];
    const capacity = clusterCapacity(nodes, []);
    expect(capacity.cpu).toBeNull();
    expect(capacity.memory).toBeNull();
    expect(capacity.nodesReporting).toBe(0);
    expect(capacity.nodesTotal).toBe(2);
  });
});

describe("clusterCapacity — an empty cluster", () => {
  it("is null, with zero of zero nodes reporting", () => {
    const capacity = clusterCapacity([], []);
    expect(capacity.cpu).toBeNull();
    expect(capacity.memory).toBeNull();
    expect(capacity.nodesReporting).toBe(0);
    expect(capacity.nodesTotal).toBe(0);
  });
});

describe("clusterCapacity — some nodes have metrics, others do not", () => {
  it("pins the qualified partial: two of three nodes report, the sum covers only those two, and the counts say '2 of 3' right on the return value", () => {
    const nodes = [
      node({ name: "a", allocatableCpuMillicores: 4000, allocatableMemoryMiB: 16000 }),
      node({ name: "b", allocatableCpuMillicores: 2000, allocatableMemoryMiB: 8000 }),
      // "c" has no metric — joined since the last scrape, say.
      node({ name: "c", allocatableCpuMillicores: 6000, allocatableMemoryMiB: 24000 }),
    ];
    const metrics = [
      metric({ name: "a", cpuMillicores: 1000, memoryMiB: 4000 }),
      metric({ name: "b", cpuMillicores: 500, memoryMiB: 2000 }),
    ];
    const capacity = clusterCapacity(nodes, metrics);
    // Node "c"'s 6000m/24000MiB of capacity is excluded entirely, not folded
    // in as an allocatable with zero usage — that would understate the
    // percentage by inventing a reading nobody took.
    expect(capacity.cpu).toEqual({ usedMillicores: 1500, allocatableMillicores: 6000 });
    expect(capacity.memory).toEqual({ usedMiB: 6000, allocatableMiB: 24000 });
    // The qualifier travels with the number: a consumer cannot show the 25%
    // CPU figure above without "2 of 3" being right there to show beside it.
    expect(capacity.nodesReporting).toBe(2);
    expect(capacity.nodesTotal).toBe(3);
  });
});

describe("clusterCapacity — a metric names a node that is not in the list", () => {
  it("drops the ghost metric entirely, and nodesReporting never exceeds nodesTotal", () => {
    const nodes = [
      node({ name: "a", allocatableCpuMillicores: 4000, allocatableMemoryMiB: 16000 }),
      node({ name: "b", allocatableCpuMillicores: 4000, allocatableMemoryMiB: 16000 }),
    ];
    const metrics = [
      metric({ name: "a", cpuMillicores: 1000, memoryMiB: 4000 }),
      metric({ name: "b", cpuMillicores: 1000, memoryMiB: 4000 }),
      // "ghost" names no node in the list — metrics-server racing the node
      // list, or holding a stale entry for a node that was just drained and
      // removed. It must contribute to neither sum nor either count.
      metric({ name: "ghost", cpuMillicores: 9999, memoryMiB: 9999 }),
    ];
    const capacity = clusterCapacity(nodes, metrics);
    expect(capacity.cpu).toEqual({ usedMillicores: 2000, allocatableMillicores: 8000 });
    expect(capacity.memory).toEqual({ usedMiB: 8000, allocatableMiB: 32000 });
    // The invariant worth stating outright: however this is computed, the
    // reporting count can never exceed the node count. A loop that iterates
    // metrics instead of nodes can violate this silently — more nodes
    // "reporting" than exist, and a percentage nobody can explain.
    expect(capacity.nodesReporting).toBeLessThanOrEqual(capacity.nodesTotal);
    expect(capacity.nodesReporting).toBe(2);
    expect(capacity.nodesTotal).toBe(2);
  });
});

/**
 * A pod's usage, against what it was given (#864). A node has one number to be
 * a share of; a pod has two, either of which may be missing.
 */
describe("podUsage", () => {
  const reading = { cpuMillicores: 250, memoryMiB: 300 };
  const whole = {
    cpuReqMillicores: 100, cpuLimMillicores: 500, cpuLimAll: true,
    memReqMiB: 128, memLimMiB: 400, memLimAll: true,
  };

  it("measures against the limit when every container sets one, and says so", () => {
    expect(podUsage(whole, reading)).toEqual({
      cpu: { percent: 50, of: "limit", bound: 500 },
      memory: { percent: 75, of: "limit", bound: 400 },
    });
  });

  it("falls back to the request for a resource whose limit is not the pod's whole ceiling", () => {
    // One container has no CPU limit: 500m is not a ceiling the pod has.
    const usage = podUsage({ ...whole, cpuLimAll: false }, reading);
    expect(usage.cpu).toEqual({ percent: 250, of: "request", bound: 100 });
    // Memory is decided on its own.
    expect(usage.memory?.of).toBe("limit");
  });

  it("falls back to the request when there is no limit at all", () => {
    const usage = podUsage({ cpuReqMillicores: 500, memReqMiB: 600 }, reading);
    expect(usage.cpu).toEqual({ percent: 50, of: "request", bound: 500 });
    expect(usage.memory).toEqual({ percent: 50, of: "request", bound: 600 });
  });

  it("reports honestly above 100 against a request, never clamped", () => {
    expect(podUsage({ cpuReqMillicores: 100 }, reading).cpu?.percent).toBe(250);
  });

  it("is null, not zero, with neither a limit nor a request", () => {
    expect(podUsage({}, reading)).toEqual({ cpu: null, memory: null });
    expect(podUsage({ cpuReqMillicores: 0, cpuLimMillicores: 0, cpuLimAll: false }, reading).cpu).toBeNull();
  });

  it("does not take a flag without a limit as a ceiling of zero", () => {
    // `cpuLimAll` true with no figure is not a limit; the request stands.
    expect(podUsage({ cpuLimAll: true, cpuLimMillicores: 0, cpuReqMillicores: 200 }, reading).cpu).toEqual({
      percent: 125, of: "request", bound: 200,
    });
  });

  it("is null for both when the pod has no reading", () => {
    expect(podUsage(whole, undefined)).toEqual({ cpu: null, memory: null });
  });

  it("does not round", () => {
    expect(podUsage({ cpuReqMillicores: 300 }, { cpuMillicores: 100, memoryMiB: 0 }).cpu?.percent).toBeCloseTo(33.3333, 3);
  });

  it("reads an idle pod as 0%, which is a measurement", () => {
    expect(podUsage(whole, { cpuMillicores: 0, memoryMiB: 0 })).toEqual({
      cpu: { percent: 0, of: "limit", bound: 500 },
      memory: { percent: 0, of: "limit", bound: 400 },
    });
  });
});
