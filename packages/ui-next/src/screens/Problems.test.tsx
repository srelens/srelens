import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClusterContext, EventSummary, NodeSummary, PodSummary } from "@srelens/core";
import type { EventRow } from "../lib/kinds/events";
import type { PodRow } from "../lib/kinds/columns";
import type { ResourceList } from "../lib/resourceList";

const NOW = Date.parse("2026-11-04T12:00:00Z");
const context: ClusterContext = {
  stableId: "prod-id",
  key: "prod-key",
  name: "prod",
  cluster: "prod",
  server: "https://prod.example",
  isCurrent: false,
  sourceFile: "/mock/config",
  authKind: "client certificate",
};

const mocks = vi.hoisted(() => ({
  activeContext: vi.fn(),
  resourceList: vi.fn(),
  openTab: vi.fn(),
  useTabs: vi.fn(),
  useNow: vi.fn(),
  namespaceOptions: vi.fn(),
}));

vi.mock("../lib/clusters", async (original) => ({
  ...(await original<typeof import("../lib/clusters")>()),
  getKubeconfigFiles: () => [],
  useActiveContext: mocks.activeContext,
}));
vi.mock("../lib/resourceList", async (original) => ({
  ...(await original<typeof import("../lib/resourceList")>()),
  useResourceList: mocks.resourceList,
}));
vi.mock("../lib/tabsStore", async (original) => ({
  ...(await original<typeof import("../lib/tabsStore")>()),
  openTab: mocks.openTab,
  useTabs: mocks.useTabs,
}));
vi.mock("../lib/ageCell", async (original) => ({
  ...(await original<typeof import("../lib/ageCell")>()),
  useNow: mocks.useNow,
}));
vi.mock("@srelens/core/react", async (original) => ({
  ...(await original<typeof import("@srelens/core/react")>()),
  useNamespaceOptions: mocks.namespaceOptions,
}));
vi.mock("../lib/workspace", async (original) => ({
  ...(await original<typeof import("../lib/workspace")>()),
  useNamespaces: () => [],
  useFallbackNamespace: () => undefined,
}));
vi.mock("./collections/NamespaceScope", () => ({
  NamespaceScopePicker: () => <button type="button">All namespaces</button>,
  CollectionScopeNotice: () => null,
}));

import { detailRoute } from "../lib/detailRoute";
import { logsRoute } from "./Logs";
import { Problems } from "./Problems";

const pod = (name: string, status: string, over: Partial<PodSummary> = {}): PodRow => ({
  name,
  namespace: "checkout",
  phase: "Running",
  status,
  ready: "0/1",
  restarts: 4,
  node: "worker-1",
  image: "example/web:1",
  age: "3m",
  created: "2026-11-04T11:57:00Z",
  ...over,
} as PodRow);

const node = (name: string, over: Partial<NodeSummary> = {}): NodeSummary => ({
  name,
  status: "Ready",
  unschedulable: false,
  taints: 0,
  taintDetails: [],
  version: "v1.34.0",
  roles: "worker",
  age: "3d",
  created: "2026-11-01T12:00:00Z",
  allocatableCpuMillicores: 4_000,
  allocatableMemoryMiB: 16_384,
  allocatablePods: 110,
  instanceType: "m7i.xlarge",
  ...over,
});

const event = (object: string, over: Partial<EventSummary> = {}): EventRow => ({
  name: `checkout/${object}.17b`,
  namespace: "checkout",
  type: "Warning",
  reason: "ReconcileFailed",
  object,
  message: "The controller could not reconcile this object",
  created: "2026-11-04T11:58:00Z",
  age: "2m",
  count: 2,
  ...over,
} as EventRow);

function listed<Row>(rows: Row[], over: Partial<ResourceList<Row>> = {}): ResourceList<Row> {
  return {
    rows,
    status: rows.length > 0 ? "ready" : "empty",
    namespaceFailures: [],
    stale: false,
    watch: "live",
    reload: vi.fn(),
    ...over,
  };
}

let lists: Record<string, ResourceList<never>>;

function draw() {
  return render(<Problems route="/problems" />);
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.activeContext.mockReturnValue(context);
  mocks.useTabs.mockReturnValue({ workspace: { pausedClusters: [] } });
  mocks.useNow.mockReturnValue(NOW);
  mocks.namespaceOptions.mockReturnValue({ namespaces: ["checkout", "platform"], items: [], complete: true, scope: "all", error: "" });
  lists = {
    pods: listed([pod("web-7d4b", "CrashLoopBackOff"), pod("healthy", "Running", { ready: "1/1" })]) as ResourceList<never>,
    nodes: listed([node("worker-2", { unschedulable: true }), node("worker-1")]) as ResourceList<never>,
    events: listed([event("Deployment/api", { objectApiVersion: "acme.io/v1" })]) as ResourceList<never>,
  };
  mocks.resourceList.mockImplementation((_context: string, kind: string) => lists[kind]);
});

describe("Problems", () => {
  it("shows one live triage table, excludes healthy resources, and filters by source and text", async () => {
    draw();

    expect(screen.getByRole("heading", { name: "Problems" })).toBeTruthy();
    expect(screen.getByText("web-7d4b")).toBeTruthy();
    expect(screen.getByText("worker-2")).toBeTruthy();
    expect(screen.getByText("api")).toBeTruthy();
    expect(screen.queryByText("healthy")).toBeNull();
    expect(screen.queryByText("worker-1")).toBeNull();
    expect(screen.getByText("3 current problems")).toBeTruthy();

    await userEvent.click(screen.getByRole("tab", { name: "Pods" }));
    expect(screen.getByText("web-7d4b")).toBeTruthy();
    expect(screen.queryByText("worker-2")).toBeNull();
    expect(screen.queryByText("api")).toBeNull();

    await userEvent.click(screen.getByRole("tab", { name: "All" }));
    await userEvent.type(screen.getByRole("searchbox", { name: "Filter problems" }), "cordon");
    expect(screen.getByText("worker-2")).toBeTruthy();
    expect(screen.queryByText("web-7d4b")).toBeNull();
  });

  it("opens details with the event's custom group and offers pod logs as a separate quick action", async () => {
    draw();

    await userEvent.click(screen.getByRole("button", { name: "Open acme.io/Deployment checkout/api" }));
    expect(mocks.openTab).toHaveBeenLastCalledWith(
      detailRoute("Deployment", "checkout", "api", { cluster: "prod", group: "acme.io" }),
      { clusterName: "prod" },
    );

    await userEvent.click(screen.getByRole("button", { name: "Follow logs for Pod checkout/web-7d4b" }));
    expect(mocks.openTab).toHaveBeenLastCalledWith(
      logsRoute("Pod", "checkout", "web-7d4b"),
      { clusterName: "prod" },
    );
  });

  it("keeps partial read failures visible beside the rows that did answer, and retries each source", async () => {
    const podReload = vi.fn();
    lists.pods = listed([], { status: "error", error: "pods forbidden", reload: podReload }) as ResourceList<never>;
    draw();

    expect(screen.getByText("Pods could not be checked")).toBeTruthy();
    expect(screen.getByText("worker-2")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Retry pods" }));
    expect(podReload).toHaveBeenCalledTimes(1);
  });

  it("never calls a complete check clean when one source failed", () => {
    lists.pods = listed([], { status: "error", error: "pods forbidden" }) as ResourceList<never>;
    lists.nodes = listed([]) as ResourceList<never>;
    lists.events = listed([]) as ResourceList<never>;
    draw();

    expect(screen.queryByText("No problems detected")).toBeNull();
    expect(screen.getByText("Pods could not be checked")).toBeTruthy();
  });

  it("shows a truthful clean state only after all three sources answered", () => {
    lists.pods = listed([]) as ResourceList<never>;
    lists.nodes = listed([]) as ResourceList<never>;
    lists.events = listed([]) as ResourceList<never>;
    draw();

    expect(screen.getByText("No problems detected")).toBeTruthy();
    expect(screen.getByText(/pods, nodes, and warning events/)).toBeTruthy();
  });

  it("refreshes all three sources from one visible action", async () => {
    draw();
    await userEvent.click(screen.getByRole("button", { name: "Refresh problems" }));
    expect(Object.values(lists).every((list) => vi.mocked(list.reload).mock.calls.length === 1)).toBe(true);
  });

  it("does not let an action button bubble into the row's open gesture", () => {
    draw();
    const logs = screen.getByRole("button", { name: "Follow logs for Pod checkout/web-7d4b" });
    fireEvent.click(logs);
    expect(mocks.openTab).toHaveBeenCalledTimes(1);
    expect(within(logs.closest("tr")!).getByText("web-7d4b")).toBeTruthy();
  });
});
