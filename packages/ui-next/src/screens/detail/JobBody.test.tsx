import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import type { K8sObject, PodMetric, PodSummary } from "@srelens/core";

// The composition test below renders `GenericBody`, whose related-pods
// section calls `podsForSelector`/`podMetrics` for a Job (its
// `relatedPodSelector` reads `spec.selector.matchLabels`, non-empty here) —
// mocked so the test controls what "the cluster said" without one.
// `importOriginal` keeps every formatter (`durationBetween`, `str`, ...) intact.
const { podsForSelector, podMetrics } = vi.hoisted(() => ({
  podsForSelector: vi.fn(async (): Promise<{ pods?: PodSummary[]; error?: string }> => ({ pods: [] })),
  podMetrics: vi.fn(async (): Promise<{ metrics?: PodMetric[]; error?: string }> => ({ metrics: [] })),
}));

vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  podsForSelector,
  podMetrics,
}));

import { GenericBody } from "./GenericBody";
import { JobDetailsBody } from "./JobBody";

function job(
  spec: Record<string, unknown>,
  status: Record<string, unknown> = {},
  metadata: NonNullable<K8sObject["metadata"]> = { name: "backup", namespace: "default" },
): K8sObject {
  return { kind: "Job", apiVersion: "batch/v1", metadata, spec, status } as K8sObject;
}

describe("JobDetailsBody", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    podsForSelector.mockResolvedValue({ pods: [] });
    podMetrics.mockResolvedValue({ metrics: [] });
  });

  describe("Job", () => {
    it("shows completions and parallelism", () => {
      render(<JobDetailsBody object={job({ completions: 3, parallelism: 2 })} />);
      expect(screen.getByText("3")).toBeDefined();
      expect(screen.getByText("2")).toBeDefined();
    });

    it("shows succeeded, failed and active counts", () => {
      render(<JobDetailsBody object={job({}, { succeeded: 2, failed: 1, active: 1 })} />);
      expect(screen.getByText("2")).toBeDefined();
      expect(screen.getAllByText("1").length).toBeGreaterThanOrEqual(2);
    });

    it("defaults succeeded, failed and active to 0 when absent", () => {
      render(<JobDetailsBody object={job({})} />);
      expect(screen.getAllByText("0")).toHaveLength(3);
    });

    it("shows a completed Job's duration between start and completion", () => {
      render(
        <JobDetailsBody
          object={job(
            {},
            { startTime: "2026-08-20T00:00:00Z", completionTime: "2026-08-20T00:05:30Z" },
          )}
        />,
      );
      expect(screen.getByText("5m 30s")).toBeDefined();
    });

    it("shows a still-running Job's elapsed time suffixed (running), not a duration", () => {
      const now = new Date("2026-08-20T00:10:00Z").getTime();
      vi.useFakeTimers();
      vi.setSystemTime(now);
      render(<JobDetailsBody object={job({}, { startTime: "2026-08-20T00:00:00Z" })} />);
      expect(screen.getByText("10m (running)")).toBeDefined();
      expect(screen.queryByText(/^5m 30s$/)).toBeNull();
      vi.useRealTimers();
    });

    it("shows a dash for Started/Completed/Duration on a Job that hasn't started", () => {
      render(<JobDetailsBody object={job({})} />);
      expect(screen.getAllByText("—").length).toBeGreaterThanOrEqual(3);
    });
  });

  it("is a flat block, not a card", () => {
    const { container } = render(<JobDetailsBody object={job({})} />);
    expect(container.querySelector("section.section")).not.toBeNull();
    expect(container.querySelector(".card")).toBeNull();
  });

  describe("composition with GenericBody", () => {
    it("renders exactly one Pods section for a Job reached through GenericBody", async () => {
      const j = job({ selector: { matchLabels: { "job-name": "backup" } } });
      render(
        <GenericBody kind="Job" object={j} context="ctx">
          <JobDetailsBody object={j} />
        </GenericBody>,
      );
      await waitFor(() => expect(screen.getAllByRole("heading", { name: "Pods" })).toHaveLength(1));
    });
  });
});
