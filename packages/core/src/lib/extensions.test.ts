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
    await configureExtensions({ action: "enable", id: "org.test.app", enabled: true });
    expect(invokeCapability).toHaveBeenCalledWith("extensions.configure", {
      action: "enable", id: "org.test.app",
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
  it("requests CRD columns using the host camelCase contract",async()=>{
    await readExtension("org.test.app",2,"list","cluster/a","ns",true);
    expect(invokeCapability).toHaveBeenCalledWith("extensions.read",{id:"org.test.app",revision:2,capability:"list",context:"cluster/a",namespace:"ns",useCrdColumns:true});
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

it("sends a host-selected app resource and the reviewed resourceVersion for actions", async () => {
  const {inspectExtensionResource,actOnExtensionResource}=await import("./extensions");
  const resource={id:"org.srelens.flux",revision:3,capability:"kustomizations",context:"cluster/a",namespace:"team",name:"apps"};
  await inspectExtensionResource(resource);
  expect(invokeCapability).toHaveBeenLastCalledWith("extensions.resource",resource);
  await actOnExtensionResource(resource,"suspend","uid","12");
  expect(invokeCapability).toHaveBeenLastCalledWith("extensions.action",{resource,action:"suspend",uid:"uid",resourceVersion:"12"});
});

it("gives each app resource its own cluster, page, namespace and name route", async () => {
  const {extensionResourceRoute}=await import("./extensions");
  const route=extensionResourceRoute("cluster/a","org.srelens.flux","kustomizations","team","apps");
  expect(parseExtensionRoute(route)).toEqual({context:"cluster/a",id:"org.srelens.flux",page:"kustomizations",namespace:"team",resourceName:"apps"});
  expect(route).not.toBe(extensionResourceRoute("cluster/b","org.srelens.flux","kustomizations","team","apps"));
});
