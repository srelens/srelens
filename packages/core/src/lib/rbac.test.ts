import { describe, it, expect, vi } from "vitest";
import {
  listServiceAccounts,
  listRoles,
  listClusterRoles,
  listRoleBindings,
  listClusterRoleBindings,
  podsForServiceAccount,
  bindingsForServiceAccount,
} from "./rbac";

describe("listServiceAccounts", () => {
  it("calls k8s.listServiceAccounts and returns typed rows", async () => {
    const invoke = vi.fn().mockResolvedValue({
      serviceaccounts: [{ name: "builder", namespace: "ci", secrets: 2, age: "3d" }],
    });
    const out = await listServiceAccounts("kind-dev", "ci", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listServiceAccounts", { context: "kind-dev", namespace: "ci" });
    expect(out.serviceaccounts?.[0].secrets).toBe(2);
  });

  it("returns an error string when the call rejects", async () => {
    const out = await listServiceAccounts("x", "ci", () => Promise.reject(new Error("boom")));
    expect(out.error).toContain("boom");
  });
});

describe("listRoles / listClusterRoles", () => {
  it("lists namespaced roles with rule counts", async () => {
    const invoke = vi.fn().mockResolvedValue({ roles: [{ name: "reader", namespace: "default", rules: 3, age: "1d" }] });
    const out = await listRoles("kind-dev", "default", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listRoles", { context: "kind-dev", namespace: "default" });
    expect(out.roles?.[0].rules).toBe(3);
  });

  it("lists cluster roles without a namespace", async () => {
    const invoke = vi.fn().mockResolvedValue({ clusterroles: [{ name: "admin", rules: 10, age: "9d" }] });
    const out = await listClusterRoles("kind-dev", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listClusterRoles", { context: "kind-dev" });
    expect(out.clusterroles?.[0].rules).toBe(10);
  });
});

describe("listRoleBindings / listClusterRoleBindings", () => {
  it("lists namespaced bindings with roleRef and subject count", async () => {
    const invoke = vi.fn().mockResolvedValue({
      rolebindings: [{ name: "read-pods", namespace: "default", role: "Role/pod-reader", subjects: 1, age: "2d" }],
    });
    const out = await listRoleBindings("kind-dev", "default", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listRoleBindings", { context: "kind-dev", namespace: "default" });
    expect(out.rolebindings?.[0].role).toBe("Role/pod-reader");
  });

  it("lists cluster bindings without a namespace", async () => {
    const invoke = vi.fn().mockResolvedValue({
      clusterrolebindings: [{ name: "admin-binding", role: "ClusterRole/cluster-admin", subjects: 0, age: "5d" }],
    });
    const out = await listClusterRoleBindings("kind-dev", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listClusterRoleBindings", { context: "kind-dev" });
    expect(out.clusterrolebindings?.[0].role).toBe("ClusterRole/cluster-admin");
  });
});

describe("bindingsForServiceAccount", () => {
  it("calls k8s.bindingsForServiceAccount with the SA name", async () => {
    const invoke = vi.fn().mockResolvedValue({
      bindings: [{ name: "read-pods", namespace: "ci", kind: "RoleBinding", role: "Role/pod-reader" }],
    });
    const out = await bindingsForServiceAccount("kind-dev", "ci", "builder", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.bindingsForServiceAccount", {
      context: "kind-dev",
      namespace: "ci",
      serviceaccount: "builder",
    });
    expect(out.bindings?.[0].role).toBe("Role/pod-reader");
  });
});

describe("podsForServiceAccount", () => {
  it("calls k8s.podsForServiceAccount with the SA name", async () => {
    const invoke = vi.fn().mockResolvedValue({ pods: [{ name: "web-1", namespace: "default" }] });
    const out = await podsForServiceAccount("kind-dev", "default", "builder", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.podsForServiceAccount", {
      context: "kind-dev",
      namespace: "default",
      serviceaccount: "builder",
    });
    expect(out.pods?.[0].name).toBe("web-1");
  });
});
