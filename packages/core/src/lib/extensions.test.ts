import { describe, expect, it, vi } from "vitest";
vi.mock("../transport/transport", () => ({
  invokeCapability: vi.fn().mockResolvedValue({}),
}));
vi.mock("../transport/platform", () => ({ isTauri: vi.fn(() => true), isWeb: false }));
import { invokeCapability } from "../transport/transport";
import { isTauri } from "../transport/platform";
import {
  clearExtensionSecret,
  setExtensionSecret,
  configureExtensions,
  readExtension,
  resolveExtensionColumns,
  resolveExtensionLinks,
  resolveExtensionReverseLinks,
  extensionRoute,
  parseExtensionRoute,
  itemStatus,
  itemStatuses,
  providersFor,
  queryExtensionProvider,
  type ExtensionManifest,
} from "./extensions";
// What the Rust `QueryIn` test deserializes, byte for byte (#569).
import queryProviderPayload from "./extension-query-provider.json";

describe("itemStatus: one normalized status per listed resource (#541)", () => {
  const legacy = { ready: 0, suspended: 1, progressing: 2 };
  const item = (columns: string[], status?: { status: "warning"; label: string }) =>
    ({ name: "a", namespace: "n", age: "1d", columns, ...(status ? { status } : {}) });

  it("takes the host's resolved status when the app declares a resolver", () => {
    expect(itemStatus(item(["True"], { status: "warning", label: "Out of sync" }), legacy)).toBe("warning");
  });

  it("maps deprecated statusColumns onto the same six statuses", () => {
    expect(itemStatus(item(["True", "true", "False"]), legacy)).toBe("suspended");
    expect(itemStatus(item(["True", "false", "True"]), legacy)).toBe("progressing");
    expect(itemStatus(item(["True", "false", "False"]), legacy)).toBe("healthy");
    expect(itemStatus(item(["False"]), legacy)).toBe("error");
    expect(itemStatus(item([]), legacy)).toBe("unknown");
  });

  it("says unknown when nothing classifies the item, and maps a list in order", () => {
    expect(itemStatus(item(["True"]))).toBe("unknown");
    expect(itemStatuses([item(["False"]), item(["True"], { status: "warning", label: "W" })], legacy))
      .toEqual(["error", "warning"]);
  });
});
describe("app secrets (#543): write-only, desktop only", () => {
  it("sets and clears through extension.secretStore with the host's field names", async () => {
    vi.mocked(invokeCapability).mockClear();
    vi.mocked(invokeCapability).mockResolvedValueOnce({ set: true });
    await expect(setExtensionSecret("org.test.app", "token", "s3cret")).resolves.toEqual({ set: true });
    expect(invokeCapability).toHaveBeenCalledWith("extension.secretStore", {
      action: "set", id: "org.test.app", setting: "token", secret: "s3cret",
    });
    vi.mocked(invokeCapability).mockResolvedValueOnce({ set: false });
    await clearExtensionSecret("org.test.app", "token");
    expect(invokeCapability).toHaveBeenLastCalledWith("extension.secretStore", {
      action: "clear", id: "org.test.app", setting: "token",
    });
    // Every secret of the app, as a reset asks.
    vi.mocked(invokeCapability).mockResolvedValueOnce({ set: false });
    await clearExtensionSecret("org.test.app");
    expect(invokeCapability).toHaveBeenLastCalledWith("extension.secretStore", { action: "clear", id: "org.test.app" });
  });

  it("answers only whether it is set, whatever else a host sends back", async () => {
    vi.mocked(invokeCapability).mockResolvedValueOnce({ set: true, secret: "echoed" });
    await expect(setExtensionSecret("org.test.app", "token", "s3cret")).resolves.toEqual({ set: true });
  });

  it("refuses on the web before the value leaves the page", async () => {
    vi.mocked(isTauri).mockReturnValue(false);
    vi.mocked(invokeCapability).mockClear();
    try {
      await expect(setExtensionSecret("org.test.app", "token", "s3cret")).rejects.toThrow(/desktop app/);
      await expect(clearExtensionSecret("org.test.app", "token")).rejects.toThrow(/desktop app/);
      expect(invokeCapability).not.toHaveBeenCalled();
    } finally {
      vi.mocked(isTauri).mockReturnValue(true);
    }
  });
});
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
  it("sends a link resolve with the resolver's own field names (#545)", async () => {
    const resource = { apiVersion: "apps/v1", kind: "Deployment", metadata: { name: "api", namespace: "team" } };
    await resolveExtensionLinks("org.test.app", 3, "cluster/a", "team", "apps/Deployment", resource);
    // `ResolveLinks` in crates/registry/src/extensions/links.rs denies unknown fields.
    expect(invokeCapability).toHaveBeenLastCalledWith("extensions.resolveLinks", {
      id: "org.test.app", revision: 3, context: "cluster/a", namespace: "team", kind: "apps/Deployment", resource,
    });
  });
  it("sends a reverse link resolve with the forward resolver's field names (#728)", async () => {
    const resource = { apiVersion: "argoproj.io/v1alpha1", kind: "Application", metadata: { name: "guestbook", namespace: "argocd" } };
    await resolveExtensionReverseLinks("org.test.app", 3, "cluster/a", "argocd", "argoproj.io/Application", resource);
    // Both resolvers deserialize `ResolveLinks`, which denies unknown fields.
    expect(invokeCapability).toHaveBeenLastCalledWith("extensions.resolveReverseLinks", {
      id: "org.test.app", revision: 3, context: "cluster/a", namespace: "argocd", kind: "argoproj.io/Application", resource,
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

it("sends a package's digest list with its manifest and signature for the check (#562)", async () => {
  const { validateExtension } = await import("./extensions");
  await validateExtension("{}", ["k8s.listCustomResource"], [1, 2], "{\"format\":\"srelens-extension-package\"}");
  expect(invokeCapability).toHaveBeenLastCalledWith("extensions.validate", {
    manifest: "{}",
    grants: ["k8s.listCustomResource"],
    signature: [1, 2],
    digests: "{\"format\":\"srelens-extension-package\"}",
  });
  // An unsigned package still sends its list, so the host checks the manifest against it.
  await validateExtension("{}", [], undefined, "{}");
  expect(invokeCapability).toHaveBeenLastCalledWith("extensions.validate", { manifest: "{}", grants: [], digests: "{}" });
});

it("sends a package file as base64, for review and for install (#562)", async () => {
  const { reviewExtensionPackage, encodePackage } = await import("./extensions");
  const bytes = new Uint8Array([0x1f, 0x8b, 0x08, 0x00, 0xff]);
  expect(encodePackage(bytes)).toBe("H4sIAP8=");
  // Larger than one chunk of the encoder, so the chunks are joined in order.
  const large = Uint8Array.from({ length: 0x8000 * 2 + 5 }, (_, at) => at % 251);
  expect(atob(encodePackage(large)).length).toBe(large.length);
  expect(Uint8Array.from(atob(encodePackage(large)), (c) => c.charCodeAt(0))).toEqual(large);
  await reviewExtensionPackage(bytes);
  expect(invokeCapability).toHaveBeenLastCalledWith("extensions.packageManifest", { package: "H4sIAP8=" });
  await configureExtensions({ action: "installPackage", package: "H4sIAP8=", grants: ["k8s.listCustomResource"], reviewedRevision: 2 });
  expect(invokeCapability).toHaveBeenLastCalledWith("extensions.configure", {
    action: "installPackage",
    package: "H4sIAP8=",
    grants: ["k8s.listCustomResource"],
    reviewedRevision: 2,
  });
  await configureExtensions({ action: "installCatalogPackage", id: "org.srelens.flux", sha256: "a", packageSha256: "b", grants: [] });
  expect(invokeCapability).toHaveBeenLastCalledWith("extensions.configure", {
    action: "installCatalogPackage",
    id: "org.srelens.flux",
    sha256: "a",
    packageSha256: "b",
    grants: [],
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

it("distinguishes context-key routes from literal context-name routes", async () => {
  const { extensionClusterRoute, extensionClusterResourceRoute } = await import("./extensions");
  const key = "/kube/team.yaml#team%23prod";
  const route = extensionClusterRoute(key, "org.test.app", "page", "team");
  expect(parseExtensionRoute(route)).toEqual({ context: key, contextKey: key, id: "org.test.app", page: "page", namespace: "team" });
  expect(parseExtensionRoute(extensionRoute(key, "org.test.app", "page"))).not.toHaveProperty("contextKey");
  expect(parseExtensionRoute(extensionClusterResourceRoute(key, "org.test.app", "page", "team", "resource"))?.resourceName).toBe("resource");
});

describe("app routes name their cluster by context key (#695)", () => {
  // `/kube/a` declaring `b#c` and `/kube/a#b` declaring `c` share the stable ID
  // `/kube/a#b#c`; their keys encode `#` in each part, so they differ.
  const first = "/kube/a#b%23c";
  const second = "/kube/a%23b#c";

  it("gives two contexts that share a stable ID two routes to one page, resource and card", async () => {
    const { extensionClusterRoute, extensionClusterResourceRoute, extensionCardRoute } = await import("./extensions");
    expect(extensionClusterRoute(first, "org.test.app", "page", "team"))
      .toBe("/extension-contexts/%2Fkube%2Fa%23b%2523c/org.test.app/page/team");
    expect(extensionClusterRoute(second, "org.test.app", "page", "team"))
      .toBe("/extension-contexts/%2Fkube%2Fa%2523b%23c/org.test.app/page/team");
    expect(extensionClusterResourceRoute(first, "org.test.app", "page", "team", "web"))
      .not.toBe(extensionClusterResourceRoute(second, "org.test.app", "page", "team", "web"));
    expect(extensionCardRoute(first, "org.test.app", "page", "team", "expiring"))
      .not.toBe(extensionCardRoute(second, "org.test.app", "page", "team", "expiring"));
    expect(parseExtensionRoute(extensionClusterResourceRoute(second, "org.test.app", "page", "team", "web"))).toEqual({
      context: second, contextKey: second, id: "org.test.app", page: "page", namespace: "team", resourceName: "web",
    });
  });

  it("still reads a route opened before, which names its cluster by stable ID", () => {
    const route = "/extension-clusters/%2Fkube%2Fa%23b%23c/org.test.app/page/team/web";
    expect(parseExtensionRoute(route)).toEqual({
      context: "/kube/a#b#c", clusterId: "/kube/a#b#c", id: "org.test.app", page: "page", namespace: "team", resourceName: "web",
    });
    // One string can be a key and a stable ID; which one the route means is in its prefix.
    expect(parseExtensionRoute("/extension-clusters/%2Fkube%2Fx%23y%2523z/org.test.app/page/")).not.toHaveProperty("contextKey");
  });
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
      context: id, contextKey: id, id: "org.test.app", page: "certificates", namespace: "team", card: "expiring soon",
    });
    // A filtered page is a different tab from the unfiltered one, and from another card's.
    expect(route).not.toBe(extensionClusterRoute(id, "org.test.app", "certificates", "team"));
    expect(route).not.toBe(extensionCardRoute(id, "org.test.app", "certificates", "team", "expired"));
    expect(parseExtensionRoute(extensionClusterRoute(id, "org.test.app", "certificates", "team"))).not.toHaveProperty("card");
  });

  it("carries a card's several namespaces in its route and its read", async () => {
    const { extensionCardRoute } = await import("./extensions");
    const id = "/kube/config#prod";
    const route = extensionCardRoute(id, "org.test.app", "certificates", "", "expiring", ["team", "prod"]);
    expect(parseExtensionRoute(route)).toEqual({
      context: id, contextKey: id, id: "org.test.app", page: "certificates", namespace: "",
      card: "expiring", namespaces: ["prod", "team"],
    });
    // One selection, one tab, whatever order it was picked in.
    expect(route).toBe(extensionCardRoute(id, "org.test.app", "certificates", "", "expiring", ["prod", "team"]));
    expect(route).not.toBe(extensionCardRoute(id, "org.test.app", "certificates", "", "expiring"));
    // One namespace stays in the path, as every other app route has it.
    expect(extensionCardRoute(id, "org.test.app", "certificates", "team", "expiring", ["team"])).toBe(
      extensionCardRoute(id, "org.test.app", "certificates", "team", "expiring"),
    );
    vi.mocked(invokeCapability).mockClear();
    await readExtension("org.test.app", 2, "list", "cluster/a", "", true, "expiring", ["prod", "team"]);
    expect(invokeCapability).toHaveBeenLastCalledWith("extensions.read", {
      id: "org.test.app", revision: 2, capability: "list", context: "cluster/a", namespace: "",
      useCrdColumns: true, card: "expiring", namespaces: ["prod", "team"],
    });
    expect(parseExtensionRoute("/extension-clusters/c/org.test.app/page/?namespaces=a,b")).toBeNull();
    expect(parseExtensionRoute("/extension-clusters/c/org.test.app/page/team?card=x&namespaces=a,b")).toBeNull();
  });

  it("refuses a card on a resource route, an empty card and an unknown parameter", () => {
    expect(parseExtensionRoute("/extension-clusters/c/org.test.app/page/team/name?card=x")).toBeNull();
    expect(parseExtensionRoute("/extension-clusters/c/org.test.app/page/team?card=")).toBeNull();
    expect(parseExtensionRoute("/extension-clusters/c/org.test.app/page/team?other=x")).toBeNull();
  });
});

describe("providers (#569)", () => {
  it("queries one exactly as the host's QueryIn accepts it", async () => {
    vi.mocked(invokeCapability).mockClear();
    await queryExtensionProvider({
      id: "org.example.observability", revision: 3, provider: "cpu", context: "kind-dev",
      namespace: "team", resourceKind: "apps/Deployment", name: "web", rangeSeconds: 3600,
    });
    expect(invokeCapability).toHaveBeenCalledWith("extensions.queryProvider", queryProviderPayload);
    expect(JSON.stringify(vi.mocked(invokeCapability).mock.calls[0][1])).not.toMatch(/resource_kind|range_seconds/);
  });

  it("leaves the range out when the view names none, so the host's default holds", async () => {
    vi.mocked(invokeCapability).mockClear();
    await queryExtensionProvider({
      id: "a.b", revision: 1, provider: "cpu", context: "c", namespace: "n", resourceKind: "/Pod", name: "p",
    });
    expect(vi.mocked(invokeCapability).mock.calls[0][1]).not.toHaveProperty("rangeSeconds");
  });

  const manifest = {
    contributions: {
      pages: [], detailTabs: [], detailLinks: [],
      metricProviders: [
        { id: "cpu", title: "CPU", capability: "prom", language: "promql", forKinds: ["apps/Deployment"], unit: "cores", query: "up" },
        { id: "pods", title: "Pod CPU", capability: "prom", language: "promql", forKinds: ["/Pod"], unit: "cores", query: "up" },
      ],
      logProviders: [{ id: "loki", title: "Loki", capability: "loki", language: "logql", forKinds: ["/Pod"], query: "{}" }],
    },
  } as unknown as ExtensionManifest;

  it("finds the providers of one list declared for a kind, in manifest order", () => {
    expect(providersFor(manifest, "metrics", "apps/Deployment").map((p) => p.id)).toEqual(["cpu"]);
    expect(providersFor(manifest, "metrics", "/Pod").map((p) => p.id)).toEqual(["pods"]);
    expect(providersFor(manifest, "logs", "/Pod").map((p) => p.id)).toEqual(["loki"]);
    expect(providersFor(manifest, "traces", "/Pod")).toEqual([]);
    // A manifest from before providers, or one the host has not checked, has none.
    expect(providersFor({ contributions: {} } as unknown as ExtensionManifest, "logs", "/Pod")).toEqual([]);
  });
});
