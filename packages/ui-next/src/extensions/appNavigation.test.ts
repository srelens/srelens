import { describe, expect, it } from "vitest";
import type { CrdRef, InstalledExtension } from "@srelens/core";
import { appNavigation } from "./appNavigation";

const app = {
  enabled: true, revision: 3, settings: {}, grants: [], source: "local", installedAt: 1, history: [],
  manifest: {
    id: "org.srelens.trivy", name: "Trivy", version: "0.1.0", srelensApiVersion: "^0.7",
    kind: "executable", permissions: [], capabilities: [],
    sidecar: { binaries: {}, operations: [{ name: "source-status", title: "Overview" }, { name: "list-images", title: "Images" }] },
    contributions: { pages: [], detailTabs: [], detailLinks: [] },
  },
} as InstalledExtension;

it("opens executable operations on the cluster and installed revision they belong to", () => {
  const navigation = appNavigation([app], "config#a/b");
  expect(navigation.map(({ id, label }) => ({ id, label }))).toEqual([{ id: "extension:org.srelens.trivy", label: "Trivy" }]);
  expect(navigation[0].children?.map(({ id, label }) => ({ id, label }))).toEqual([
    { id: "route:/extension-operation-contexts/config%23a%2Fb/org.srelens.trivy/3/source-status", label: "Overview" },
    { id: "route:/extension-operation-contexts/config%23a%2Fb/org.srelens.trivy/3/list-images", label: "Images" },
  ]);
  expect(appNavigation([{ ...app, enabled: false }], "config#a/b")).toEqual([]);
  expect(appNavigation([{ ...app, contexts: ["other"] }], "config#a/b")).toEqual([]);
});

it("omits an app whose only operations are hidden", () => {
  const hidden = structuredClone(app);
  hidden.manifest.contributions.pages = [];
  hidden.manifest.sidecar!.operations = [{name:"findings",title:"Findings",view:{hidden:true}}];
  expect(appNavigation([hidden], "config#demo")).toEqual([]);
});

/** #1001: an app with one place to go is a row, not a heading with one row under it. */
it("draws an app with one destination as a leaf, named after the app", () => {
  const single = structuredClone(app);
  single.manifest.sidecar!.operations = [{ name: "list-images", title: "Images" }];
  const [entry] = appNavigation([single], "config#a/b");
  expect(entry.children).toBeUndefined();
  expect(entry.id).toBe("route:/extension-operation-contexts/config%23a%2Fb/org.srelens.trivy/3/list-images");
  expect(entry.label).toBe("Trivy");
  expect(entry.icon).toBeTruthy();
});

/**
 * #1024: a page whose readers need a CRD this cluster doesn't serve is not
 * offered on it — the same rule `ExtensionRequirements` holds the page to.
 */
describe("on a cluster without an app's CRDs", () => {
  const reader = (name: string, args: Record<string, unknown>, versions?: string[]) => ({
    name, title: name, target: "k8s.listCustomResource", inputs: [], versions, arguments: { namespaced: true, ...args },
  });
  const certs = structuredClone(app);
  certs.manifest.id = "org.example.certs";
  certs.manifest.name = "Certificates";
  certs.manifest.sidecar = undefined;
  certs.manifest.capabilities = [
    reader("certs", { group: "cert-manager.io", version: "v1", plural: "certificates", kind: "Certificate" }),
    reader("releases", { group: "helm.toolkit.fluxcd.io", plural: "helmreleases", kind: "HelmRelease" }, ["v2", "v2beta2"]),
    { name: "pods", title: "Pods", target: "k8s.listPods", inputs: [], arguments: {} },
  ];
  certs.manifest.contributions.pages = [
    { id: "certificates", title: "Certificates", capability: "certs" },
    { id: "releases", title: "Releases", capability: "releases" },
    { id: "workloads", title: "Workloads", capability: "pods" },
    // Built-in itself, but its dashboard draws the certificates page.
    { id: "overview", title: "Overview", capability: "pods", dashboard: { pages: ["certificates"] } },
  ];
  const CERT = { name: "certificates.cert-manager.io", group: "cert-manager.io", version: "v1", versions: ["v1"], plural: "certificates", kind: "Certificate", namespaced: true };
  const RELEASE = (versions: string[]) => ({ name: "helmreleases.helm.toolkit.fluxcd.io", group: "helm.toolkit.fluxcd.io", version: versions[0], versions, plural: "helmreleases", kind: "HelmRelease", namespaced: true });
  const shown = (plugin: InstalledExtension, crds?: CrdRef[]) =>
    appNavigation([plugin], "ctx", crds).flatMap((node) => node.children ?? [node]).flatMap((node) => node.children ?? [node])
      .map((node) => node.id.split("/").filter(Boolean).at(-1));

  it("offers every page when the cluster serves what each reads", () => {
    expect(shown(certs, [CERT, RELEASE(["v2"])])).toEqual(["certificates", "releases", "workloads", "overview"]);
  });
  it("drops a page whose CRD is missing, and one whose dashboard draws it", () => {
    expect(shown(certs, [RELEASE(["v2"])])).toEqual(["releases", "workloads"]);
  });
  it("keeps a page the cluster serves at an older listed version, and drops one it serves at none", () => {
    expect(shown(certs, [CERT, RELEASE(["v2beta2"])])).toEqual(["certificates", "releases", "workloads", "overview"]);
    expect(shown(certs, [CERT, RELEASE(["v2beta1"])])).toEqual(["certificates", "workloads", "overview"]);
  });
  it("drops a page whose CRD has the other scope", () => {
    expect(shown(certs, [{ ...CERT, namespaced: false }, RELEASE(["v2"])])).toEqual(["releases", "workloads"]);
  });
  it("offers every page when what the cluster serves is unknown", () => {
    expect(shown(certs, undefined)).toEqual(["certificates", "releases", "workloads", "overview"]);
  });
  it("leaves an app that reads only built-in kinds alone", () => {
    expect(shown(app, [])).toEqual(["source-status", "list-images"]);
  });
  it("lists no app with nothing left to open, and keeps one with a visible operation", () => {
    const only = structuredClone(certs);
    only.manifest.contributions.pages = only.manifest.contributions.pages.slice(0, 2);
    expect(appNavigation([only], "ctx", [])).toEqual([]);
    only.manifest.sidecar = { binaries: {}, operations: [{ name: "scan", title: "Scan" }] };
    expect(appNavigation([only], "ctx", []).map(({ label }) => label)).toEqual(["Certificates"]);
  });
});
