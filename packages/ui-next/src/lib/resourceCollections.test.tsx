import { act, renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SELECTS_NOTHING, settingsStorage, type ResourceResolution } from "@srelens/core";

const resolveResourceCollections = vi.hoisted(() => vi.fn());
vi.mock("@srelens/core", async (orig) => ({ ...(await orig<typeof import("@srelens/core")>()), resolveResourceCollections }));

import { COLLECTIONS_KEY, __resetCollectionsForTests, getCollections, saveCollection } from "./collections";
import { RESOLVE_AGAIN_MS, useResourceCollectionFilter } from "./resourceCollections";
import { makeTab } from "./tabs";
import { TabScope } from "./tabScope";
import { getState, setState } from "./tabsStore";

const DEPLOYMENT = { kind: "Deployment" };
const ROWS = [
  { name: "payment-service", namespace: "dev" },
  { name: "order-service", namespace: "dev" },
  { name: "payment-service", namespace: "prod" },
];
const answer = (keys: string[]): ResourceResolution => ({
  keys,
  collections: [{ name: "release", matched: keys.length, members: [] }],
  partial: false,
});

const inTab = ({ children }: { children: ReactNode }) => <TabScope.Provider value="t">{children}</TabScope.Provider>;
const view = () => getState().workspaces[0].tabs[0].view;
const settle = (ms = 500) => act(() => vi.advanceTimersByTimeAsync(ms));

function filterFor(rows = ROWS, namespaces: string[] = []) {
  return renderHook(
    ({ rows: r, ns }) => useResourceCollectionFilter("kind-dev", DEPLOYMENT, ns, r),
    { wrapper: inTab, initialProps: { rows, ns: namespaces } },
  );
}

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  settingsStorage.removeItem(COLLECTIONS_KEY);
  __resetCollectionsForTests();
  resolveResourceCollections.mockReset().mockResolvedValue(answer(["dev/payment-service", "prod/payment-service"]));
  setState({
    workspaces: [{ id: "w", name: "W", clusters: ["c"], activeCluster: "c", tabs: [{ ...makeTab("/k/deployments"), id: "t" }], activeId: "t", closed: [] }],
    currentId: "w",
  });
  saveCollection({ name: "release", type: "resource", kinds: ["apps/Deployment"], members: ["payment-service"] });
  saveCollection({ name: "svc", type: "resource", kinds: ["Service"], patterns: ["*"] });
  return () => vi.useRealTimers();
});

describe("useResourceCollectionFilter", () => {
  it("offers the resource collections that cover the kind, and leaves the rows alone until one is chosen", () => {
    const { result } = filterFor();
    expect(result.current.offered.map((c) => c.name)).toEqual(["release"]);
    expect(result.current.state.status).toBe("off");
    expect(result.current.apply(ROWS)).toBe(ROWS);
    expect(resolveResourceCollections).not.toHaveBeenCalled();
  });

  it("keeps the choice on the tab, records the use, and narrows the rows to what the backend matched", async () => {
    const { result } = filterFor(ROWS, ["dev", "prod"]);
    act(() => result.current.choose(["release"]));
    expect(view()?.collections).toEqual(["release"]);
    expect(getCollections().find((c) => c.name === "release")?.lastUsedAt).toEqual(expect.any(Number));
    // Not every row while it is worked out.
    expect(result.current.state.status).toBe("resolving");
    expect(result.current.apply(ROWS)).toEqual([]);
    await settle();
    expect(resolveResourceCollections).toHaveBeenCalledExactlyOnceWith(
      "kind-dev",
      DEPLOYMENT,
      ["dev", "prod"],
      [expect.objectContaining({ name: "release" })],
    );
    expect(result.current.apply(ROWS).map((r) => `${r.namespace}/${r.name}`)).toEqual(["dev/payment-service", "prod/payment-service"]);
  });

  /** An object created or deleted may now match a pattern; the answer stays on screen meanwhile. */
  it("works it out again when the rows change, without hiding the list while it does", async () => {
    const { result, rerender } = filterFor();
    act(() => result.current.choose(["release"]));
    await settle();
    rerender({ rows: [...ROWS, { name: "payment-service", namespace: "qa" }], ns: [] });
    expect(result.current.state.status).toBe("ready");
    resolveResourceCollections.mockResolvedValue(answer(["dev/payment-service", "prod/payment-service", "qa/payment-service"]));
    await settle();
    expect(resolveResourceCollections).toHaveBeenCalledTimes(2);
    expect(result.current.apply([...ROWS, { name: "payment-service", namespace: "qa" }])).toHaveLength(3);
  });

  it("works it out again on a timer, and not at all for rows that only changed in place", async () => {
    const { result, rerender } = filterFor();
    act(() => result.current.choose(["release"]));
    await settle();
    rerender({ rows: ROWS.map((r) => ({ ...r })), ns: [] });
    await settle();
    expect(resolveResourceCollections).toHaveBeenCalledTimes(1);
    // The timer; then, once React has run the effect it set off, the
    // settling delay that effect starts.
    await settle(RESOLVE_AGAIN_MS);
    await settle();
    expect(resolveResourceCollections).toHaveBeenCalledTimes(2);
  });

  it("matches nothing, and asks nothing, inside a namespace scope that selects nothing", async () => {
    const { result } = filterFor(ROWS, [...SELECTS_NOTHING]);
    act(() => result.current.choose(["release"]));
    await settle();
    expect(resolveResourceCollections).not.toHaveBeenCalled();
    expect(result.current.apply(ROWS)).toEqual([]);
  });

  it("says so when it could not be worked out, and lists nothing", async () => {
    resolveResourceCollections.mockRejectedValue(new Error("forbidden"));
    const { result } = filterFor();
    act(() => result.current.choose(["release"]));
    await settle();
    expect(result.current.state).toEqual({ status: "failed", error: "forbidden" });
    expect(result.current.apply(ROWS)).toEqual([]);
  });

  it("shows every row again once the chosen collection is deleted, or none is chosen", async () => {
    const { result } = filterFor();
    act(() => result.current.choose(["release"]));
    await settle();
    act(() => result.current.choose([]));
    expect(result.current.state.status).toBe("off");
    expect(result.current.apply(ROWS)).toBe(ROWS);
  });
});
