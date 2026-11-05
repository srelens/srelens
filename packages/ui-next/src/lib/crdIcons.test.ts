import { describe, it, expect } from "vitest";
import { BadgeCheck, Cloud, GitMerge, Hammer } from "lucide-react";
import { Icons } from "./icons";
import { crdIcon } from "./crdIcons";

describe("crdIcon", () => {
  it("finds a group by its exact name", () => {
    expect(crdIcon("cert-manager.io")).toBe(BadgeCheck);
  });

  it("falls back from a subgroup to the group it belongs to", () => {
    expect(crdIcon("acme.cert-manager.io")).toBe(BadgeCheck);
    expect(crdIcon("source.toolkit.fluxcd.io")).toBe(GitMerge);
  });

  it("prefers the longer match where both are listed", () => {
    expect(crdIcon("karpenter.k8s.aws")).toBe(Hammer);
    expect(crdIcon("elbv2.k8s.aws")).toBe(Cloud);
  });

  it("matches whole labels only", () => {
    expect(crdIcon("notcert-manager.io")).toBe(Icons.crds);
  });

  it("keeps the generic glyph for a group it does not know", () => {
    expect(crdIcon("widgets.example.com")).toBe(Icons.crds);
    expect(crdIcon("core")).toBe(Icons.crds);
    expect(crdIcon("")).toBe(Icons.crds);
    expect(crdIcon("constructor")).toBe(Icons.crds);
  });

  // Real groups from the operators the table is for. Each must land on a glyph
  // of its own: a key misspelled in the table falls through to the generic one.
  it.each([
    "cert-manager.io",
    "acme.cert-manager.io",
    "argoproj.io",
    "source.toolkit.fluxcd.io",
    "kustomize.toolkit.fluxcd.io",
    "helm.toolkit.fluxcd.io",
    "networking.istio.io",
    "security.istio.io",
    "policy.linkerd.io",
    "cilium.io",
    "crd.projectcalico.org",
    "operator.tigera.io",
    "monitoring.coreos.com",
    "gateway.networking.k8s.io",
    "gateway.networking.x-k8s.io",
    "serving.knative.dev",
    "eventing.knative.dev",
    "keda.sh",
    "velero.io",
    "pkg.crossplane.io",
    "ec2.aws.upbound.io",
    "external-secrets.io",
    "bitnami.com",
    "kyverno.io",
    "wgpolicyk8s.io",
    "templates.gatekeeper.sh",
    "constraints.gatekeeper.sh",
    "traefik.io",
    "traefik.containo.us",
    "k8s.nginx.org",
    "karpenter.sh",
    "karpenter.k8s.aws",
    "s3.services.k8s.aws",
    "eks.amazonaws.com",
    "storage.cnrm.cloud.google.com",
    "networking.gke.io",
    "containerservice.azure.com",
    "kafka.strimzi.io",
    "elasticsearch.k8s.elastic.co",
    "tekton.dev",
    "triggers.tekton.dev",
    "kubevirt.io",
    "cdi.kubevirt.io",
    "longhorn.io",
    "ceph.rook.io",
    "metallb.io",
    "snapshot.storage.k8s.io",
    "metrics.k8s.io",
    "actions.github.com",
    "actions.summerwind.dev",
  ])("gives %s an icon of its own", (group) => {
    const icon = crdIcon(group);
    expect(icon).toBeTruthy();
    expect(icon).not.toBe(Icons.crds);
  });
});
