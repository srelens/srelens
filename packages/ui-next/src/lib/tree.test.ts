import { describe, it, expect } from "vitest";
import type { CrdRef } from "@srelens/core";
import { Icons } from "./icons";
import type { ResourceNode } from "@srelens/ui-kit";
import { crdNodes, kindNodes, NAV_GROUPS, navLabel, navNodes, routeForNode } from "./tree";

const labels = (nodes: ResourceNode[] | undefined) => (nodes ?? []).map((n) => n.label);
const ids = (nodes: ResourceNode[] | undefined) => (nodes ?? []).map((n) => n.id);

const crd = (name: string, group: string, kind: string): CrdRef => ({
  name,
  group,
  version: "v1",
  kind,
  plural: name.split(".")[0],
  namespaced: true,
});

const CERTS = crd("certificates.cert-manager.io", "cert-manager.io", "Certificate");
const ISSUERS = crd("issuers.cert-manager.io", "cert-manager.io", "Issuer");
const ARGO = crd("applications.argoproj.io", "argoproj.io", "Application");

describe("navLabel", () => {
  it("puts core's Title Case labels in sentence case", () => {
    expect(navLabel("Port Forwards")).toBe("Port forwards");
    expect(navLabel("Cluster Role Bindings")).toBe("Cluster role bindings");
    expect(navLabel("Mutating Webhook Configs")).toBe("Mutating webhook configs");
  });

  it("keeps a Kubernetes kind's own capitals", () => {
    expect(navLabel("ConfigMaps")).toBe("ConfigMaps");
    expect(navLabel("StatefulSets")).toBe("StatefulSets");
    expect(navLabel("Custom resources")).toBe("Custom resources");
  });
});

describe("kindNodes", () => {
  it("is the five resource groups, in order", () => {
    expect(labels(kindNodes())).toEqual(["Workloads", "Config", "Network", "Storage", "Access control"]);
    expect(ids(kindNodes())).toEqual(NAV_GROUPS.map((g) => g.id));
  });

  it("labels every kind in sentence case, keeping the kinds' own names", () => {
    const workloads = kindNodes().find((n) => n.id === "workloads");
    expect(labels(workloads?.children)).toEqual(["Pods", "Deployments", "StatefulSets", "DaemonSets", "ReplicaSets", "Jobs", "CronJobs"]);
    expect(ids(workloads?.children)).toEqual([
      "kind:pods",
      "kind:deployments",
      "kind:statefulsets",
      "kind:daemonsets",
      "kind:replicasets",
      "kind:jobs",
      "kind:cronjobs",
    ]);
    const config = kindNodes().find((n) => n.id === "config");
    expect(labels(config?.children)).toContain("Horizontal pod autoscalers");
    expect(labels(config?.children)).toContain("ConfigMaps");
  });

  it("carries a glyph on every group and every leaf", () => {
    for (const group of kindNodes()) {
      expect(group.icon).toBeTruthy();
      for (const child of group.children ?? []) expect(child.icon).toBeTruthy();
    }
  });

  it("has no group that folds shut to begin with", () => {
    for (const group of kindNodes()) expect(group.defaultExpanded).not.toBe(false);
  });
});

describe("navNodes", () => {
  const app: ResourceNode = { id: "extension:flux", label: "Flux" };
  const tree = navNodes({ apps: [app], crds: [], agent: true });

  it("arranges the tree in three bands: the cluster, its resources, then apps and tools", () => {
    expect(ids(tree)).toEqual([
      "route:/overview", "kind:nodes", "kind:namespaces", "kind:events",
      "workloads", "config", "network", "storage", "access", "crds",
      "extension:flux", "route:/helm", "route:/forwards", "route:/topology", "route:/incidents", "route:/agent",
    ]);
    expect(tree.map((n) => n.band)).toEqual([
      ...Array(4).fill("cluster"),
      ...Array(6).fill("resources"),
      ...Array(6).fill("tools"),
    ]);
    expect(labels(tree.slice(10))).toEqual(["Flux", "Helm releases", "Port forwards", "Topology", "Incidents", "Agent"]);
  });

  it("has no Home: that is a new tab's start screen, not a place in a cluster", () => {
    const all = (nodes: ResourceNode[]): ResourceNode[] => nodes.flatMap((n) => [n, ...all(n.children ?? [])]);
    expect(all(tree).map((n) => routeForNode(n.id, []))).not.toContain("/");
  });

  it("offers no Agent where the assistant may not act", () => {
    expect(ids(navNodes({ crds: [], agent: false }))).not.toContain("route:/agent");
  });

  it("keeps Custom resources shut to begin with, holding what it is given", () => {
    const crds = navNodes({ crds: crdNodes([CERTS]), agent: false }).find((n) => n.id === "crds");
    expect(crds?.defaultExpanded).toBe(false);
    expect(ids(crds?.children)).toEqual(["crdgroup:cert-manager.io"]);
  });
});

describe("crdNodes", () => {
  it("gathers a group's CRDs under one node", () => {
    const nodes = crdNodes([CERTS, ISSUERS]);
    expect(nodes).toHaveLength(1);
    expect(nodes[0].id).toBe("crdgroup:cert-manager.io");
    expect(nodes[0].label).toBe("cert-manager.io");
    expect(labels(nodes[0].children)).toEqual(["Certificate", "Issuer"]);
    expect(ids(nodes[0].children)).toEqual(["crd:certificates.cert-manager.io", "crd:issuers.cert-manager.io"]);
  });

  it("sorts the groups alphabetically", () => {
    expect(ids(crdNodes([CERTS, ARGO, ISSUERS]))).toEqual(["crdgroup:argoproj.io", "crdgroup:cert-manager.io"]);
  });

  it("starts every group folded shut — there can be hundreds", () => {
    for (const group of crdNodes([CERTS, ARGO])) expect(group.defaultExpanded).toBe(false);
  });

  it("gives each custom resource its group's icon, not the fallback", () => {
    expect(crdNodes([CERTS])[0].children?.[0].icon).toBe(Icons.crds);
  });

  it("is empty for no CRDs", () => {
    expect(crdNodes([])).toEqual([]);
  });
});

describe("routeForNode", () => {
  const crds = [CERTS, ISSUERS];

  it("sends a kind to its list", () => {
    expect(routeForNode("kind:pods", crds)).toBe("/k/pods");
  });

  it("sends events to the events screen, which is not a list", () => {
    expect(routeForNode("kind:events", crds)).toBe("/events");
  });

  it("sends a CRD to its list, by full name", () => {
    expect(routeForNode("crd:certificates.cert-manager.io", crds)).toBe("/k/certificates.cert-manager.io");
  });

  it("refuses a CRD the cluster no longer has", () => {
    expect(routeForNode("crd:certificates.cert-manager.io", [])).toBeNull();
  });

  it("passes a route node through", () => {
    expect(routeForNode("route:/incidents", crds)).toBe("/incidents");
  });

  it("has nothing to open for a group", () => {
    expect(routeForNode("crdgroup:x", crds)).toBeNull();
    expect(routeForNode("workloads", crds)).toBeNull();
  });

  it("has nothing to open for the CRD-discovery retry leaf", () => {
    expect(routeForNode("crd-error", crds)).toBeNull();
  });
});

/**
 * #402: the sidebar's per-kind counts ride on the nodes. The map is keyed by
 * node id so one lookup serves kinds, CRDs and the forwards route alike, and
 * a row with no entry must come out exactly as it did before counts existed —
 * no `count` key at all, which is what keeps the kit from drawing a slot.
 */
describe("counts on nodes", () => {
  const leaf = (nodes: ResourceNode[], id: string): ResourceNode | undefined =>
    nodes.flatMap((n) => [n, ...(n.children ?? [])]).find((n) => n.id === id);

  it("puts a count on the kind it names", () => {
    const nodes = navNodes({ counts: new Map([["kind:pods", "38"], ["kind:nodes", "0"]]), crds: [], agent: false });
    expect(leaf(nodes, "kind:pods")?.count).toBe("38");
    // Zero is an answer — a list that came back empty — and is drawn.
    expect(leaf(nodes, "kind:nodes")?.count).toBe("0");
  });

  it("leaves a row with no entry exactly as it was", () => {
    const nodes = kindNodes(new Map([["kind:pods", "38"]]));
    expect("count" in (leaf(nodes, "kind:deployments") ?? {})).toBe(false);
    expect(kindNodes(new Map())).toEqual(kindNodes());
  });

  it("never counts a group", () => {
    const nodes = kindNodes(new Map([["workloads", "9"]]));
    expect("count" in (nodes.find((n) => n.id === "workloads") ?? {})).toBe(false);
  });

  it("counts a CRD and the forwards route", () => {
    const crds = crdNodes([CERTS], new Map([["crd:certificates.cert-manager.io", "4"]]));
    expect(leaf(crds, "crd:certificates.cert-manager.io")?.count).toBe("4");
    expect(leaf(navNodes({ counts: new Map([["route:/forwards", "2"]]), crds: [], agent: false }), "route:/forwards")?.count).toBe("2");
  });
});
