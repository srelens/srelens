import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ClusterContext, CrdRef } from "@srelens/core";
import { Nav } from "./Nav";
import { currentWorkspace, openTab, setState } from "../lib/tabsStore";
import { defaultState } from "../lib/tabs";
import { resetView, setLink } from "../lib/workspace";

// The CRD list is the one thing here that talks to a cluster. Mocked at the
// module boundary — partially, so `RESOURCE_LABELS` and the rest of core stay
// real and the tree is labelled the way the app labels it.
const { listCrds } = vi.hoisted(() => ({ listCrds: vi.fn() }));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  listCrds,
}));

const ctx = (name: string): ClusterContext => ({
  name,
  stableId: `id:${name}`,
  cluster: name,
  server: "https://example",
  isCurrent: false,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
});

const PROD = ctx("prod-eu");

const CERTS: CrdRef = {
  name: "certificates.cert-manager.io",
  group: "cert-manager.io",
  version: "v1",
  kind: "Certificate",
  plural: "certificates",
  namespaced: true,
};

beforeEach(() => {
  setState(defaultState([PROD]));
  resetView();
  vi.clearAllMocks();
  listCrds.mockResolvedValue({ crds: [] });
});

const tabFor = (route: string) => currentWorkspace().tabs.find((t) => t.route === route);

describe("Nav", () => {
  it("keeps a tab per kind, so clicking four leaves four", async () => {
    // The property, not the flag. Under the preview pattern each click
    // replaced the last, so comparing four kinds left one tab — which is the
    // opposite of what a tab strip is for.
    render(<Nav contexts={[PROD]} />);

    for (const kind of ["Pods", "Deployments", "Services", "Nodes"]) {
      await userEvent.click(await screen.findByRole("treeitem", { name: kind }));
    }

    const routes = currentWorkspace().tabs.map((t) => t.route);
    expect(routes).toContain("/k/pods");
    expect(routes).toContain("/k/deployments");
    expect(routes).toContain("/k/services");
    expect(routes).toContain("/k/nodes");
  });

  it("lists a built-in kind and opens it in a tab of its own", async () => {
    render(<Nav contexts={[PROD]} />);

    await userEvent.click(await screen.findByRole("treeitem", { name: "Pods" }));

    const tab = tabFor("/k/pods");
    // Not a preview: a second kind must not replace the first.
    expect(tab?.preview).toBeFalsy();
    expect(tab?.sub).toBe("prod-eu");
    expect(currentWorkspace().activeId).toBe(tab?.id);
  });

  it("says so when no cluster is in focus", () => {
    setState(defaultState([]));
    render(<Nav contexts={[]} />);

    expect(screen.getByText("No cluster selected")).toBeTruthy();
    expect(screen.queryByRole("tree")).toBeNull();
  });

  it("shows a discovered CRD under Custom resources, by API group", async () => {
    listCrds.mockResolvedValue({ crds: [CERTS] });
    render(<Nav contexts={[PROD]} />);

    // Both folds start shut: a cluster with a few operators would otherwise
    // bury the built-in kinds under a few hundred custom ones.
    await userEvent.click(await screen.findByRole("treeitem", { name: "Custom resources" }));
    await userEvent.click(await screen.findByRole("treeitem", { name: "cert-manager.io" }));

    expect(await screen.findByRole("treeitem", { name: "Certificate" })).toBeTruthy();
    expect(listCrds).toHaveBeenCalledWith("prod-eu");
  });

  it("opens an app screen from the Investigate group", async () => {
    render(<Nav contexts={[PROD]} />);

    await userEvent.click(await screen.findByRole("treeitem", { name: "Incidents" }));

    expect(tabFor("/incidents")?.preview).toBeFalsy();
  });

  it("names the cluster and how it is linked", async () => {
    setLink(PROD.stableId, "connected");
    render(<Nav contexts={[PROD]} />);

    expect(await screen.findByText("prod-eu")).toBeTruthy();
    expect(screen.getByText("Connected")).toBeTruthy();
  });

  it("marks the node whose route the active tab is on", async () => {
    openTab("/k/deployments", { clusterName: PROD.name });
    render(<Nav contexts={[PROD]} />);

    const row = await screen.findByRole("treeitem", { name: "Deployments" });
    expect(row.getAttribute("aria-selected")).toBe("true");
    expect((await screen.findByRole("treeitem", { name: "Pods" })).getAttribute("aria-selected")).toBe("false");
  });

  it("opens no tab for a group", async () => {
    render(<Nav contexts={[PROD]} />);
    const before = currentWorkspace().tabs.length;

    await userEvent.click(await screen.findByRole("treeitem", { name: "Workloads" }));

    expect(currentWorkspace().tabs).toHaveLength(before);
  });

  it("filters the tree by the sidebar's search box", async () => {
    render(<Nav contexts={[PROD]} />);
    await screen.findByRole("treeitem", { name: "Pods" });

    await userEvent.type(screen.getByRole("searchbox", { name: "Filter resources" }), "secre");

    expect(screen.getByRole("treeitem", { name: "Secrets" })).toBeTruthy();
    expect(screen.queryByRole("treeitem", { name: "Pods" })).toBeNull();
  });

  it("keeps the whole tree when CRD discovery fails, and retries from inside Custom resources", async () => {
    listCrds.mockResolvedValue({ error: "crds forbidden" });
    render(<Nav contexts={[PROD]} />);

    // The built-ins are not RBAC-gated the way CRD discovery is: an ordinary
    // user who cannot list CRDs must still get Pods and the rest of the tree.
    expect(await screen.findByRole("treeitem", { name: "Pods" })).toBeTruthy();

    await userEvent.click(await screen.findByRole("treeitem", { name: "Custom resources" }));
    const retry = await screen.findByRole("treeitem", { name: "Custom resources unavailable — retry" });

    const before = currentWorkspace().tabs.length;
    await userEvent.click(retry);

    expect(listCrds).toHaveBeenCalledTimes(2);
    expect(currentWorkspace().tabs).toHaveLength(before);
  });

  it("asks the new cluster for its CRDs when the cluster changes", async () => {
    const other = ctx("staging");
    const { rerender } = render(<Nav contexts={[PROD, other]} />);
    await screen.findByRole("treeitem", { name: "Pods" });

    setState(defaultState([other]));
    rerender(<Nav contexts={[PROD, other]} />);

    await vi.waitFor(() => expect(listCrds).toHaveBeenCalledWith("staging"));
  });

  it("stays folded shut across a remount once the user has closed every group", async () => {
    const { unmount } = render(<Nav contexts={[PROD]} />);
    await screen.findByRole("treeitem", { name: "Pods" });

    for (const label of ["Cluster", "Workloads", "Config", "Network", "Storage", "Access control", "Investigate"]) {
      await userEvent.click(screen.getByRole("treeitem", { name: label }));
    }
    expect(screen.queryByRole("treeitem", { name: "Pods" })).toBeNull();

    unmount();
    render(<Nav contexts={[PROD]} />);

    // A remount must not read "every group closed" as "nothing has been
    // seeded yet" and reopen all six — that is the state the user just put
    // the sidebar in on purpose.
    expect(screen.queryByRole("treeitem", { name: "Pods" })).toBeNull();
  });
});
