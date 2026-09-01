import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import type { ClusterContext, PodSummary } from "@srelens/core";

const { podsOnNode } = vi.hoisted(() => ({
  podsOnNode: vi.fn(),
}));

vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  podsOnNode,
}));

import { defaultState } from "../../lib/tabs";
import * as store from "../../lib/tabsStore";
import { NodePodsSection } from "./sections";

const NOW = Date.parse("2026-09-01T12:00:00Z");
const CTX: ClusterContext = {
  name: "prod-eu",
  stableId: "prod",
  cluster: "prod",
  server: "https://prod",
  isCurrent: true,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
};

function pod(i: number): PodSummary {
  return {
    name: `pod-${String(i).padStart(2, "0")}`,
    namespace: i % 2 === 0 ? "default" : "kube-system",
    phase: "Running",
    ready: "1/1",
    restarts: 0,
    node: "worker-2",
    age: "stale",
    createdAt: new Date(NOW - 15_000).toISOString(),
    image: "example.test/app:1",
  };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
  podsOnNode.mockReset();
  podsOnNode.mockResolvedValue({ pods: Array.from({ length: 14 }, (_, i) => pod(i)) });
  store.setState(defaultState([CTX]));
});

afterEach(() => vi.useRealTimers());

describe("NodePodsSection", () => {
  it("lists a capped, stable slice with the true total and live ages", async () => {
    render(<NodePodsSection context="prod-eu" node="worker-2" />);
    await act(async () => Promise.resolve());

    expect(podsOnNode).toHaveBeenCalledWith("prod-eu", "worker-2");
    expect(screen.getByRole("heading", { level: 3, name: "Pods (14)" })).toBeDefined();
    expect(screen.getAllByRole("row")).toHaveLength(13);
    expect(screen.getByRole("button", { name: "Sort by Pod" })).toBeDefined();
    expect(screen.getByRole("button", { name: "Sort by Namespace" })).toBeDefined();
    expect(screen.getByRole("button", { name: "Sort by Age" })).toBeDefined();
    expect(screen.getAllByText("15s")).toHaveLength(12);

    act(() => vi.advanceTimersByTime(30_000));
    expect(screen.getAllByText("45s")).toHaveLength(12);
  });

  it("opens a pod detail from a row and the node-filtered Pods list from View all", async () => {
    render(<NodePodsSection context="prod-eu" node="worker-2" />);
    await act(async () => Promise.resolve());

    fireEvent.click(screen.getByText("pod-00"));
    expect(store.activeRoute()).toBe("/k/Pod/default/pod-00");

    fireEvent.click(screen.getByRole("button", { name: "View all 14 pods on worker-2" }));
    expect(store.activeRoute()).toBe("/k/pods");
    const tab = store.currentWorkspace().tabs.find((candidate) => candidate.route === "/k/pods");
    expect(tab?.view).toMatchObject({ filter: "worker-2", filterKey: "node" });
  });

  it("keeps loading, refusal and an empty node distinct", async () => {
    podsOnNode.mockImplementation(() => new Promise(() => {}));
    const view = render(<NodePodsSection context="prod-eu" node="worker-2" />);
    expect(screen.getByText("Loading pods on worker-2")).toBeDefined();

    podsOnNode.mockResolvedValue({ error: "forbidden" });
    await act(async () => {
      view.rerender(<NodePodsSection context="prod-eu" node="worker-3" />);
      await Promise.resolve();
    });
    expect(screen.getByText(/doesn't have permission/)).toBeDefined();
    expect(screen.getByText("forbidden")).toBeDefined();

    podsOnNode.mockResolvedValue({ pods: [] });
    await act(async () => {
      view.rerender(<NodePodsSection context="prod-eu" node="worker-4" />);
      await Promise.resolve();
    });
    expect(screen.getByText("No pods on this node")).toBeDefined();
  });
});
