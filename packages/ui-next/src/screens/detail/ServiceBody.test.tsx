import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { EndpointSliceSummary, K8sObject, PodMetric, PodSummary } from "@srelens/core";

// The "Endpoint Slices" section reads live EndpointSlices for the Service's
// namespace via core's `listEndpointSlices` — mocked here so a test controls
// what "the cluster said" without one. `podsForSelector`/`podMetrics` are
// mocked too, since the composition test below renders `GenericBody`, whose
// related-pods section calls them for any Service with a selector.
// `importOriginal` keeps every formatter (`serviceExternalAddress`, `str`,
// `asRecord`, ...) intact.
const { listEndpointSlices, podsForSelector, podMetrics } = vi.hoisted(() => ({
  listEndpointSlices: vi.fn(async (): Promise<{ endpointslices?: EndpointSliceSummary[]; error?: string }> => ({
    endpointslices: [],
  })),
  podsForSelector: vi.fn(async (): Promise<{ pods?: PodSummary[]; error?: string }> => ({ pods: [] })),
  podMetrics: vi.fn(async (): Promise<{ metrics?: PodMetric[]; error?: string }> => ({ metrics: [] })),
}));

// What §A.4's dialog reaches for once a Ports row opens it. Stubbed for the
// same reason as the three above: this file has no cluster. `toKubectl` and
// `kindToForwardTarget` stay REAL — they are what decides that a Service reads
// `svc/`, which is half of what these tests are about.
const forwardCore = vi.hoisted(() => ({
  listNamespaces: vi.fn(async () => ({ namespaces: ["default"] })),
  listServices: vi.fn(async () => ({ services: [{ name: "checkout", namespace: "default" }] })),
  listPods: vi.fn(async () => ({ pods: [] })),
  startPortForward: vi.fn(async () => ({ id: 1, localPort: 9090 })),
}));

vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  listEndpointSlices,
  podsForSelector,
  podMetrics,
  ...forwardCore,
}));

import { GenericBody } from "./GenericBody";
import { ServiceDetailsBody } from "./ServiceBody";

function service(
  spec: Record<string, unknown>,
  status: Record<string, unknown> = {},
  metadata: NonNullable<K8sObject["metadata"]> = { name: "checkout", namespace: "default" },
): K8sObject {
  return { kind: "Service", apiVersion: "v1", metadata, spec, status } as K8sObject;
}

const SLICE_MINE: EndpointSliceSummary = {
  name: "checkout-abcde",
  namespace: "default",
  addressType: "IPv4",
  endpoints: "3/3",
  ports: "80",
  service: "checkout",
  age: "2d",
};

const SLICE_OTHER: EndpointSliceSummary = {
  name: "billing-xyz",
  namespace: "default",
  addressType: "IPv4",
  endpoints: "1/1",
  ports: "80",
  service: "billing",
  age: "2d",
};

describe("ServiceDetailsBody", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listEndpointSlices.mockResolvedValue({ endpointslices: [] });
    podsForSelector.mockResolvedValue({ pods: [] });
    podMetrics.mockResolvedValue({ metrics: [] });
  });

  describe("Connection", () => {
    it("shows the type, cluster IP, external address and session affinity", () => {
      render(
        <ServiceDetailsBody
          object={service({
            type: "LoadBalancer",
            clusterIP: "10.0.0.5",
            sessionAffinity: "ClientIP",
            externalIPs: ["203.0.113.5"],
          })}
          context="ctx"
        />,
      );
      expect(screen.getByText("Type")).toBeDefined();
      expect(screen.getByText("LoadBalancer")).toBeDefined();
      expect(screen.getByText("10.0.0.5")).toBeDefined();
      expect(screen.getByText("203.0.113.5")).toBeDefined();
      expect(screen.getByText("ClientIP")).toBeDefined();
    });

    it("defaults Type to ClusterIP when spec.type is absent", () => {
      render(<ServiceDetailsBody object={service({ clusterIP: "10.0.0.5" })} context="ctx" />);
      expect(screen.getByText("ClusterIP")).toBeDefined();
    });

    it("shows a LoadBalancer with no address yet as pending, not empty", () => {
      render(<ServiceDetailsBody object={service({ type: "LoadBalancer" })} context="ctx" />);
      expect(screen.getByText("<pending>")).toBeDefined();
    });

    it("shows a ClusterIP service's external address as a dash", () => {
      render(<ServiceDetailsBody object={service({ type: "ClusterIP" })} context="ctx" />);
      expect(screen.getByText("—")).toBeDefined();
    });

    it("shows the selector as label pairs", () => {
      // Read off the row's own text. `PairList` deliberately no longer writes
      // the value into a `title` — a truncated row that carries its value in
      // an attribute is a disclosure the reader was never shown.
      render(
        <ServiceDetailsBody
          object={service({ selector: { app: "checkout", tier: "backend" } })}
          context="ctx"
        />,
      );
      expect(screen.getByText("app=")).toBeDefined();
      expect(screen.getByText("checkout")).toBeDefined();
      expect(screen.getByText("tier=")).toBeDefined();
      expect(screen.getByText("backend")).toBeDefined();
    });

    it("wraps a long selector value rather than truncating it out of reach", () => {
      const { container } = render(
        <ServiceDetailsBody
          object={service({ selector: { "app.kubernetes.io/name": "checkout-api" } })}
          context="ctx"
        />,
      );
      expect(container.querySelector(".pairs li.truncate")).toBeNull();
      expect(container.querySelector(".pairs .v.break-all")).not.toBeNull();
    });

    it("omits the selector row for a headless/ExternalName service with none", () => {
      render(<ServiceDetailsBody object={service({ type: "ExternalName" })} context="ctx" />);
      expect(screen.queryByText("Selector")).toBeNull();
    });
  });

  describe("Ports", () => {
    it("shows each port's name, port:nodePort, target and protocol", () => {
      render(
        <ServiceDetailsBody
          object={service({
            ports: [{ name: "http", port: 80, nodePort: 30080, targetPort: 8080, protocol: "TCP" }],
          })}
          context="ctx"
        />,
      );
      expect(screen.getByText("http")).toBeDefined();
      expect(screen.getByText("80:30080")).toBeDefined();
      expect(screen.getByText("8080")).toBeDefined();
      expect(screen.getByText("TCP")).toBeDefined();
    });

    it("defaults protocol to TCP when absent", () => {
      render(
        <ServiceDetailsBody
          object={service({ ports: [{ port: 443, targetPort: 8443 }] })}
          context="ctx"
        />,
      );
      expect(screen.getByText("TCP")).toBeDefined();
    });

    it("omits the Ports section for a service with no ports", () => {
      render(<ServiceDetailsBody object={service({ type: "ExternalName" })} context="ctx" />);
      expect(screen.queryByText("Ports")).toBeNull();
    });
  });

  /**
   * The Ports table is where the reader is already looking at the exact port
   * they want to forward, and until now it could only be read. Every row is a
   * way into §A.4's dialog, prefilled with this Service and THAT row's port.
   *
   * Nothing starts here: the dialog is where the local port is named, the
   * equivalent command is read and the browser switch lives.
   */
  describe("Ports — the way in to a forward", () => {
    /** Two ports, neither of them the dialog's own placeholders (9090/8080),
     *  so a prefill can only have come from the row that was clicked. */
    const TWO_PORTS = service({
      ports: [
        { name: "postgres", port: 5432, targetPort: 9999, protocol: "TCP" },
        { name: "redis", port: 6379, targetPort: 6380, protocol: "TCP" },
      ],
    });

    const select = (name: string) => screen.getByLabelText(name) as HTMLSelectElement;
    const input = (name: string) => screen.getByLabelText(name) as HTMLInputElement;

    it("offers a forward on every port row, and opens nothing until one is picked", () => {
      render(<ServiceDetailsBody object={TWO_PORTS} context="ctx" />);
      expect(screen.getByRole("button", { name: "Forward port 5432" })).toBeDefined();
      expect(screen.getByRole("button", { name: "Forward port 6379" })).toBeDefined();
      // The half that makes every assertion below mean something: a dialog
      // that were always mounted would pass them all.
      expect(screen.queryByRole("dialog")).toBeNull();
    });

    it("opens §A.4's dialog on this Service, in its namespace, on THAT row's port", async () => {
      render(<ServiceDetailsBody object={TWO_PORTS} context="ctx" />);
      // The SECOND row, so a table that always handed over its first port
      // cannot pass.
      await userEvent.click(screen.getByRole("button", { name: "Forward port 6379" }));
      const dialog = await screen.findByRole("dialog");
      expect(within(dialog).getByText("New port forward")).toBeDefined();
      await waitFor(() => expect(select("Target").value).toBe("svc/checkout"));
      await waitFor(() => expect(select("Namespace").value).toBe("default"));
      expect(input("Remote port").value).toBe("6379");
      // Offered, not demanded, and NOT the far end again: a free port drawn
      // from a range nothing claims by convention. Asserted as the property
      // rather than a number, so this test says what it means without
      // pinning the draw.
      const offered = Number(input("Local port").value);
      expect(offered).toBeGreaterThanOrEqual(10000);
      expect(offered).toBeLessThanOrEqual(32767);
      expect(offered).not.toBe(6379);
    });

    it("forwards the service's port, not the targetPort behind it", async () => {
      // `kubectl port-forward svc/x L:R` takes the SERVICE port and resolves
      // the targetPort itself. 9999 is what a cell-reading mistake would grab.
      render(<ServiceDetailsBody object={TWO_PORTS} context="ctx" />);
      await userEvent.click(screen.getByRole("button", { name: "Forward port 5432" }));
      await screen.findByRole("dialog");
      expect(input("Remote port").value).toBe("5432");
      expect(input("Remote port").value).not.toBe("9999");
    });

    it("names the Service as a Service — svc/, from its kind", async () => {
      render(<ServiceDetailsBody object={TWO_PORTS} context="ctx" />);
      await userEvent.click(screen.getByRole("button", { name: "Forward port 5432" }));
      await screen.findByRole("dialog");
      await userEvent.clear(screen.getByLabelText("Local port"));
      await userEvent.type(screen.getByLabelText("Local port"), "9091");
      await waitFor(() =>
        expect(screen.getByText(/port-forward svc\/checkout 9091:5432/)).toBeDefined(),
      );
    });

    it("puts no port or command in a title attribute", async () => {
      render(<ServiceDetailsBody object={TWO_PORTS} context="ctx" />);
      await userEvent.click(screen.getByRole("button", { name: "Forward port 5432" }));
      await screen.findByRole("dialog");
      const joined = Array.from(document.querySelectorAll("[title]"))
        .map((el) => el.getAttribute("title") ?? "")
        .join("\n");
      for (const leak of ["5432", "9999", "port-forward", "--context"]) {
        expect(joined).not.toContain(leak);
      }
    });
  });

  describe("Endpoint Slices", () => {
    it("shows only the slices whose service label matches this Service", async () => {
      listEndpointSlices.mockResolvedValue({ endpointslices: [SLICE_MINE, SLICE_OTHER] });
      render(<ServiceDetailsBody object={service({})} context="ctx" />);
      await waitFor(() => expect(screen.getByText("EndpointSlice/checkout-abcde")).toBeDefined());
      expect(listEndpointSlices).toHaveBeenCalledWith("ctx", "default");
      expect(screen.queryByText("EndpointSlice/billing-xyz")).toBeNull();
      // Inert text — no navigation contract exists here.
      expect(screen.queryByRole("button", { name: /^Open / })).toBeNull();
    });

    it("omits the Endpoint Slices section for a service with none", async () => {
      listEndpointSlices.mockResolvedValue({ endpointslices: [] });
      render(<ServiceDetailsBody object={service({})} context="ctx" />);
      await Promise.resolve();
      expect(screen.queryByText("Endpoint Slices")).toBeNull();
    });
  });

  describe("the run of sections", () => {
    it("is flat blocks divided by rules, not a stack of cards", () => {
      const { container } = render(
        <ServiceDetailsBody
          object={service({ type: "ClusterIP", ports: [{ port: 80, targetPort: 8080 }] })}
          context="ctx"
        />,
      );
      const blocks = [...container.children];
      expect(blocks).toHaveLength(2);
      for (const block of blocks) expect(block.matches("section.section")).toBe(true);
      expect(container.querySelector(".card")).toBeNull();
    });
  });

  describe("composition with GenericBody", () => {
    it("renders exactly one Pods section for a Service reached through GenericBody", async () => {
      const svc = service({ selector: { app: "checkout" } });
      render(
        <GenericBody kind="Service" object={svc} context="ctx">
          <ServiceDetailsBody object={svc} context="ctx" />
        </GenericBody>,
      );
      await waitFor(() => expect(screen.getAllByRole("heading", { name: "Pods" })).toHaveLength(1));
    });
  });
});
