import { act, renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { settingsStorage, type Collection } from "@srelens/core";

const matchCollectionNames = vi.hoisted(() => vi.fn());
vi.mock("@srelens/core", async (orig) => ({ ...(await orig<typeof import("@srelens/core")>()), matchCollectionNames }));

import { COLLECTIONS_KEY, __resetCollectionsForTests, saveCollection } from "./collections";
import { __resetCollectionScopeForTests } from "./collectionScope";
import { useCollectionNameFilter } from "./collectionNameFilter";
import { makeTab } from "./tabs";
import { TabScope } from "./tabScope";
import { setState, setTabNamespaceCollections } from "./tabsStore";

const ROWS = [
  { name: "payment-service", namespace: "prod-eu" },
  { name: "order-service", namespace: "prod-eu" },
  { name: "payment-service", namespace: "prod-us" },
];
const PROD: Collection = {
  name: "prod",
  type: "namespace",
  patterns: ["prod-*"],
  resources: { kinds: ["apps/Deployment"], nameRegex: "^payment-" },
};
const inTab = ({ children }: { children: ReactNode }) => <TabScope.Provider value="t">{children}</TabScope.Provider>;
const settle = () => act(() => vi.advanceTimersByTimeAsync(300));

function filterFor(kind = { kind: "Deployment" }, rows = ROWS) {
  return renderHook(({ k, r }) => useCollectionNameFilter("c", k, r), { wrapper: inTab, initialProps: { k: kind, r: rows } });
}

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  settingsStorage.removeItem(COLLECTIONS_KEY);
  __resetCollectionsForTests();
  __resetCollectionScopeForTests();
  matchCollectionNames.mockReset().mockImplementation(async (_c, _k, names: string[]) => ({
    filtered: true,
    matches: names.map((n) => n.startsWith("payment-")),
    problems: [],
  }));
  setState({
    workspaces: [{ id: "w", name: "W", clusters: ["c"], activeCluster: "c", tabs: [{ ...makeTab("/k/deployments"), id: "t" }], activeId: "t", closed: [] }],
    currentId: "w",
  });
  saveCollection(PROD);
  saveCollection({ name: "plain", type: "namespace", patterns: ["prod-*"] });
  return () => vi.useRealTimers();
});

const scopeTo = (...names: string[]) => act(() => setTabNamespaceCollections("t", "c", names));

describe("useCollectionNameFilter", () => {
  it("leaves the list alone while the tab is not scoped to a collection", () => {
    const { result } = filterFor();
    expect(result.current.state.status).toBe("off");
    expect(result.current.apply(ROWS)).toBe(ROWS);
  });

  it("narrows a kind the collection filters to the names its expression matches, nothing while it works it out", async () => {
    const { result } = filterFor();
    scopeTo("prod");
    expect(result.current.state.status).toBe("matching");
    expect(result.current.apply(ROWS)).toEqual([]);
    await settle();
    expect(matchCollectionNames).toHaveBeenCalledExactlyOnceWith([expect.objectContaining({ name: "prod" })], { kind: "Deployment" }, ["order-service", "payment-service"]);
    expect(result.current.apply(ROWS).map((r) => `${r.namespace}/${r.name}`)).toEqual(["prod-eu/payment-service", "prod-us/payment-service"]);
    expect(result.current.narrowing.map((c) => c.name)).toEqual(["prod"]);
  });

  it("leaves a kind the collection does not filter alone", async () => {
    const { result } = filterFor({ kind: "Service" });
    scopeTo("prod");
    await settle();
    expect(result.current.state.status).toBe("off");
    expect(matchCollectionNames).not.toHaveBeenCalled();
  });

  /** A union: a collection in force that does not filter this kind allows every name. */
  it("leaves the list alone when another collection in force does not filter this kind", async () => {
    const { result } = filterFor();
    scopeTo("prod", "plain");
    await settle();
    expect(result.current.state.status).toBe("off");
    expect(result.current.apply(ROWS)).toBe(ROWS);
  });

  it("matches again when the rows change, keeping the last answer on screen meanwhile", async () => {
    const { result, rerender } = filterFor();
    scopeTo("prod");
    await settle();
    const more = [...ROWS, { name: "payment-gateway", namespace: "prod-eu" }];
    rerender({ k: { kind: "Deployment" }, r: more });
    expect(result.current.state.status).toBe("ready");
    await settle();
    expect(matchCollectionNames).toHaveBeenCalledTimes(2);
    expect(result.current.apply(more)).toHaveLength(3);
  });

  it("says so when the names could not be matched, and lists nothing", async () => {
    matchCollectionNames.mockRejectedValue(new Error("backend gone"));
    const { result } = filterFor();
    scopeTo("prod");
    await settle();
    expect(result.current.state).toEqual({ status: "failed", error: "backend gone" });
    expect(result.current.apply(ROWS)).toEqual([]);
  });
});
