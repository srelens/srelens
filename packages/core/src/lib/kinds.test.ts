import { describe, it, expect } from "vitest";
import { K8S_KIND, RESOURCE_LABELS, kindToResource, type ResourceKind } from "./kinds";

describe("kinds", () => {
  it("maps a resource kind to its Kubernetes kind", () => {
    expect(K8S_KIND.pods).toBe("Pod");
    expect(K8S_KIND.deployments).toBe("Deployment");
    expect(K8S_KIND.statefulsets).toBe("StatefulSet");
  });

  it("gives overview no Kubernetes kind, since it is a view not a resource", () => {
    expect(K8S_KIND.overview).toBe("");
  });

  it("labels every kind it can map", () => {
    // The two tables must stay in step: a kind with a Kubernetes name but no
    // label renders as blank in the sidebar.
    const kinds = Object.keys(K8S_KIND) as ResourceKind[];
    for (const kind of kinds) {
      expect(RESOURCE_LABELS[kind], `no label for ${kind}`).toBeTruthy();
    }
  });
});

describe("kindToResource", () => {
  it("maps known kinds to {group, resource}, null otherwise", () => {
    expect(kindToResource("Deployment")).toEqual({ group: "apps", resource: "deployments" });
    expect(kindToResource("Pod")).toEqual({ group: "", resource: "pods" });
    expect(kindToResource("CronJob")).toEqual({ group: "batch", resource: "cronjobs" });
    expect(kindToResource("Wibble")).toBeNull();
  });
});
