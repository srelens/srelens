import { describe, it, expect, vi } from "vitest";
import { listIngresses, listEndpointSlices, listNetworkPolicies } from "./network";

describe("listIngresses", () => {
  it("calls k8s.listIngresses and returns typed rows", async () => {
    const invoke = vi.fn().mockResolvedValue({
      ingresses: [
        { name: "web", namespace: "default", class: "nginx", hosts: "app.example.com", address: "203.0.113.4", ports: "80, 443", age: "3d" },
      ],
    });
    const out = await listIngresses("kind-dev", "default", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listIngresses", { context: "kind-dev", namespace: "default" });
    expect(out.ingresses?.[0].hosts).toBe("app.example.com");
  });

  it("returns an error string when the call rejects", async () => {
    const out = await listIngresses("x", "default", () => Promise.reject(new Error("boom")));
    expect(out.error).toContain("boom");
  });
});

describe("listEndpointSlices", () => {
  it("calls k8s.listEndpointSlices and returns typed rows", async () => {
    const invoke = vi.fn().mockResolvedValue({
      endpointslices: [
        { name: "web-abc", namespace: "default", addressType: "IPv4", endpoints: "2/3", ports: "8080", service: "web", age: "1h" },
      ],
    });
    const out = await listEndpointSlices("kind-dev", "default", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listEndpointSlices", { context: "kind-dev", namespace: "default" });
    expect(out.endpointslices?.[0].service).toBe("web");
  });
});

describe("listNetworkPolicies", () => {
  it("calls k8s.listNetworkPolicies and returns typed rows", async () => {
    const invoke = vi.fn().mockResolvedValue({
      networkpolicies: [
        { name: "deny", namespace: "default", podSelector: "app=web", ingress: 1, egress: 2, policyTypes: "Ingress, Egress", age: "5d" },
      ],
    });
    const out = await listNetworkPolicies("kind-dev", "default", invoke);
    expect(invoke).toHaveBeenCalledWith("k8s.listNetworkPolicies", { context: "kind-dev", namespace: "default" });
    expect(out.networkpolicies?.[0].policyTypes).toBe("Ingress, Egress");
  });
});
