import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { openExternal, type K8sObject } from "@srelens/core";
import { IngressDetailsBody } from "./IngressBody";

vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  openExternal: vi.fn(),
}));

/** jsdom ships no clipboard at all, so there is nothing to spy on. */
const writeText = vi.fn();

beforeEach(() => {
  vi.mocked(openExternal).mockReset().mockResolvedValue(undefined);
  writeText.mockReset().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
});

/** One rule, one path: what most of the address tests need. */
function rule(host: string | undefined, path: string) {
  return {
    ...(host === undefined ? {} : { host }),
    http: { paths: [{ path, pathType: "Prefix", backend: { service: { name: "web", port: { number: 80 } } } }] },
  };
}

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

    it("shows a resource backend as kind/name instead of a bare :", () => {
      // networking.k8s.io/v1 backends are service XOR resource. Classic's
      // cell only formats the service form, which leaves resource backends
      // as ":"; show kind/name when service is absent.
      render(
        <IngressDetailsBody
          object={ingress({
            rules: [
              {
                host: "app.example.com",
                http: {
                  paths: [
                    {
                      path: "/",
                      backend: {
                        resource: {
                          apiGroup: "gateway.networking.k8s.io",
                          kind: "ServiceImport",
                          name: "web",
                        },
                      },
                    },
                  ],
                },
              },
            ],
          })}
        />,
      );
      expect(screen.getByText("ServiceImport/web")).toBeDefined();
      expect(screen.queryByText(":")).toBeNull();
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

  describe("rule addresses (#774)", () => {
    it("opens a rule's address in the system browser, over https when the Ingress terminates TLS for it", async () => {
      render(
        <IngressDetailsBody
          object={ingress({
            tls: [{ hosts: ["app.example.com"], secretName: "web-tls" }],
            rules: [rule("app.example.com", "/api")],
          })}
        />,
      );

      await userEvent.click(screen.getByRole("button", { name: "https://app.example.com/api" }));

      expect(openExternal).toHaveBeenCalledWith("https://app.example.com/api");
    });

    it("copies the same address the link opens", async () => {
      render(<IngressDetailsBody object={ingress({ rules: [rule("app.example.com", "/api")] })} />);

      await userEvent.click(screen.getByRole("button", { name: "Copy http://app.example.com/api" }));

      expect(writeText).toHaveBeenCalledWith("http://app.example.com/api");
    });

    it("links a path ingress-nginx reads as a regex to the host's root", () => {
      render(
        <IngressDetailsBody
          object={ingress(
            { rules: [rule("app.example.com", "/api(/|$)(.*)")] },
            {
              name: "web",
              namespace: "default",
              annotations: { "nginx.ingress.kubernetes.io/rewrite-target": "/$2" },
            },
          )}
        />,
      );

      expect(screen.getByRole("button", { name: "http://app.example.com/" })).toBeDefined();
      // The rule's own path still reads as written, in its own column.
      expect(screen.getByText("/api(/|$)(.*)")).toBeDefined();
    });

    it("shows a wildcard host as text, and copies it as it is", async () => {
      render(<IngressDetailsBody object={ingress({ rules: [rule("*.corp.dev", "/")] })} />);

      expect(screen.queryByRole("button", { name: /^https?:\/\// })).toBeNull();
      await userEvent.click(screen.getByRole("button", { name: "Copy *.corp.dev" }));

      expect(writeText).toHaveBeenCalledWith("*.corp.dev");
      expect(openExternal).not.toHaveBeenCalled();
    });

    it("offers nothing to open or copy for a rule with no host", () => {
      render(<IngressDetailsBody object={ingress({ rules: [rule(undefined, "/health")] })} />);

      expect(screen.queryByRole("button", { name: /^https?:\/\// })).toBeNull();
      expect(screen.queryByRole("button", { name: /^Copy / })).toBeNull();
      expect(screen.getByText("—")).toBeDefined();
    });

    it("keeps a long address whole on one line, for the table's own scroll to carry", () => {
      // Machine text stays on one line and scrolls in a bounded region
      // (design.md): an ellipsis hid the end of a long path, the part a reader
      // checks before opening it. jsdom lays nothing out, so the classes that
      // decide it are what can be asserted. (#797 review)
      const url = "http://checkout.internal.example.com/api/v2/orders/fulfilment/status";
      render(
        <IngressDetailsBody
          object={ingress({
            rules: [
              rule("checkout.internal.example.com", "/api/v2/orders/fulfilment/status"),
              rule("*.corp.example.com", "/"),
            ],
          })}
        />,
      );

      for (const text of [
        within(screen.getByRole("button", { name: url })).getByText(url),
        screen.getAllByText("*.corp.example.com").find((el) => el.closest("td")?.querySelector("button"))!,
      ]) {
        expect(text.className).toContain("whitespace-nowrap");
        for (let el: HTMLElement | null = text; el && el.tagName !== "TD"; el = el.parentElement) {
          expect(el.className).not.toContain("truncate");
        }
      }
    });

    it("says so when the browser could not be opened", async () => {
      vi.mocked(openExternal).mockRejectedValue(new Error("no handler for http"));
      render(<IngressDetailsBody object={ingress({ rules: [rule("app.example.com", "/api")] })} />);

      await userEvent.click(screen.getByRole("button", { name: "http://app.example.com/api" }));

      expect(await screen.findByText("Could not open http://app.example.com/api")).toBeDefined();
    });
  });
});
