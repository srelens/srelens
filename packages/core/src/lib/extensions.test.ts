import { describe, expect, it, vi } from "vitest";
vi.mock("../transport/transport", () => ({
  invokeCapability: vi.fn().mockResolvedValue({}),
}));
import { invokeCapability } from "../transport/transport";
import {
  configureExtensions,
  readExtension,
  resolveExtensionColumns,
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
  it("resolves a 1,000-row view in one caller-shaped batch", async () => {
    vi.mocked(invokeCapability).mockClear();
    const rows = Array.from({ length: 1_000 }, (_, index) => ({ name: `api-${index}`, namespace: "team", uid: `uid-${index}`, row: { name: `api-${index}` } }));
    await resolveExtensionColumns("org.test.app", 2, "cluster/a", "team", "apps/Deployment", rows);
    expect(invokeCapability).toHaveBeenLastCalledWith("extensions.resolveColumns", {
      id: "org.test.app", revision: 2, context: "cluster/a", namespace: "team", kind: "apps/Deployment", uids: rows,
    });
    expect(vi.mocked(invokeCapability)).toHaveBeenCalledTimes(1);
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

it("limits an app to chosen clusters, and allows every cluster when none are chosen", async () => {
  const { extensionEnabledFor } = await import("./extensions");
  type App = Parameters<typeof extensionEnabledFor>[0];
  const limited = { contexts: ["cluster/a"] } as App;
  expect(extensionEnabledFor(limited, "cluster/a")).toBe(true);
  expect(extensionEnabledFor(limited, "cluster/b")).toBe(false);
  expect(extensionEnabledFor({} as App, "cluster/b")).toBe(true);
  // A limited app stays hidden until the context's stable ID is known.
  expect(extensionEnabledFor(limited, undefined)).toBe(false);
  await configureExtensions({ action: "clusters", id: "org.test.app", contexts: ["cluster/a"] });
  expect(invokeCapability).toHaveBeenLastCalledWith("extensions.configure", {
    action: "clusters",
    id: "org.test.app",
    contexts: ["cluster/a"],
  });
  await configureExtensions({ action: "clusters", id: "org.test.app", contexts: null });
  expect(invokeCapability).toHaveBeenLastCalledWith("extensions.configure", {
    action: "clusters",
    id: "org.test.app",
    contexts: null,
  });
});

it("rolls back to a kept revision with the grants reviewed for it", async () => {
  await configureExtensions({ action: "rollback", id: "org.test.app", revision: 3, grants: ["k8s.listCustomResource"] });
  expect(invokeCapability).toHaveBeenLastCalledWith("extensions.configure", {
    action: "rollback",
    id: "org.test.app",
    revision: 3,
    grants: ["k8s.listCustomResource"],
  });
});

it("validates the exact reviewed manifest with its grants and optional signature", async () => {
  const { validateExtension } = await import("./extensions");
  await validateExtension("{}", ["k8s.listCustomResource"]);
  expect(invokeCapability).toHaveBeenLastCalledWith("extensions.validate", {
    manifest: "{}",
    grants: ["k8s.listCustomResource"],
  });
  await validateExtension("{}", [], [1, 2]);
  expect(invokeCapability).toHaveBeenLastCalledWith("extensions.validate", {
    manifest: "{}",
    grants: [],
    signature: [1, 2],
  });
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

it("announces only accepted actions so every open view of that resource can refresh", async () => {
  const { actOnExtensionResource, EXTENSION_RESOURCE_CHANGED } = await import("./extensions");
  const resource = { id: "org.srelens.flux", revision: 3, capability: "kustomizations", context: "cluster/a", namespace: "team", name: "apps" };
  const seen: unknown[] = [];
  const listener = (event: Event) => seen.push((event as CustomEvent).detail);
  window.addEventListener(EXTENSION_RESOURCE_CHANGED, listener);
  try {
    vi.mocked(invokeCapability).mockResolvedValueOnce({ requested: false });
    await actOnExtensionResource(resource, "suspend", "uid", "12");
    vi.mocked(invokeCapability).mockRejectedValueOnce(new Error("Resource changed"));
    await expect(actOnExtensionResource(resource, "suspend", "uid", "12")).rejects.toThrow("Resource changed");
    expect(seen).toEqual([]);
    vi.mocked(invokeCapability).mockResolvedValueOnce({ requested: true });
    await actOnExtensionResource(resource, "suspend", "uid", "12");
    expect(seen).toEqual([resource]);
  } finally {
    window.removeEventListener(EXTENSION_RESOURCE_CHANGED, listener);
  }
});

it("gives each app resource its own cluster, page, namespace and name route", async () => {
  const {extensionResourceRoute}=await import("./extensions");
  const route=extensionResourceRoute("cluster/a","org.srelens.flux","kustomizations","team","apps");
  expect(parseExtensionRoute(route)).toEqual({context:"cluster/a",id:"org.srelens.flux",page:"kustomizations",namespace:"team",resourceName:"apps"});
  expect(route).not.toBe(extensionResourceRoute("cluster/b","org.srelens.flux","kustomizations","team","apps"));
});

it("distinguishes stable cluster routes from literal context-name routes", async () => {
  const { extensionClusterRoute, extensionClusterResourceRoute } = await import("./extensions");
  const id = "/kube/team.yaml#team#prod";
  const route = extensionClusterRoute(id, "org.test.app", "page", "team");
  expect(parseExtensionRoute(route)).toEqual({ context: id, clusterId: id, id: "org.test.app", page: "page", namespace: "team" });
  expect(parseExtensionRoute(extensionRoute(id, "org.test.app", "page"))).not.toHaveProperty("clusterId");
  expect(parseExtensionRoute(extensionClusterResourceRoute(id, "org.test.app", "page", "team", "resource"))?.resourceName).toBe("resource");
});

describe("dashboard cards (#540)", () => {
  it("resolves an app's cards with the host's camelCase payload", async () => {
    const { resolveDashboardCards } = await import("./extensions");
    vi.mocked(invokeCapability).mockClear();
    await resolveDashboardCards("org.test.app", 3, "/kube/config#prod", ["team", "prod"]);
    expect(invokeCapability).toHaveBeenLastCalledWith("extensions.resolveCards", {
      id: "org.test.app", revision: 3, context: "/kube/config#prod", namespaces: ["team", "prod"],
    });
  });

  it("narrows a page read to a card only when one is named", async () => {
    vi.mocked(invokeCapability).mockClear();
    await readExtension("org.test.app", 2, "list", "cluster/a", "ns", true, "expiring");
    expect(invokeCapability).toHaveBeenLastCalledWith("extensions.read", {
      id: "org.test.app", revision: 2, capability: "list", context: "cluster/a", namespace: "ns",
      useCrdColumns: true, card: "expiring",
    });
    await readExtension("org.test.app", 2, "list", "cluster/a", "ns", true);
    expect(vi.mocked(invokeCapability).mock.lastCall?.[1]).not.toHaveProperty("card");
  });

  it("gives a card's target its own route, pinned to the cluster, carrying the card", async () => {
    const { extensionCardRoute, extensionClusterRoute } = await import("./extensions");
    const id = "/kube/config#prod";
    const route = extensionCardRoute(id, "org.test.app", "certificates", "team", "expiring soon");
    expect(parseExtensionRoute(route)).toEqual({
      context: id, clusterId: id, id: "org.test.app", page: "certificates", namespace: "team", card: "expiring soon",
    });
    // A filtered page is a different tab from the unfiltered one, and from another card's.
    expect(route).not.toBe(extensionClusterRoute(id, "org.test.app", "certificates", "team"));
    expect(route).not.toBe(extensionCardRoute(id, "org.test.app", "certificates", "team", "expired"));
    expect(parseExtensionRoute(extensionClusterRoute(id, "org.test.app", "certificates", "team"))).not.toHaveProperty("card");
  });

  it("refuses a card on a resource route, an empty card and an unknown parameter", () => {
    expect(parseExtensionRoute("/extension-clusters/c/org.test.app/page/team/name?card=x")).toBeNull();
    expect(parseExtensionRoute("/extension-clusters/c/org.test.app/page/team?card=")).toBeNull();
    expect(parseExtensionRoute("/extension-clusters/c/org.test.app/page/team?other=x")).toBeNull();
  });
});
