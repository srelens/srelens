import { describe, expect, it, vi } from "vitest";
vi.mock("../transport/transport", () => ({
  invokeCapability: vi.fn().mockResolvedValue({}),
}));
import { invokeCapability } from "../transport/transport";
import {
  configureExtensions,
  readExtension,
  extensionRoute,
  parseExtensionRoute,
} from "./extensions";
describe("extension contract", () => {
  it("uses the backend configure and read wire payloads", async () => {
    await configureExtensions({ action: "developerMode", enabled: true });
    expect(invokeCapability).toHaveBeenCalledWith("extensions.configure", {
      action: "developerMode",
      enabled: true,
    });
    await readExtension("org.test.app", 2, "list", "cluster/a", "ns");
    expect(invokeCapability).toHaveBeenCalledWith("extensions.read", {
      id: "org.test.app",
      revision: 2,
      capability: "list",
      context: "cluster/a",
      namespace: "ns",
    });
  });
  it("pins cluster and namespace in route identity", () => {
    const route = extensionRoute("cluster/a", "org.test.app", "page", "ns/a");
    expect(parseExtensionRoute(route)).toEqual({
      context: "cluster/a",
      id: "org.test.app",
      page: "page",
      namespace: "ns/a",
    });
    expect(extensionRoute("other", "org.test.app", "page", "ns/a")).not.toBe(
      route,
    );
    expect(parseExtensionRoute("/extensions/%bad/a/b/")).toBeNull();
  });
});

it("matches contribution kinds by their actual API group", async () => {
  const { contributionKind } = await import("./extensions");
  expect(contributionKind("Namespace")).toBe("/Namespace");
  expect(contributionKind("Deployment", "apps")).toBe("apps/Deployment");
  expect(contributionKind("acme.io/Deployment")).toBe("acme.io/Deployment");
  expect(contributionKind("UnknownCustomKind", "example.io")).toBe("example.io/UnknownCustomKind");
});

it("matches explicit API identity, including custom and unmapped built-in kinds", async () => {
  const { contributionKind } = await import("./extensions");
  expect(contributionKind("Application", "argoproj.io")).toBe("argoproj.io/Application");
  expect(contributionKind("ResourceQuota", "")).toBe("/ResourceQuota");
  expect(contributionKind("Deployment", "example.io")).toBe("example.io/Deployment");
});

it("uses backend catalog payloads without sending URLs or connecting a cluster", async () => {
  const { listExtensionCatalog, reviewCatalogExtension } = await import("./extensions");
  await listExtensionCatalog(true);
  expect(invokeCapability).toHaveBeenCalledWith("extensions.catalog", { refresh: true });
  await reviewCatalogExtension("org.srelens.flux", "abc");
  expect(invokeCapability).toHaveBeenCalledWith("extensions.catalogManifest", { id: "org.srelens.flux", sha256: "abc" });
});
