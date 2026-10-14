import { beforeEach, describe, expect, it } from "vitest";
import { act, render, screen } from "@testing-library/react";
import type { ClusterContext } from "@srelens/core";
import { extensionClusterResourceRoute, extensionClusterRoute, extensionOperationRoute, extensionRoute } from "@srelens/core";
import { resetContexts, setContexts } from "../lib/clusters";
import { detailRoute, editRoute, newRoute } from "../lib/detailRoute";
import { defaultMark, loadMarks, setMark } from "../lib/marks";
import { defaultState } from "../lib/tabs";
import { createWorkspace, setState, setWorkspaceProtected } from "../lib/tabsStore";
import { ProtectedStrip, protectedStripCluster } from "./ProtectedStrip";

const ctx = (name: string): ClusterContext => ({
  name, stableId: `${name}-id`, key: `${name}-id`, cluster: name, server: `https://${name}`,
  sourceFile: "/config", authKind: "token", isCurrent: false,
});
const prod = ctx("prod-eu");
const staging = ctx("staging");
const contexts = [prod, staging];

let production: string;
beforeEach(() => {
  loadMarks();
  resetContexts();
  setState(defaultState([prod, staging]));
  setContexts([prod, staging]);
  production = createWorkspace("Production", [prod.stableId]);
  setWorkspaceProtected(production, true);
});

describe("ProtectedStrip", () => {
  it("says the cluster is protected, by its mark name and its workspace, as a note", () => {
    setMark(prod.stableId, { ...defaultMark("prod-eu"), name: "Production EU" });
    render(<ProtectedStrip contextName="prod-eu" />);
    expect(screen.getByRole("note").textContent).toBe(
      "Production EU is protected (workspace Production). Changes ask you to type its name.",
    );
  });

  it("draws nothing for an unprotected cluster, an unknown one, or none", () => {
    const { container, rerender } = render(<ProtectedStrip contextName="staging" />);
    expect(container.innerHTML).toBe("");
    rerender(<ProtectedStrip contextName="nope" />);
    expect(container.innerHTML).toBe("");
    rerender(<ProtectedStrip contextName={null} />);
    expect(container.innerHTML).toBe("");
  });

  it("goes when the workspace stops being protected", () => {
    render(<ProtectedStrip contextName="prod-eu" />);
    expect(screen.getByRole("note")).toBeTruthy();
    act(() => setWorkspaceProtected(production, false));
    expect(screen.queryByRole("note")).toBeNull();
  });
});

describe("protectedStripCluster", () => {
  it("is the cluster pinned in a detail, edit or new route, whatever the rail is on", () => {
    expect(protectedStripCluster(detailRoute("Pod", "default", "web", { cluster: "prod-eu" }), "staging", contexts)).toBe("prod-eu");
    expect(protectedStripCluster(editRoute("Deployment", "default", "api", { cluster: "prod-eu" }), "staging", contexts)).toBe("prod-eu");
    expect(protectedStripCluster(newRoute("prod-eu"), "staging", contexts)).toBe("prod-eu");
  });

  it("is the rail's cluster for any other cluster-scoped route, the Agent's included", () => {
    expect(protectedStripCluster("/k/pods", "staging", contexts)).toBe("staging");
    expect(protectedStripCluster("/overview", "staging", contexts)).toBe("staging");
    // The agent acts on the rail's cluster, so its tab is about that cluster.
    expect(protectedStripCluster("/agent", "staging", contexts)).toBe("staging");
    // A detail route an earlier build saved, with no cluster in it, follows the rail.
    expect(protectedStripCluster("/k/Pod/default/web", "staging", contexts)).toBe("staging");
  });

  it("is no cluster for an app route, or when the rail has none", () => {
    for (const route of ["/", "/settings", "/connections", "/toolbox"]) {
      expect(protectedStripCluster(route, "staging", contexts), route).toBeNull();
    }
    expect(protectedStripCluster("/k/pods", null, contexts)).toBeNull();
  });

  // An app's page names its cluster in its route, independent of the rail
  // (ExtensionPage), so its strip is about that cluster. The route's own name
  // for it is a key, a stable ID or a display name, and only a name is
  // something `protectionFor` resolves.
  describe("for an app route that names its own cluster", () => {
    // Keys that are not the stable IDs, as a real key is when a name holds a `#`.
    const keyed = [{ ...prod, key: "prod-eu-key" }, { ...staging, key: "staging-key" }];

    it("is that cluster, not the rail's, whichever way the route names it", () => {
      const at = (route: string) => protectedStripCluster(route, "staging", keyed);
      expect(at(extensionClusterRoute("prod-eu-key", "ext", "overview"))).toBe("prod-eu");
      expect(at(extensionClusterResourceRoute("prod-eu-key", "ext", "overview", "default", "web"))).toBe("prod-eu");
      expect(at(extensionOperationRoute("prod-eu-key", "ext", 1, "scan"))).toBe("prod-eu");
      // Opened before keys: by stable ID, then by display name.
      expect(at(extensionClusterRoute("prod-eu-id", "ext", "overview").replace("/extension-contexts/", "/extension-clusters/"))).toBe("prod-eu");
      expect(at(extensionRoute("prod-eu", "ext", "overview"))).toBe("prod-eu");
    });

    it("is no cluster, and not the rail's, when the route's cluster is not listed or not one context's", () => {
      const at = (route: string, list = keyed) => protectedStripCluster(route, "staging", list);
      expect(at(extensionClusterRoute("gone-key", "ext", "overview"))).toBeNull();
      expect(at(extensionOperationRoute("gone-key", "ext", 1, "scan"))).toBeNull();
      // Two contexts sharing a stable ID: the page does not open for either (ExtensionPage).
      const twins = [prod, { ...prod, name: "twin", key: "twin-key" }];
      expect(at(extensionClusterRoute("prod-eu-id", "ext", "overview").replace("/extension-contexts/", "/extension-clusters/"), twins)).toBeNull();
    });
  });
});

describe("the strip over an app tab", () => {
  const over = (route: string, rail: string) => <ProtectedStrip contextName={protectedStripCluster(route, rail, contexts)} />;

  it("is about the cluster the tab names: shown with the rail elsewhere, and not borrowed from the rail", () => {
    // Protected cluster's page, rail on an unprotected one.
    const { container, rerender } = render(over(extensionClusterRoute(prod.key, "ext", "overview"), "staging"));
    expect(screen.getByRole("note").textContent).toContain("prod-eu is protected");
    // The reverse: an unprotected cluster's page, rail on the protected one.
    rerender(over(extensionClusterRoute(staging.key, "ext", "overview"), "prod-eu"));
    expect(container.innerHTML).toBe("");
  });
});
