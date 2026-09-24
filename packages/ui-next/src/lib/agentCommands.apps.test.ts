import { describe, it, expect, vi } from "vitest";
import { extensionClusterResourceRoute, extensionClusterRoute, type ExtensionManifest } from "@srelens/core";
import flux from "../../../../examples/extensions/flux.json";
import { commandsFor, matchCommands, type CommandDeps } from "./agentCommands";

// App-contributed commands (#544): the host names the app, a page command
// opens the page on the cluster in focus, and an action command only asks the
// resource's own tab for its review.
const manifest = flux as unknown as ExtensionManifest;
const app = { id: manifest.id, name: "Flux", manifest };
const deps = (route: string, over: Partial<CommandDeps> = {}): CommandDeps => ({
  route,
  context: "prod-eu",
  clusterId: "stable-prod",
  clusters: [],
  workspaces: [],
  openTab: vi.fn(),
  setActiveCluster: vi.fn(),
  switchWorkspace: vi.fn(),
  onToggleTheme: vi.fn(),
  openAction: vi.fn(),
  openResource: vi.fn(),
  apps: (clusterId) => (clusterId === "stable-prod" ? [app] : []),
  openAppAction: vi.fn(),
  ...over,
});
const helmRelease = extensionClusterResourceRoute("stable-prod", manifest.id, "helmreleases", "team", "web");

describe("app commands in the / palette", () => {
  it("lists the app's page commands under Apps, named for the app", () => {
    const apps = commandsFor(deps("/settings")).filter((c) => c.group === "Apps");
    expect(apps.map((c) => c.label)).toContain("Flux: Open Helm releases");
    expect(apps.every((c) => c.label.startsWith("Flux: "))).toBe(true);
    expect(matchCommands(apps, "helm rel").map((c) => c.label)).toEqual(["Flux: Open Helm releases"]);
  });

  it("opens a page on the cluster in focus, pinned in its route", () => {
    const d = deps("/settings");
    commandsFor(d).find((c) => c.label === "Flux: Open Helm releases")!.run();
    expect(d.openTab).toHaveBeenCalledWith(extensionClusterRoute("stable-prod", manifest.id, "helmreleases"), { clusterName: "prod-eu" });
  });

  it("offers nothing from an app that is not enabled on the cluster in focus", () => {
    expect(commandsFor(deps("/settings", { clusterId: "stable-dev" })).some((c) => c.group === "Apps")).toBe(false);
    expect(commandsFor(deps("/settings", { clusterId: undefined })).some((c) => c.group === "Apps")).toBe(false);
  });

  it("offers an action only on a resource of its kind, and only asks for the review", () => {
    expect(commandsFor(deps("/settings")).some((c) => c.label === "Flux: Reconcile Helm release")).toBe(false);
    const d = deps(helmRelease);
    const actions = commandsFor(d).filter((c) => c.group === "Action");
    expect(actions.map((c) => c.label)).toEqual(["Flux: Reconcile Helm release"]);
    expect(actions[0].danger).toBeUndefined();
    actions[0].run();
    expect(d.openAppAction).toHaveBeenCalledWith({
      route: helmRelease,
      request: { id: manifest.id, capability: "helmreleases", context: "stable-prod", namespace: "team", name: "web", action: "helmreleases-reconcile" },
    });
    expect(d.openTab).not.toHaveBeenCalled();
  });

  it("reads the resource's cluster from its route, not from the rail", () => {
    // The resource tab pins its cluster; the rail may have moved since.
    const d = deps(helmRelease, { clusterId: "stable-dev", context: "dev" });
    const reconcile = commandsFor(d).find((c) => c.group === "Action");
    expect(reconcile?.label).toBe("Flux: Reconcile Helm release");
    reconcile!.run();
    expect(vi.mocked(d.openAppAction!).mock.calls[0][0].request.context).toBe("stable-prod");
  });

  it("offers a Kustomization's reconcile on a Kustomization, not on a Helm release", () => {
    const route = extensionClusterResourceRoute("stable-prod", manifest.id, "kustomizations", "team", "apps");
    expect(commandsFor(deps(route)).filter((c) => c.group === "Action").map((c) => c.label)).toEqual(["Flux: Reconcile Kustomization"]);
  });
});
