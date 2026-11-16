import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ClusterContext, CrdRef, K8sObject } from "@srelens/core";

const { listCrds, listCustomResource } = vi.hoisted(() => ({
  listCrds: vi.fn(),
  listCustomResource: vi.fn(),
}));

vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  listCrds,
  listCustomResource,
}));

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
      ["HTTPRoute", null, 8],
      ["GRPCRoute", null, 8],
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
    expect(await screen.findByRole("heading", { name: "Routes (1)" })).toBeDefined();
    expect(screen.getByText("local")).toBeDefined();
    expect(screen.getByText(/Could not list GRPCRoute: forbidden/)).toBeDefined();
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
