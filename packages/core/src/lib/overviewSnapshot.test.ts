import { beforeEach, describe, expect, it, vi } from "vitest";

const platform = vi.hoisted(() => ({ isTauri: vi.fn() }));
vi.mock("../transport/platform", () => ({ isTauri: platform.isTauri }));

import {
  clearPersistedOverview,
  loadPersistedOverview,
  persistOverview,
  type OverviewSnapshot,
} from "./overviewSnapshot";

beforeEach(() => {
  platform.isTauri.mockReturnValue(true);
});

function snapshot(): OverviewSnapshot {
  return {
    stats: {
      nodes: { total: 4, ready: 3 },
      pods: { total: 6, running: 5, pending: 1, other: 0 },
      deployments: 2,
      services: 3,
      namespaces: 4,
      events: { total: 0, normal: 0, warnings: 0, recentWarnings: [] },
    },
    updatedAt: 1_755_000_000_000,
  };
}

describe("loadPersistedOverview", () => {
  it("loads the snapshot via overview_snapshot_load", async () => {
    const invoke = vi.fn().mockResolvedValue(snapshot());

    const loaded = await loadPersistedOverview("kind-dev", invoke);

    expect(invoke).toHaveBeenCalledWith("overview_snapshot_load", { context: "kind-dev" });
    expect(loaded).toEqual(snapshot());
  });

  it("returns null when nothing is persisted", async () => {
    const invoke = vi.fn().mockResolvedValue(null);
    expect(await loadPersistedOverview("kind-dev", invoke)).toBeNull();
  });

  it("returns null when the command is unavailable (web mode)", async () => {
    const invoke = vi.fn().mockRejectedValue(new Error("unknown command: overview_snapshot_load"));
    expect(await loadPersistedOverview("kind-dev", invoke)).toBeNull();
  });

  it("returns null when the payload has the wrong shape", async () => {
    const invoke = vi.fn().mockResolvedValue({ stats: null, updatedAt: "yesterday" });
    expect(await loadPersistedOverview("kind-dev", invoke)).toBeNull();
  });

  it("returns null when the timestamp can't be represented as a Date", async () => {
    // Rust's i64 round-trips values far past JS's ±8.64e15 Date range;
    // formatUpdatedAt would throw a RangeError on them mid-hydration.
    for (const updatedAt of [9_000_000_000_000_000, -9_000_000_000_000_000]) {
      const invoke = vi.fn().mockResolvedValue({ ...snapshot(), updatedAt });
      expect(await loadPersistedOverview("kind-dev", invoke)).toBeNull();
    }
  });

  it("returns null when stats is missing nested fields (older schema)", async () => {
    // Syntactically valid but incomplete payloads must read as cache misses:
    // the overview dereferences stats.nodes.ready etc. straight in render.
    const incomplete = [
      { stats: {}, updatedAt: 123 },
      { stats: { nodes: { total: 1, ready: 1 } }, updatedAt: 123 },
      { ...snapshot(), stats: { ...snapshot().stats, pods: { total: 6 } } },
      { ...snapshot(), stats: { ...snapshot().stats, events: { total: 0 } } },
      { ...snapshot(), stats: { ...snapshot().stats, deployments: "2" } },
      { ...snapshot(), stats: { ...snapshot().stats, events: null } },
    ];
    for (const payload of incomplete) {
      const invoke = vi.fn().mockResolvedValue(payload);
      expect(await loadPersistedOverview("kind-dev", invoke)).toBeNull();
    }
  });
});

describe("persistOverview", () => {
  it("saves the snapshot via overview_snapshot_save", async () => {
    const invoke = vi.fn().mockResolvedValue(null);

    await persistOverview("kind-dev", snapshot(), invoke);

    expect(invoke).toHaveBeenCalledWith("overview_snapshot_save", {
      context: "kind-dev",
      snapshot: snapshot(),
    });
  });

  it("swallows command failures", async () => {
    const invoke = vi.fn().mockRejectedValue(new Error("unknown command"));
    await expect(persistOverview("kind-dev", snapshot(), invoke)).resolves.toBeUndefined();
  });
});

describe("web mode", () => {
  // The commands only exist on desktop; in web mode each call would be a real
  // authenticated HTTP request the server 404s after resolving the user's env.
  it("never sends the commands when not running under Tauri", async () => {
    platform.isTauri.mockReturnValue(false);
    const invoke = vi.fn();

    expect(await loadPersistedOverview("kind-dev", invoke)).toBeNull();
    await persistOverview("kind-dev", snapshot(), invoke);
    await clearPersistedOverview("kind-dev", invoke);

    expect(invoke).not.toHaveBeenCalled();
  });
});

describe("clearPersistedOverview", () => {
  it("clears one context via overview_snapshot_clear", async () => {
    const invoke = vi.fn().mockResolvedValue(null);

    await clearPersistedOverview("kind-dev", invoke);

    expect(invoke).toHaveBeenCalledWith("overview_snapshot_clear", { context: "kind-dev" });
  });

  it("clears every context when called without one", async () => {
    const invoke = vi.fn().mockResolvedValue(null);

    await clearPersistedOverview(undefined, invoke);

    expect(invoke).toHaveBeenCalledWith("overview_snapshot_clear", { context: null });
  });

  it("swallows command failures", async () => {
    const invoke = vi.fn().mockRejectedValue(new Error("unknown command"));
    await expect(clearPersistedOverview("kind-dev", invoke)).resolves.toBeUndefined();
  });
});
