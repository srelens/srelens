import { describe, expect, it } from "vitest";
import { serviceExternalAddress } from "./serviceAddress";

describe("serviceExternalAddress", () => {
  it("reads a load balancer's ip", () => {
    expect(
      serviceExternalAddress({
        spec: { type: "LoadBalancer" },
        status: { loadBalancer: { ingress: [{ ip: "34.1.2.3" }] } },
      }),
    ).toBe("34.1.2.3");
  });

  it("reads a load balancer's hostname", () => {
    // AWS publishes a hostname rather than an ip; without this a fully
    // provisioned service looks like it has no address.
    expect(
      serviceExternalAddress({
        spec: { type: "LoadBalancer" },
        status: { loadBalancer: { ingress: [{ hostname: "a1b2.elb.amazonaws.com" }] } },
      }),
    ).toBe("a1b2.elb.amazonaws.com");
  });

  it("separates a pending load balancer from having no address at all", () => {
    expect(serviceExternalAddress({ spec: { type: "LoadBalancer" } })).toBe("<pending>");
    expect(serviceExternalAddress({ spec: { type: "ClusterIP" } })).toBe("");
  });

  it("includes manually assigned external IPs on any type", () => {
    expect(
      serviceExternalAddress({ spec: { type: "NodePort", externalIPs: ["192.0.2.1", "192.0.2.2"] } }),
    ).toBe("192.0.2.1, 192.0.2.2");
  });

  it("shows the load balancer and the assigned addresses together", () => {
    expect(
      serviceExternalAddress({
        spec: { type: "LoadBalancer", externalIPs: ["192.0.2.1"] },
        status: { loadBalancer: { ingress: [{ ip: "34.1.2.3" }] } },
      }),
    ).toBe("34.1.2.3, 192.0.2.1");
  });

  it("resolves an ExternalName to what it points at", () => {
    expect(
      serviceExternalAddress({ spec: { type: "ExternalName", externalName: "db.example.com" } }),
    ).toBe("db.example.com");
  });

  it("treats a service with no type as ClusterIP, as the API does", () => {
    expect(serviceExternalAddress({ spec: {} })).toBe("");
  });

  it("survives an object that hasn't loaded or is malformed", () => {
    expect(serviceExternalAddress(undefined)).toBe("");
    expect(serviceExternalAddress({ spec: { type: "LoadBalancer" }, status: "nonsense" })).toBe(
      "<pending>",
    );
    expect(
      serviceExternalAddress({
        spec: { type: "LoadBalancer" },
        status: { loadBalancer: { ingress: [{}] } },
      }),
    ).toBe("<pending>");
  });
});
