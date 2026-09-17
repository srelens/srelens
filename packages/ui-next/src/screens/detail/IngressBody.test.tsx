import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import type { K8sObject } from "@srelens/core";
import { IngressDetailsBody } from "./IngressBody";

function ingress(
  spec: Record<string, unknown>,
  metadata: NonNullable<K8sObject["metadata"]> = { name: "web", namespace: "default" },
): K8sObject {
  return { kind: "Ingress", apiVersion: "networking.k8s.io/v1", metadata, spec } as K8sObject;
}

describe("IngressDetailsBody", () => {
  describe("Ingress", () => {
    it("shows the ingress class", () => {
      render(<IngressDetailsBody object={ingress({ ingressClassName: "nginx" })} />);
      expect(screen.getByText("Class")).toBeDefined();
      expect(screen.getByText("nginx")).toBeDefined();
    });

    it("shows TLS hosts and secrets", () => {
      render(
        <IngressDetailsBody
          object={ingress({
            tls: [{ hosts: ["app.example.com", "www.example.com"], secretName: "web-tls" }],
          })}
        />,
      );
      expect(screen.getByText("app.example.com, www.example.com")).toBeDefined();
      expect(screen.getByText("Secret/web-tls")).toBeDefined();
    });
  });

  describe("Rules", () => {
    it("shows each rule's host, path and service:port backend", () => {
      // The contract classic's ObjectDetail test pins: path and service
      // mapping on the details pane. Without this body the new design never
      // rendered either.
      render(
        <IngressDetailsBody
          object={ingress({
            ingressClassName: "nginx",
            rules: [
              {
                host: "app.example.com",
                http: {
                  paths: [
                    {
                      path: "/",
                      backend: { service: { name: "web", port: { number: 80 } } },
                    },
                    {
                      path: "/api",
                      pathType: "Prefix",
                      backend: { service: { name: "api", port: { name: "http" } } },
                    },
                  ],
                },
              },
            ],
          })}
        />,
      );
      expect(screen.getAllByText("app.example.com")).toHaveLength(2);
      expect(screen.getByText("/")).toBeDefined();
      expect(screen.getByText("web:80")).toBeDefined();
      expect(screen.getByText("/api")).toBeDefined();
      expect(screen.getByText("api:http")).toBeDefined();
    });

    it("defaults a missing host to * and a missing path to /", () => {
      render(
        <IngressDetailsBody
          object={ingress({
            rules: [
              {
                http: {
                  paths: [{ backend: { service: { name: "web", port: { number: 80 } } } }],
                },
              },
            ],
          })}
        />,
      );
      expect(screen.getByText("*")).toBeDefined();
      expect(screen.getByText("/")).toBeDefined();
      expect(screen.getByText("web:80")).toBeDefined();
    });

    it("omits the Rules section when there are no path mappings", () => {
      render(<IngressDetailsBody object={ingress({ ingressClassName: "nginx" })} />);
      expect(screen.queryByText("Rules")).toBeNull();
    });

    it("lists a second host's paths as their own rows", () => {
      render(
        <IngressDetailsBody
          object={ingress({
            rules: [
              {
                host: "a.example.com",
                http: {
                  paths: [{ path: "/", backend: { service: { name: "a", port: { number: 80 } } } }],
                },
              },
              {
                host: "b.example.com",
                http: {
                  paths: [{ path: "/", backend: { service: { name: "b", port: { number: 8080 } } } }],
                },
              },
            ],
          })}
        />,
      );
      expect(screen.getByText("a.example.com")).toBeDefined();
      expect(screen.getByText("a:80")).toBeDefined();
      expect(screen.getByText("b.example.com")).toBeDefined();
      expect(screen.getByText("b:8080")).toBeDefined();
    });
  });
});
