import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import React from "react";

const mocks = vi.hoisted(() => ({
  listNamespaces: vi.fn(),
  listPods: vi.fn(),
  listNodes: vi.fn(),
  listResource: vi.fn(),
  listEvents: vi.fn(),
  loadPersistedOverview: vi.fn(),
  persistOverview: vi.fn(),
  clearPersistedOverview: vi.fn(),
}));

vi.mock("@srelens/core/lib/workloads", () => ({
  listNamespaces: mocks.listNamespaces,
  listPods: mocks.listPods,
}));

vi.mock("@srelens/core/lib/manifest", () => ({
  listNodes: mocks.listNodes,
  listResource: mocks.listResource,
  listEvents: mocks.listEvents,
}));

vi.mock("@srelens/core/lib/overviewSnapshot", () => ({
  loadPersistedOverview: mocks.loadPersistedOverview,
  persistOverview: mocks.persistOverview,
  clearPersistedOverview: mocks.clearPersistedOverview,
}));

import { ClusterOverview, clearClusterOverviewCache } from "./ClusterOverview";
import type { OverviewSnapshot } from "@srelens/core";

beforeEach(() => {
  clearClusterOverviewCache();
  Object.values(mocks).forEach((mock) => mock.mockReset());
  mocks.listNodes.mockResolvedValue({ nodes: [{ status: "Ready" }] });
  mocks.listPods.mockResolvedValue({ pods: [{ phase: "Running" }] });
  mocks.listResource.mockResolvedValue({ items: [{ name: "one" }] });
  mocks.listNamespaces.mockResolvedValue({ namespaces: ["default"] });
  mocks.listEvents.mockResolvedValue({ events: [] });
  mocks.loadPersistedOverview.mockResolvedValue(null);
  mocks.persistOverview.mockResolvedValue(undefined);
  mocks.clearPersistedOverview.mockResolvedValue(undefined);
});

describe("ClusterOverview cache", () => {
  it("reuses a fresh per-context snapshot after the page remounts", async () => {
    const first = render(<ClusterOverview context="kind-dev" />);
    expect(await screen.findAllByText("1 / 1")).toHaveLength(2);
    first.unmount();

    render(<ClusterOverview context="kind-dev" />);
    expect(screen.getAllByText("1 / 1")).toHaveLength(2);
    expect(mocks.listNodes).toHaveBeenCalledTimes(1);
    expect(mocks.listPods).toHaveBeenCalledTimes(1);
    expect(mocks.listResource).toHaveBeenCalledTimes(2);
  });

  it("forces a refresh while keeping the cached dashboard visible", async () => {
    render(<ClusterOverview context="kind-dev" />);
    expect(await screen.findAllByText("1 / 1")).toHaveLength(2);

    fireEvent.click(screen.getByRole("button", { name: "Refresh cluster overview" }));
    expect(screen.getAllByText("1 / 1")).toHaveLength(2);
    await waitFor(() => expect(mocks.listNodes).toHaveBeenCalledTimes(2));
  });
});

function persistedSnapshot(updatedAt: number): OverviewSnapshot {
  return {
    stats: {
      nodes: { total: 4, ready: 3 },
      pods: { total: 6, running: 5, pending: 1, other: 0 },
      deployments: 2,
      services: 3,
      namespaces: 4,
      events: { total: 0, normal: 0, warnings: 0, recentWarnings: [] },
    },
    updatedAt,
  };
}

/** A promise the test resolves by hand, to order fetch vs hydrate. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

describe("ClusterOverview persistence", () => {
  it("hydrates the persisted snapshot while the first fetch is pending", async () => {
    mocks.loadPersistedOverview.mockResolvedValue(persistedSnapshot(Date.now() - 3 * 86_400_000));
    // Never-resolving fetches: only the persisted snapshot can paint the tiles.
    mocks.listNodes.mockReturnValue(new Promise(() => {}));

    render(<ClusterOverview context="kind-cold" />);

    expect(await screen.findByText("3 / 4")).toBeDefined();
    expect(screen.getByText("5 / 6")).toBeDefined();
    expect(mocks.loadPersistedOverview).toHaveBeenCalledWith("kind-cold");
  });

  it("shows the snapshot's date while hydrated data is days old", async () => {
    mocks.loadPersistedOverview.mockResolvedValue(persistedSnapshot(Date.now() - 3 * 86_400_000));
    mocks.listNodes.mockReturnValue(new Promise(() => {}));

    render(<ClusterOverview context="kind-cold" />);

    // A days-old timestamp must not masquerade as today's: the header shows its date.
    expect(await screen.findByText(/updated .*\b20\d\d\b/)).toBeDefined();
  });

  it("replaces the hydrated snapshot when fresh data arrives", async () => {
    mocks.loadPersistedOverview.mockResolvedValue(persistedSnapshot(Date.now() - 3 * 86_400_000));

    render(<ClusterOverview context="kind-cold" />);

    expect(await screen.findAllByText("1 / 1")).toHaveLength(2);
    expect(screen.queryByText("3 / 4")).toBeNull();
  });

  it("never lets a slow hydrate clobber already-fresh data", async () => {
    const slowLoad = deferred<OverviewSnapshot>();
    mocks.loadPersistedOverview.mockReturnValue(slowLoad.promise);

    render(<ClusterOverview context="kind-race" />);
    expect(await screen.findAllByText("1 / 1")).toHaveLength(2);

    slowLoad.resolve(persistedSnapshot(Date.now() - 3 * 86_400_000));
    await waitFor(() => expect(screen.queryByText("3 / 4")).toBeNull());
    expect(screen.getAllByText("1 / 1")).toHaveLength(2);
  });

  it("remounting during the first fetch still applies the fresh result", async () => {
    // A recent (within-TTL) persisted snapshot must not let a remount skip
    // joining the pending live request — the fresh data has to reach the
    // screen, not just the module cache.
    mocks.loadPersistedOverview.mockResolvedValue(persistedSnapshot(Date.now() - 1000));
    const slowNodes = deferred<{ nodes: Array<{ status: string }> }>();
    mocks.listNodes.mockReturnValue(slowNodes.promise);

    const first = render(<ClusterOverview context="kind-remount" />);
    expect(await screen.findByText("3 / 4")).toBeDefined();
    first.unmount();

    render(<ClusterOverview context="kind-remount" />);
    slowNodes.resolve({ nodes: [{ status: "Ready" }] });

    expect(await screen.findAllByText("1 / 1")).toHaveLength(2);
  });

  it("persists a successful fetch for the next cold start", async () => {
    render(<ClusterOverview context="kind-dev" />);
    expect(await screen.findAllByText("1 / 1")).toHaveLength(2);

    await waitFor(() => expect(mocks.persistOverview).toHaveBeenCalledTimes(1));
    const [context, snapshot] = mocks.persistOverview.mock.calls[0];
    expect(context).toBe("kind-dev");
    expect(snapshot.stats.nodes).toEqual({ total: 1, ready: 1 });
  });

  it("a clear during an in-flight fetch stops it from repopulating the caches", async () => {
    // The documented logout/reset flow: nothing fetched before the clear may
    // survive it — the resolving request must not re-cache or re-persist.
    const slowNodes = deferred<{ nodes: Array<{ status: string }> }>();
    mocks.listNodes.mockReturnValue(slowNodes.promise);

    render(<ClusterOverview context="kind-clear" />);
    clearClusterOverviewCache();
    slowNodes.resolve({ nodes: [{ status: "Ready" }] });

    expect(await screen.findAllByText("1 / 1")).toHaveLength(2);
    expect(mocks.persistOverview).not.toHaveBeenCalled();
  });

  it("clearClusterOverviewCache also clears the persisted copy", () => {
    clearClusterOverviewCache("kind-a");
    expect(mocks.clearPersistedOverview).toHaveBeenCalledWith("kind-a");

    clearClusterOverviewCache();
    expect(mocks.clearPersistedOverview).toHaveBeenCalledWith(undefined);
  });
});

describe("ClusterOverview error handling", () => {
  it("renders a friendly connectivity message on a connection timeout", async () => {
    mocks.listNamespaces.mockResolvedValue({ error: "handler error: list namespaces timed out" });

    render(<ClusterOverview context="kind-unreachable" />);

    // Friendly title, not the raw backend string.
    expect(await screen.findByText("Request timed out")).toBeDefined();
    expect(screen.getByText(/didn't respond in time/)).toBeDefined();
    expect(screen.queryByText(/handler error/)).toBeNull();
  });

  it("retries the load when the user clicks Retry", async () => {
    mocks.listNamespaces.mockResolvedValueOnce({
      error: "handler error: list namespaces timed out",
    });

    render(<ClusterOverview context="kind-flaky" />);
    fireEvent.click(await screen.findByRole("button", { name: "Retry" }));

    // Second attempt succeeds with the default healthy mocks.
    expect(await screen.findAllByText("1 / 1")).toHaveLength(2);
  });
});
