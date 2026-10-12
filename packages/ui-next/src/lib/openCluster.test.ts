import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClusterContext } from "@srelens/core";

vi.mock("./probe", () => ({ invalidateProbe: vi.fn(), probeCluster: vi.fn(async () => undefined) }));

import { openCluster, openOnCluster } from "./openCluster";
import { defaultState } from "./tabs";
import * as store from "./tabsStore";

const context = (stableId: string, name: string): ClusterContext => ({
  name, stableId, key: stableId, cluster: name, server: `https://${name}`, isCurrent: false,
  sourceFile: "/home/dana/.kube/config", authKind: "client certificate",
});
const PROD = context("id-prod", "prod-eu");
const STAGE = context("id-stage", "staging-eu");

const routes = () => store.currentWorkspace().tabs.map((t) => t.route);
const active = () => store.currentWorkspace().tabs.find((t) => t.id === store.currentWorkspace().activeId)!;

beforeEach(() => {
  store.setState(defaultState([PROD, STAGE]));
});

/**
 * Clicking a cluster in the rail used to mean "its overview", always. With
 * each cluster keeping its own tabs (#908) that would move the reader off the
 * tab they left every time they came back.
 */
describe("openCluster", () => {
  it("opens the overview of a cluster looked at for the first time, and nothing of the cluster left", () => {
    store.openTab("/k/pods", { clusterName: "prod-eu" });
    openCluster(STAGE);
    expect(store.activeCluster()).toBe("id-stage");
    expect(routes()).toEqual(["/", "/overview"]);
    expect(active().sub).toBe("staging-eu");
  });

  it("comes back to the tab the reader left on a cluster, without opening its overview over it", () => {
    store.openTab("/k/pods", { clusterName: "prod-eu" });
    store.openTab("/k/services", { clusterName: "prod-eu" });
    openCluster(STAGE);
    openCluster(PROD);
    expect(routes()).toEqual(["/", "/k/pods", "/k/services"]);
    expect(active().route).toBe("/k/services");
  });

  it("opens the overview when the reader left the cluster on an app-wide tab, which says nothing about it", () => {
    store.openTab("/k/pods", { clusterName: "prod-eu" });
    store.openTab("/settings");
    openCluster(STAGE);
    openCluster(PROD);
    expect(active().route).toBe("/overview");
    expect(routes()).toEqual(["/", "/k/pods", "/settings", "/overview"]);
  });

  it("still opens the overview of the cluster already in focus: that is what choosing it again asks for", () => {
    store.openTab("/k/pods", { clusterName: "prod-eu" });
    openCluster(PROD);
    expect(active().route).toBe("/overview");
    expect(routes()).toEqual(["/", "/k/pods", "/overview"]);
  });

  it("puts a cluster the workspace does not have into it, on its overview", () => {
    const other = context("id-dev", "dev");
    openCluster(other);
    expect(store.currentWorkspace().clusters).toContain("id-dev");
    expect(active().route).toBe("/overview");
    expect(active().sub).toBe("dev");
  });
});

describe("openOnCluster", () => {
  it("opens the route among the cluster's own tabs, not among those of the cluster left", () => {
    store.openTab("/k/pods", { clusterName: "prod-eu" });
    openCluster(STAGE);
    store.openTab("/k/nodes", { clusterName: "staging-eu" });
    openOnCluster(PROD, "/k/deployments");
    expect(routes()).toEqual(["/", "/k/pods", "/k/deployments"]);
    expect(active().sub).toBe("prod-eu");
  });
});
