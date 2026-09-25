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
  contextKey: "key-prod",
  clusters: [],
  workspaces: [],
  openTab: vi.fn(),
  setActiveCluster: vi.fn(),
  switchWorkspace: vi.fn(),
  onToggleTheme: vi.fn(),
  openAction: vi.fn(),
  openResource: vi.fn(),
  apps: (contextKey) => (contextKey === "key-prod" ? [app] : []),
  hostContext: (contextKey) => (contextKey === "key-prod" ? "srelens-context:/kube/prod#prod" : undefined),
  openAppAction: vi.fn(),
  ...over,
});
const helmRelease = extensionClusterResourceRoute("key-prod", manifest.id, "helmreleases", "team", "web");

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
    expect(d.openTab).toHaveBeenCalledWith(extensionClusterRoute("key-prod", manifest.id, "helmreleases"), { clusterName: "prod-eu" });
  });

  it("opens a page of each of two contexts that share a stable ID in a tab of its own (#695)", () => {
    // `/kube/a` declaring `b#c` and `/kube/a#b` declaring `c` share `/kube/a#b#c`.
    const routes = [["/kube/a#b%23c", "b#c"], ["/kube/a%23b#c", "c"]].map(([contextKey, context]) => {
      const d = deps("/settings", { contextKey, context, apps: () => [app] });
      commandsFor(d).find((c) => c.label === "Flux: Open Helm releases")!.run();
      expect(vi.mocked(d.openTab).mock.calls[0][1]).toEqual({ clusterName: context });
      return vi.mocked(d.openTab).mock.calls[0][0];
    });
    expect(routes).toEqual([
      "/extension-contexts/%2Fkube%2Fa%23b%2523c/org.srelens.flux/helmreleases/",
      "/extension-contexts/%2Fkube%2Fa%2523b%23c/org.srelens.flux/helmreleases/",
    ]);
  });

  it("offers nothing from an app that is not enabled on the cluster in focus", () => {
    expect(commandsFor(deps("/settings", { contextKey: "key-dev" })).some((c) => c.group === "Apps")).toBe(false);
    expect(commandsFor(deps("/settings", { contextKey: undefined })).some((c) => c.group === "Apps")).toBe(false);
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
      request: { id: manifest.id, capability: "helmreleases", context: "srelens-context:/kube/prod#prod", namespace: "team", name: "web", action: "helmreleases-reconcile" },
    });
    expect(d.openTab).not.toHaveBeenCalled();
  });

  it("reads the resource's cluster from its route, not from the rail", () => {
    // The resource tab pins its cluster; the rail may have moved since.
    const d = deps(helmRelease, { contextKey: "key-dev", context: "dev" });
    const reconcile = commandsFor(d).find((c) => c.group === "Action");
    expect(reconcile?.label).toBe("Flux: Reconcile Helm release");
    reconcile!.run();
    expect(vi.mocked(d.openAppAction!).mock.calls[0][0].request.context).toBe("srelens-context:/kube/prod#prod");
  });

  it("offers no action on a cluster the host lists without a pinned ID to ask it by", () => {
    expect(commandsFor(deps(helmRelease, { hostContext: () => undefined })).some((c) => c.group === "Action")).toBe(false);
  });

  it("offers no action on a tab opened before, whose route names its cluster by stable ID", () => {
    // That ID may be two contexts'; the tab says which only once its page is opened again.
    const legacy = "/extension-clusters/key-prod/org.srelens.flux/helmreleases/team/web";
    expect(commandsFor(deps(legacy)).some((c) => c.group === "Action")).toBe(false);
  });

  it("offers a Kustomization's reconcile on a Kustomization, not on a Helm release", () => {
    const route = extensionClusterResourceRoute("key-prod", manifest.id, "kustomizations", "team", "apps");
    expect(commandsFor(deps(route)).filter((c) => c.group === "Action").map((c) => c.label)).toEqual(["Flux: Reconcile Kustomization"]);
  });
});
