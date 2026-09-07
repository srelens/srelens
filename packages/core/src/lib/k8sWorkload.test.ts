import { describe, it, expect } from "vitest";
import { summarizeAffinity, updateStrategy, relatedPodSelector } from "./k8sWorkload";
import type { K8sObject } from "./manifest";

// Moved verbatim from apps/desktop/src/components/ResourceOverview.test.tsx
// (only the import path changed).
describe("summarizeAffinity", () => {
  it("summarizes required and preferred rules per affinity type", () => {
    const affinity = {
      nodeAffinity: {
        requiredDuringSchedulingIgnoredDuringExecution: { nodeSelectorTerms: [{}, {}] },
        preferredDuringSchedulingIgnoredDuringExecution: [{}],
      },
      podAntiAffinity: {
        requiredDuringSchedulingIgnoredDuringExecution: [{}],
      },
    };
    expect(summarizeAffinity(affinity)).toEqual([
      "Node affinity: 2 required, 1 preferred",
      "Pod anti-affinity: 1 required",
    ]);
  });

  it("returns an empty list when there is no affinity", () => {
    expect(summarizeAffinity({})).toEqual([]);
  });
});

// Facts, not a sentence: the two designs word these differently and each
// formats at its own edge (classic's `updateStrategyText` in
// `ResourceOverview.tsx`, the new design's in `WorkloadBody.tsx`). What is
// shared, and tested here, is WHICH fields are read and how they are read.
describe("updateStrategy", () => {
  it("defaults the type to RollingUpdate, as the API server does", () => {
    expect(updateStrategy({})).toEqual({
      type: "RollingUpdate",
      partition: undefined,
      maxSurge: undefined,
      maxUnavailable: undefined,
    });
  });

  it("uses an explicit type when given, rather than always defaulting", () => {
    expect(updateStrategy({ type: "OnDelete" }).type).toBe("OnDelete");
  });

  it("reads partition, surge and unavailable out of rollingUpdate", () => {
    expect(updateStrategy({ type: "RollingUpdate", rollingUpdate: { partition: 1, maxUnavailable: 1, maxSurge: 1 } })).toEqual(
      { type: "RollingUpdate", partition: "1", maxSurge: "1", maxUnavailable: "1" },
    );
  });

  it("keeps every value as written, so a percentage survives", () => {
    // A surge or an unavailable may legally be `"25%"`; parsing it into a
    // number would have to invent a base to resolve it against.
    expect(updateStrategy({ rollingUpdate: { maxSurge: "25%" } }).maxSurge).toBe("25%");
    expect(updateStrategy({ rollingUpdate: { maxSurge: 2 } }).maxSurge).toBe("2");
  });

  it("keeps an explicit zero, which is the strictest setting there is", () => {
    // `maxUnavailable: 0` means "take nothing down while rolling". A
    // truthiness test would drop it and leave the row looking unset.
    expect(updateStrategy({ rollingUpdate: { maxUnavailable: 0 } }).maxUnavailable).toBe("0");
  });

  it("reports an unset field as undefined rather than as an empty string", () => {
    // The distinction each design's formatter turns into "print this clause
    // or leave it out"; an empty string would print a labelled blank.
    const out = updateStrategy({ rollingUpdate: { maxSurge: "25%" } });
    expect(out.partition).toBeUndefined();
    expect(out.maxUnavailable).toBeUndefined();
  });

  it("survives a strategy with no rollingUpdate block at all", () => {
    expect(updateStrategy({ type: "OnDelete" })).toEqual({
      type: "OnDelete",
      partition: undefined,
      maxSurge: undefined,
      maxUnavailable: undefined,
    });
  });
});

// classic's ResourceOverview.test.tsx did not cover relatedPodSelector either;
// written here against its actual per-kind branches.
describe("relatedPodSelector", () => {
  it("reads a Service's selector directly, with no matchLabels indirection", () => {
    const svc = { spec: { selector: { app: "web" } } } as K8sObject;
    expect(relatedPodSelector("Service", svc)).toEqual({ app: "web" });
  });

  it("reads a DaemonSet's selector through matchLabels, unlike Service", () => {
    const ds = { spec: { selector: { matchLabels: { app: "logging" } } } } as K8sObject;
    expect(relatedPodSelector("DaemonSet", ds)).toEqual({ app: "logging" });
    // Proves the matchLabels indirection is actually used (not the bare
    // selector, as Service uses): a selector with no matchLabels field
    // yields nothing.
    const bare = { spec: { selector: { app: "logging" } } } as K8sObject;
    expect(relatedPodSelector("DaemonSet", bare)).toEqual({});
  });

  it("reads a Job's selector through matchLabels, same as DaemonSet", () => {
    const job = { spec: { selector: { matchLabels: { "job-name": "backup" } } } } as K8sObject;
    expect(relatedPodSelector("Job", job)).toEqual({ "job-name": "backup" });
  });

  it("reads a PodDisruptionBudget's selector through matchLabels, as its own switch case", () => {
    const pdb = { spec: { selector: { matchLabels: { app: "pdb" } } } } as K8sObject;
    expect(relatedPodSelector("PodDisruptionBudget", pdb)).toEqual({ app: "pdb" });
  });

  it("reads a NetworkPolicy's podSelector, not its selector", () => {
    const np = {
      spec: {
        selector: { matchLabels: { app: "wrong" } },
        podSelector: { matchLabels: { app: "netpol" } },
      },
    } as K8sObject;
    expect(relatedPodSelector("NetworkPolicy", np)).toEqual({ app: "netpol" });
  });

  it("returns an empty object for a kind with no related-pod selector", () => {
    const deploy = { spec: { selector: { matchLabels: { app: "deploy" } } } } as K8sObject;
    expect(relatedPodSelector("Deployment", deploy)).toEqual({});
  });
});
