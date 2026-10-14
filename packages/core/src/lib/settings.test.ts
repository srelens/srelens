import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  loadClusterNamespaces,
  removeClusterNamespace,
  saveClusterNamespaces,
  getDefaultNamespace,
  setDefaultNamespace,
  DEFAULT_WORKSPACE_LAYOUT,
  loadWorkspaceLayout,
  saveWorkspaceLayout,
  contextDisplayName,
  loadContextProfiles,
  saveContextProfiles,
  KUBECONFIG_FILES_CHANGED,
  loadKubeconfigFiles,
  saveKubeconfigFiles,
  loadContextOrder,
  orderContexts,
  saveContextOrder,
  loadUpdateChannel,
  saveUpdateChannel,
  REQUEST_TIMEOUT,
  clampTimeoutSecs,
  getRequestTimeoutSecs,
  setRequestTimeoutSecs,
  loadHiddenColumns,
  saveHiddenColumns,
} from "./settings";

beforeEach(() => localStorage.clear());

describe("request timeout setting", () => {
  it("defaults to the documented default when unset", () => {
    expect(getRequestTimeoutSecs()).toBe(REQUEST_TIMEOUT.DEFAULT);
  });

  it("clamps values to the supported range", () => {
    expect(clampTimeoutSecs(REQUEST_TIMEOUT.MAX + 1)).toBe(REQUEST_TIMEOUT.MAX);
    expect(clampTimeoutSecs(0)).toBe(REQUEST_TIMEOUT.MIN);
    expect(clampTimeoutSecs(-5)).toBe(REQUEST_TIMEOUT.MIN);
    expect(clampTimeoutSecs(15)).toBe(15);
    // A large-cluster budget is now inside the range, not clamped away (#238).
    expect(clampTimeoutSecs(90)).toBe(90);
    expect(clampTimeoutSecs("nonsense")).toBe(REQUEST_TIMEOUT.DEFAULT);
  });

  it("persists and reloads a clamped value", () => {
    expect(setRequestTimeoutSecs(25)).toBe(25);
    expect(getRequestTimeoutSecs()).toBe(25);
    // Out-of-range writes are clamped on the way in.
    expect(setRequestTimeoutSecs(999)).toBe(REQUEST_TIMEOUT.MAX);
    expect(getRequestTimeoutSecs()).toBe(REQUEST_TIMEOUT.MAX);
  });

  it("falls back to the default when stored data is corrupt", () => {
    localStorage.setItem("srelens.requestTimeoutSecs", "{not json");
    expect(getRequestTimeoutSecs()).toBe(REQUEST_TIMEOUT.DEFAULT);
  });
});

describe("settings persistence", () => {
  it("round-trips per-cluster namespaces", () => {
    expect(loadClusterNamespaces()).toEqual({});
    saveClusterNamespaces({ "kind-dev": "kube-system", prod: "monitoring" });
    expect(loadClusterNamespaces()).toEqual({ "kind-dev": "kube-system", prod: "monitoring" });
  });

  it("removes only the deleted cluster's namespace", () => {
    saveClusterNamespaces({ "kind-dev": "kube-system", prod: "monitoring" });
    removeClusterNamespace("prod");
    expect(loadClusterNamespaces()).toEqual({ "kind-dev": "kube-system" });
  });

  it("round-trips the default namespace", () => {
    expect(getDefaultNamespace()).toBe("");
    setDefaultNamespace("monitoring");
    expect(getDefaultNamespace()).toBe("monitoring");
  });

  it("tolerates corrupt storage", () => {
    localStorage.setItem("srelens.clusterNamespaces", "{not json");
    expect(loadClusterNamespaces()).toEqual({});
  });

  it("persists workspace panel widths and bounds invalid values", () => {
    expect(loadWorkspaceLayout()).toEqual(DEFAULT_WORKSPACE_LAYOUT);
    saveWorkspaceLayout({ leftSidebarWidth: 260, rightSidebarWidth: 640 });
    expect(loadWorkspaceLayout()).toEqual({ leftSidebarWidth: 260, rightSidebarWidth: 640 });

    localStorage.setItem(
      "srelens.workspaceLayout",
      JSON.stringify({ leftSidebarWidth: 20, rightSidebarWidth: 2000 }),
    );
    expect(loadWorkspaceLayout()).toEqual({ leftSidebarWidth: 160, rightSidebarWidth: 960 });
  });

  it("persists context names, short labels, colors, and logos", () => {
    const profiles = {
      production: { displayName: "Production EU", shortName: "EU", color: "#dc2626", logo: "custom" as const, logoUrl: "https://example.com/logo.png" },
    };
    saveContextProfiles(profiles);
    expect(loadContextProfiles()).toEqual(profiles);
    expect(contextDisplayName("production", profiles.production)).toBe("Production EU");
    expect(contextDisplayName("staging", undefined)).toBe("staging");
  });

  it("persists and deduplicates additional kubeconfig files", () => {
    saveKubeconfigFiles(["/tmp/a", "/tmp/b", "/tmp/a"]);
    expect(loadKubeconfigFiles()).toEqual(["/tmp/a", "/tmp/b"]);
  });

  it("tells listeners the kubeconfig files in use, even when saving them fails", () => {
    const heard: string[][] = [];
    const listener = (event: Event) => heard.push((event as CustomEvent<string[]>).detail);
    window.addEventListener(KUBECONFIG_FILES_CHANGED, listener);
    try {
      saveKubeconfigFiles(["/tmp/edge.yaml", "/tmp/edge.yaml"]);
      // The caller keeps using the new files whether or not storage took them, so
      // listeners must hear the live list, not what storage still holds.
      const setItem = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
        throw new Error("quota exceeded");
      });
      try {
        saveKubeconfigFiles(["/tmp/edge.yaml", "/tmp/core.yaml"]);
      } finally {
        setItem.mockRestore();
      }
    } finally {
      window.removeEventListener(KUBECONFIG_FILES_CHANGED, listener);
    }
    expect(heard).toEqual([["/tmp/edge.yaml"], ["/tmp/edge.yaml", "/tmp/core.yaml"]]);
    expect(loadKubeconfigFiles()).toEqual(["/tmp/edge.yaml"]);
  });

  it("persists hidden columns per view independently", () => {
    expect(loadHiddenColumns("nodes")).toEqual([]);
    saveHiddenColumns("nodes", ["cpu", "memory", "cpu"]);
    expect(loadHiddenColumns("nodes")).toEqual(["cpu", "memory"]);
    // A different view keeps its own set.
    expect(loadHiddenColumns("pods")).toEqual([]);
    saveHiddenColumns("pods", ["restarts"]);
    expect(loadHiddenColumns("nodes")).toEqual(["cpu", "memory"]);
    expect(loadHiddenColumns("pods")).toEqual(["restarts"]);
  });

  it("returns no hidden columns when storage is corrupt", () => {
    localStorage.setItem("srelens.hiddenColumns", "{not json");
    expect(loadHiddenColumns("nodes")).toEqual([]);
  });

  it("persists context order and appends unordered contexts", () => {
    saveContextOrder(["prod", "dev", "prod"]);
    expect(loadContextOrder()).toEqual(["prod", "dev"]);
    expect(orderContexts([{ name: "dev" }, { name: "new" }, { name: "prod" }], loadContextOrder()))
      .toEqual([{ name: "prod" }, { name: "dev" }, { name: "new" }]);
  });

  it("persists the update channel and defaults to stable", () => {
    expect(loadUpdateChannel()).toBe("stable");
    saveUpdateChannel("dev");
    expect(loadUpdateChannel()).toBe("dev");
  });

  it("falls back to stable for unknown stored channels", () => {
    localStorage.setItem("srelens.updateChannel", "nightly");
    expect(loadUpdateChannel()).toBe("stable");
  });
});
