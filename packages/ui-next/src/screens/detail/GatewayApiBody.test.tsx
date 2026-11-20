import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ClusterContext, CrdRef, K8sObject } from "@srelens/core";

const { listCrds, listCustomResource, useAccess } = vi.hoisted(() => ({
  listCrds: vi.fn(),
  listCustomResource: vi.fn(),
  useAccess: vi.fn(),
}));

vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  listCrds,
  listCustomResource,
}));

vi.mock("@srelens/core/react", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core/react")>()),
  useAccess,
}));

/** What `useAccess` answers: not yet known, the hook's own state while it asks. */
const unknownAccess = { allowed: () => false, reason: () => "", known: () => false, loading: true };
/** The cluster answered: no. */
const deniedAccess = { allowed: () => false, reason: () => "", known: () => true, loading: false };

import { resetContexts } from "../../lib/clusters";
import { defaultState } from "../../lib/tabs";
import * as store from "../../lib/tabsStore";
import { GatewayApiBody } from "./GatewayApiBody";

const G = "gateway.networking.k8s.io";
const CTX: ClusterContext = {
  name: "prod-eu",
  stableId: "prod",
  key: "prod",
  cluster: "prod",
  server: "https://prod",
  isCurrent: true,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
};

const crd = (plural: string, kind: string): CrdRef => ({
  name: `${plural}.${G}`,
  group: G,
  version: "v1",
  kind,
  plural,
  namespaced: true,
});

const GATEWAY: K8sObject = {
  kind: "Gateway",
  metadata: { name: "edge", namespace: "infra" },
  spec: { listeners: [{ name: "https", protocol: "HTTPS", port: 443, hostname: "*.example.com" }] },
  status: { listeners: [{ name: "https", attachedRoutes: 2, conditions: [{ type: "Programmed", status: "True" }] }] },
};

const ROUTE: K8sObject = {
  kind: "HTTPRoute",
  metadata: { name: "shop", namespace: "store" },
  spec: {
    parentRefs: [{ name: "edge", namespace: "infra" }],
    rules: [
      {
        matches: [{ path: { type: "PathPrefix", value: "/api" } }],
        backendRefs: [
          { name: "api", port: 8080, weight: 90 },
          { group: "multicluster.x-k8s.io", kind: "ServiceImport", name: "far" },
        ],
      },
    ],
  },
  status: {
    parents: [{ parentRef: { name: "edge", namespace: "infra" }, conditions: [{ type: "Accepted", status: "False" }] }],
  },
};

const lastRoute = () => store.currentWorkspace().tabs.at(-1)?.route;

beforeEach(() => {
  vi.clearAllMocks();
  store.setState(defaultState([CTX]));
  resetContexts();
  useAccess.mockReturnValue(unknownAccess);
  listCrds.mockResolvedValue({ crds: [crd("gateways", "Gateway"), crd("httproutes", "HTTPRoute"), crd("grpcroutes", "GRPCRoute")] });
  listCustomResource.mockImplementation(async (_context: string, ref: CrdRef) =>
    ref.kind === "HTTPRoute"
      ? {
          items: [
            { name: "shop", namespace: "store", age: "", columns: ["edge", "infra", "", "", "", "", "", ""] },
            { name: "elsewhere", namespace: "store", age: "", columns: ["other", "", "", "", "", "", "", ""] },
          ],
        }
      : { items: [] },
  );
});

describe("a Gateway's pane", () => {
  it("lists each listener with what is bound to it", () => {
    render(<GatewayApiBody object={GATEWAY} context="prod-eu" />);
    const row = screen.getByRole("row", { name: /https/ });
    for (const text of ["HTTPS", "443", "*.example.com", "2", "True"]) {
      expect(within(row).getByText(text)).toBeDefined();
    }
  });

  it("lists the routes attached to it, across every route kind the cluster serves", async () => {
    render(<GatewayApiBody object={GATEWAY} context="prod-eu" />);
    expect(await screen.findByRole("heading", { name: "Routes (1)" })).toBeDefined();
    expect(screen.queryByText("elsewhere")).toBeNull();
    // Every namespace, with the parent columns: a route may attach from anywhere.
    const asked = listCustomResource.mock.calls.map(([, ref, namespace]) => [ref.kind, namespace, ref.printerColumns.length]);
    expect(asked).toEqual([
      ["HTTPRoute", null, 16],
      ["GRPCRoute", null, 16],
    ]);
  });

  it("opens a route from its row", async () => {
    render(<GatewayApiBody object={GATEWAY} context="prod-eu" />);
    await userEvent.click(await screen.findByText("shop"));
    expect(lastRoute()).toBe(`/k/prod-eu/${encodeURIComponent(`${G}/HTTPRoute`)}/store/shop`);
  });

  it("falls back to the gateway's namespace, and names a kind it still could not read", async () => {
    // A role bound in one namespace cannot list cluster-wide; a role without
    // grpcroutes cannot list them at all. Neither empties the section.
    listCustomResource.mockImplementation(async (_context: string, ref: CrdRef, namespace: string | null) =>
      ref.kind === "HTTPRoute" && namespace === "infra"
        ? { items: [{ name: "local", namespace: "infra", age: "", columns: ["edge", "", "", "", "", "", "", ""] }] }
        : { error: "forbidden" },
    );
    render(<GatewayApiBody object={GATEWAY} context="prod-eu" />);
    // Only the gateway's namespace was read for HTTPRoute: a lower bound, said so.
    expect(await screen.findByRole("heading", { name: "Routes (1+)" })).toBeDefined();
    expect(screen.getByText("Listed HTTPRoute only in infra")).toBeDefined();
    expect(screen.getByText("local")).toBeDefined();
    expect(screen.getByText("Could not list GRPCRoute")).toBeDefined();
    // The shared failure display: the reason, and a way to ask again.
    const calls = listCustomResource.mock.calls.length;
    await userEvent.click(screen.getByRole("button", { name: "Retry listing GRPCRoute" }));
    await waitFor(() => expect(listCustomResource.mock.calls.length).toBeGreaterThan(calls));
    expect(await screen.findByText("local")).toBeDefined();
    expect(screen.getByRole("button", { name: "Retry listing HTTPRoute across the cluster" })).toBeDefined();
  });

  it("does not present a list cut at the row cap as complete", async () => {
    // Past the shared cap the routes that attach here may be the ones not read.
    listCustomResource.mockResolvedValue({ items: [], truncated: true });
    render(<GatewayApiBody object={GATEWAY} context="prod-eu" />);
    expect(await screen.findByRole("heading", { name: "Routes (0+)" })).toBeDefined();
    expect(screen.getByText(/stops at the shared list row cap/)).toBeDefined();
    expect(screen.queryByText("No routes attach to this gateway")).toBeNull();
  });

  it("says so when the routes could not be listed", async () => {
    listCustomResource.mockResolvedValue({ error: "forbidden" });
    render(<GatewayApiBody object={GATEWAY} context="prod-eu" />);
    expect(await screen.findByText(/forbidden/)).toBeDefined();
  });
});

describe("a route's pane", () => {
  it("names each parent with its verdict, and opens the gateway", async () => {
    render(<GatewayApiBody object={ROUTE} context="prod-eu" />);
    const row = screen.getByRole("row", { name: /edge/ });
    expect(within(row).getByText("False")).toBeDefined();
    await userEvent.click(within(row).getByRole("button", { name: "infra/edge" }));
    expect(lastRoute()).toBe(`/k/prod-eu/${encodeURIComponent(`${G}/Gateway`)}/infra/edge`);
  });

  it("asks whether the parent gateway can be read, on this cluster", () => {
    render(<GatewayApiBody object={ROUTE} context="prod-eu" />);
    expect(useAccess).toHaveBeenCalledWith("prod-eu", [
      { verb: "get", group: G, resource: "gateways", namespace: "infra", name: "edge" },
    ]);
  });

  it("names a parent gateway the reader may not read without offering to open it (#1104)", () => {
    // HTTPRoute-only RBAC: the route says everything the pane shows about its
    // parent, and a link would only open a 403.
    useAccess.mockReturnValue(deniedAccess);
    render(<GatewayApiBody object={ROUTE} context="prod-eu" />);
    const row = screen.getByRole("row", { name: /edge/ });
    expect(within(row).queryByRole("button", { name: "infra/edge" })).toBeNull();
    expect(within(row).getByText("infra/edge")).toBeDefined();
    // Said in words, not only by the link going missing.
    const note = within(row).getByText("no read access");
    expect(note.getAttribute("title")).toBe("You don't have permission to get gateways in infra");
    // What the route itself carries is still there.
    expect(within(row).getByText("False")).toBeDefined();
  });

  it("keeps the link while the answer is not in, or the check itself failed", () => {
    // A failed check is not a denial (access.ts), so it must not read as one.
    useAccess.mockReturnValue(unknownAccess);
    render(<GatewayApiBody object={ROUTE} context="prod-eu" />);
    expect(screen.getByRole("button", { name: "infra/edge" })).toBeDefined();
    expect(screen.queryByText("no read access")).toBeNull();
  });

  it("shows what each rule matches and opens a Service backend", async () => {
    render(<GatewayApiBody object={ROUTE} context="prod-eu" />);
    expect(screen.getByText("PathPrefix /api")).toBeDefined();
    await userEvent.click(screen.getByRole("button", { name: "api:8080" }));
    expect(lastRoute()).toBe("/k/prod-eu/Service/store/api");
  });

  it("names a backend that is not a Service without offering to open it", () => {
    render(<GatewayApiBody object={ROUTE} context="prod-eu" />);
    expect(screen.getByText("ServiceImport/far")).toBeDefined();
    expect(screen.queryByRole("button", { name: /far/ })).toBeNull();
  });

  it("does not open a Gateway of another API group as the Gateway API's own", () => {
    const foreign: K8sObject = {
      ...ROUTE,
      spec: { parentRefs: [{ group: "example.com", kind: "Gateway", name: "edge", namespace: "infra" }] },
      status: {},
    };
    render(<GatewayApiBody object={foreign} context="prod-eu" />);
    expect(screen.getByText("Gateway.example.com/infra/edge")).toBeDefined();
    expect(screen.queryByRole("button", { name: "infra/edge" })).toBeNull();
  });

  it("writes resource values in the UI font, on one line (design.md)", async () => {
    const { container } = render(<GatewayApiBody object={ROUTE} context="prod-eu" />);
    const gateway = render(<GatewayApiBody object={GATEWAY} context="prod-eu" />);
    await gateway.findByRole("heading", { name: "Routes (1)" });
    for (const root of [container, gateway.container]) {
      expect(root.querySelectorAll(".font-mono")).toHaveLength(0);
    }
    expect(screen.getByText("PathPrefix /api").className).toContain("whitespace-nowrap");
  });

  it("asks the cluster for nothing", () => {
    render(<GatewayApiBody object={ROUTE} context="prod-eu" />);
    expect(listCrds).not.toHaveBeenCalled();
  });
});

describe("a kind with nothing of its own to show", () => {
  it("adds no block", () => {
    const { container } = render(
      <GatewayApiBody object={{ kind: "ReferenceGrant", metadata: { name: "g", namespace: "store" } }} context="prod-eu" />,
    );
    expect(container.textContent).toBe("");
  });
});
