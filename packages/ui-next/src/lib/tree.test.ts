import { describe, it, expect } from "vitest";
import { RESOURCE_LABELS, type CrdRef } from "@srelens/core";
import type { ResourceNode } from "@srelens/ui-kit";
import { crdNodes, INVESTIGATE, kindNodes, NAV_GROUPS, routeForNode } from "./tree";

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

describe("kindNodes", () => {
  it("is the six groups, in order", () => {
    expect(labels(kindNodes())).toEqual(["Cluster", "Workloads", "Config", "Network", "Storage", "Access control"]);
    expect(ids(kindNodes())).toEqual(NAV_GROUPS.map((g) => g.id));
  });

  it("labels every kind the way core does", () => {
    const workloads = kindNodes().find((n) => n.id === "workloads");
    expect(labels(workloads?.children)).toEqual([
      RESOURCE_LABELS.pods,
      RESOURCE_LABELS.deployments,
      RESOURCE_LABELS.statefulsets,
      RESOURCE_LABELS.daemonsets,
      RESOURCE_LABELS.replicasets,
      RESOURCE_LABELS.jobs,
      RESOURCE_LABELS.cronjobs,
    ]);
    expect(ids(workloads?.children)).toEqual([
      "kind:pods",
      "kind:deployments",
      "kind:statefulsets",
      "kind:daemonsets",
      "kind:replicasets",
      "kind:jobs",
      "kind:cronjobs",
    ]);
  });

  it("carries a glyph on every group and every leaf", () => {
    for (const group of kindNodes()) {
      expect(group.icon).toBeTruthy();
      for (const child of group.children ?? []) expect(child.icon).toBeTruthy();
    }
  });

  it("puts overview, port forwards and helm in the Cluster group as routes, not kinds", () => {
    const cluster = kindNodes()[0];
    expect(ids(cluster.children)).toEqual([
      "route:/overview",
      "kind:nodes",
      "kind:namespaces",
      "kind:events",
      "route:/forwards",
      "route:/helm",
    ]);
    expect(labels(cluster.children)).toEqual([
      RESOURCE_LABELS.overview,
      RESOURCE_LABELS.nodes,
      RESOURCE_LABELS.namespaces,
      RESOURCE_LABELS.events,
      RESOURCE_LABELS.portforwards,
      RESOURCE_LABELS.helmreleases,
    ]);
  });

  it("has no group that folds shut to begin with", () => {
    for (const group of kindNodes()) expect(group.defaultExpanded).not.toBe(false);
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

describe("INVESTIGATE", () => {
  it("is the control room, incidents, topology and the agent", () => {
    expect(INVESTIGATE.map((i) => [i.label, i.route])).toEqual([
      ["Control room", "/"],
      ["Incidents", "/incidents"],
      ["Topology", "/topology"],
      ["Agent", "/agent"],
    ]);
  });
});
