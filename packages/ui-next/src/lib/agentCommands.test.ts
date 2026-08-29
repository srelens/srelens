// jsdom, not node: `logsRoute` is imported from `screens/Logs.tsx`, whose own
// graph reaches the Terminals screen through `@xterm/addon-fit`, a UMD bundle
// that reads `self` while it evaluates — see `agentSuggestions.test.ts` for
// the same note against the same hazard.
import { describe, it, expect, vi } from "vitest";
import { commandsFor, matchCommands, type CommandDeps } from "./agentCommands";

const base: Omit<CommandDeps, "route"> = {
  context: "prod-eu",
  clusters: [{ id: "id-stage", name: "stage-eu" }],
  workspaces: [{ id: "w2", name: "Platform" }],
  openTab: vi.fn(),
  setActiveCluster: vi.fn(),
  switchWorkspace: vi.fn(),
  onToggleTheme: vi.fn(),
  openAction: vi.fn(),
};

describe("the / palette", () => {
  it("offers Action only where the route names a resource", () => {
    const onDetail = commandsFor({ ...base, route: "/k/Deployment/checkout/api" });
    expect(onDetail.some((c) => c.group === "Action")).toBe(true);
    const onSettings = commandsFor({ ...base, route: "/settings" });
    expect(onSettings.some((c) => c.group === "Action")).toBe(false);
  });

  it("offers Go only where the route names a resource", () => {
    // Ruling G: `Go` takes the same subject `Action` does — it is not built
    // from the route table, so it must vanish on a route with no resource
    // exactly as `Action` does, not merely "usually agree with it".
    const onDetail = commandsFor({ ...base, route: "/k/Deployment/checkout/api" });
    expect(onDetail.some((c) => c.group === "Go")).toBe(true);
    const onSettings = commandsFor({ ...base, route: "/settings" });
    expect(onSettings.some((c) => c.group === "Go")).toBe(false);
  });

  it("does not offer a rollback, because nothing behind it exists", () => {
    const all = commandsFor({ ...base, route: "/k/Deployment/checkout/api" });
    expect(all.some((c) => /roll ?back/i.test(c.label))).toBe(false);
  });

  it("gates Go commands by the kind's own KindActions, not by kind name alone", () => {
    // A Deployment offers logs but not shell or forward (`descriptors.ts`);
    // a Pod offers all three. A module that drew every Go command for any
    // resource would pass the "present on a resource route" test above while
    // still being wrong about which resource offers what.
    const onDeployment = commandsFor({ ...base, route: "/k/Deployment/checkout/api" });
    expect(onDeployment.some((c) => c.id === "logs")).toBe(true);
    expect(onDeployment.some((c) => c.id === "shell")).toBe(false);
    expect(onDeployment.some((c) => c.id === "forward")).toBe(false);

    const onPod = commandsFor({ ...base, route: "/k/Pod/checkout/api-0" });
    expect(onPod.some((c) => c.id === "shell")).toBe(true);
    expect(onPod.some((c) => c.id === "forward")).toBe(true);
  });

  it("routes a destructive action through openAction, never straight to the capability", () => {
    const openAction = vi.fn();
    const all = commandsFor({ ...base, route: "/k/Deployment/checkout/api", openAction });
    all.find((c) => c.id === "restart")?.run();
    expect(openAction).toHaveBeenCalledWith(
      expect.objectContaining({ action: "restart", name: "api", namespace: "checkout", context: "prod-eu" }),
    );
  });

  it("marks the destructive ones so the list can show it", () => {
    const all = commandsFor({ ...base, route: "/k/Deployment/checkout/api" });
    expect(all.find((c) => c.id === "restart")?.danger).toBe(true);
    expect(all.find((c) => c.id === "scale")?.danger).toBe(true);
    expect(all.find((c) => c.id === "logs")?.danger).toBeUndefined();
  });

  it("pins the context at build time, not read live off some later call", () => {
    const openAction = vi.fn();
    const first = commandsFor({ ...base, route: "/k/Deployment/checkout/api", context: "prod-eu", openAction });
    // A second build with a different context must not change what the
    // FIRST list's command does when it finally runs.
    commandsFor({ ...base, route: "/k/Deployment/checkout/api", context: "staging", openAction });
    first.find((c) => c.id === "restart")?.run();
    expect(openAction).toHaveBeenCalledWith(expect.objectContaining({ context: "prod-eu" }));
  });

  it("still offers Cluster, Workspace and Toggle theme with no resource in view", () => {
    const onSettings = commandsFor({ ...base, route: "/settings" });
    expect(onSettings.some((c) => c.group === "Cluster")).toBe(true);
    expect(onSettings.some((c) => c.label === "Toggle theme")).toBe(true);
  });

  it("passes both id and name to a Cluster switch, so the strip relabels", () => {
    const setActiveCluster = vi.fn();
    const all = commandsFor({ ...base, route: "/settings", setActiveCluster });
    all.find((c) => c.group === "Cluster")?.run();
    expect(setActiveCluster).toHaveBeenCalledWith("id-stage", "stage-eu");
  });

  it("switches workspace by id", () => {
    const switchWorkspace = vi.fn();
    const all = commandsFor({ ...base, route: "/settings", switchWorkspace });
    all.find((c) => c.group === "Workspace" && c.label !== "Toggle theme")?.run();
    expect(switchWorkspace).toHaveBeenCalledWith("w2");
  });

  it("toggles theme via the injected handler", () => {
    const onToggleTheme = vi.fn();
    const all = commandsFor({ ...base, route: "/settings", onToggleTheme });
    all.find((c) => c.label === "Toggle theme")?.run();
    expect(onToggleTheme).toHaveBeenCalled();
  });

  it("matches on the label, case-insensitively", () => {
    const all = commandsFor({ ...base, route: "/helm" });
    expect(matchCommands(all, "logs").every((c) => /logs/i.test(c.label))).toBe(true);
    const workspaceMatches = matchCommands(all, "PLATFORM");
    expect(workspaceMatches.some((c) => c.label === "Switch to Platform")).toBe(true);
  });

  it("returns every command for an empty query", () => {
    const all = commandsFor({ ...base, route: "/helm" });
    expect(matchCommands(all, "")).toEqual(all);
  });
});
