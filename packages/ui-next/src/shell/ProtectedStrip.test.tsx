import { beforeEach, describe, expect, it } from "vitest";
import { act, render, screen } from "@testing-library/react";
import type { ClusterContext } from "@srelens/core";
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
    expect(protectedStripCluster(detailRoute("Pod", "default", "web", { cluster: "prod-eu" }), "staging")).toBe("prod-eu");
    expect(protectedStripCluster(editRoute("Deployment", "default", "api", { cluster: "prod-eu" }), "staging")).toBe("prod-eu");
    expect(protectedStripCluster(newRoute("prod-eu"), "staging")).toBe("prod-eu");
  });

  it("is the rail's cluster for any other cluster-scoped route, the Agent's included", () => {
    expect(protectedStripCluster("/k/pods", "staging")).toBe("staging");
    expect(protectedStripCluster("/overview", "staging")).toBe("staging");
    // The agent acts on the rail's cluster, so its tab is about that cluster.
    expect(protectedStripCluster("/agent", "staging")).toBe("staging");
    // A detail route an earlier build saved, with no cluster in it, follows the rail.
    expect(protectedStripCluster("/k/Pod/default/web", "staging")).toBe("staging");
  });

  it("is no cluster for an app route, or when the rail has none", () => {
    for (const route of ["/", "/settings", "/connections", "/toolbox"]) {
      expect(protectedStripCluster(route, "staging"), route).toBeNull();
    }
    expect(protectedStripCluster("/k/pods", null)).toBeNull();
  });
});
