import { describe, it, expect, vi } from "vitest";
import {
  toolboxStatus,
  diagnoseContext,
  searchPlugins,
  installKubectl,
  installHelm,
  installKrew,
  installPlugin,
  upgradePlugin,
  removePlugin,
} from "./toolbox";

describe("toolbox lib wrappers", () => {
  it("toolboxStatus unwraps the tools array", async () => {
    const invoke = vi.fn().mockResolvedValue({ tools: [{ name: "kubectl", installed: true }] });
    const r = await toolboxStatus(invoke);
    expect(invoke).toHaveBeenCalledWith("toolbox.status", {});
    expect(r.data).toEqual([{ name: "kubectl", installed: true }]);
  });

  it("diagnoseContext passes the context and returns the report", async () => {
    const report = { context: "dev", items: [] };
    const invoke = vi.fn().mockResolvedValue(report);
    const r = await diagnoseContext("dev", invoke);
    expect(invoke).toHaveBeenCalledWith("toolbox.diagnoseContext", { context: "dev" });
    expect(r.data).toEqual(report);
  });

  it("searchPlugins passes the query and unwraps plugins", async () => {
    const invoke = vi.fn().mockResolvedValue({ plugins: [{ name: "oidc-login", description: "", installed: false }] });
    const r = await searchPlugins("oidc", invoke);
    expect(invoke).toHaveBeenCalledWith("toolbox.searchPlugins", { query: "oidc" });
    expect(r.data?.[0].name).toBe("oidc-login");
  });

  it.each([
    ["installKubectl", installKubectl, "toolbox.installKubectl"],
    ["installHelm", installHelm, "toolbox.installHelm"],
    ["installKrew", installKrew, "toolbox.installKrew"],
  ] as const)("%s invokes %s with no args", async (_name, fn, id) => {
    const invoke = vi.fn().mockResolvedValue({ tool: "x", version: "v1", path: "/p" });
    const r = await fn(invoke);
    expect(invoke).toHaveBeenCalledWith(id, {});
    expect(r.data?.version).toBe("v1");
  });

  it.each([
    ["installPlugin", installPlugin, "toolbox.installPlugin"],
    ["upgradePlugin", upgradePlugin, "toolbox.upgradePlugin"],
    ["removePlugin", removePlugin, "toolbox.removePlugin"],
  ] as const)("%s passes the plugin name to %s", async (_name, fn, id) => {
    const invoke = vi.fn().mockResolvedValue({ plugin: "oidc-login", output: "ok" });
    const r = await fn("oidc-login", invoke);
    expect(invoke).toHaveBeenCalledWith(id, { plugin: "oidc-login" });
    expect(r.data?.plugin).toBe("oidc-login");
  });

  it("maps a thrown error into the result", async () => {
    const invoke = vi.fn().mockRejectedValue(new Error("krew not found"));
    const r = await installKrew(invoke);
    expect(r.error).toContain("krew not found");
    expect(r.data).toBeUndefined();
  });
});
