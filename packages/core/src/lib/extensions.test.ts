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
  expect(contributionKind("Deployment")).toBe("apps/Deployment");
  expect(contributionKind("acme.io/Deployment")).toBe("acme.io/Deployment");
  expect(contributionKind("UnknownCustomKind")).toBe("");
});
it("pins Freelens broker scope after the frame request fields", async () => {
  const { readFreelensExtension } = await import("./extensions");
  await readFreelensExtension("org.freelensapp.fluxcd", 3, "staging", {
    operation: "events",
    context: "prod",
    revision: 99,
    id: "other",
  });
  expect(invokeCapability).toHaveBeenCalledWith("extensions.freelensRead", {
    operation: "events",
    id: "org.freelensapp.fluxcd",
    revision: 3,
    context: "staging",
  });
});
