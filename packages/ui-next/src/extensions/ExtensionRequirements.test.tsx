import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async original => ({ ...(await original<typeof import("@srelens/core")>()), listCrds: vi.fn() }));
import { listCrds, type InstalledExtension } from "@srelens/core";
import { ExtensionRequirements } from "./ExtensionRequirements";
const plugin = { manifest: { id: "org.test.argo", name: "Argo CD", capabilities: [{ name: "apps", target: "k8s.listCustomResource", arguments: { group: "argoproj.io", version: "v1alpha1", plural: "applications", kind: "Application", namespaced: true } }], contributions: { pages: [{ id: "apps", capability: "apps" }] } }, revision: 1 } as unknown as InstalledExtension;
const crd = { name: "applications.argoproj.io", group: "argoproj.io", version: "v1alpha1", plural: "applications", kind: "Application", namespaced: true };
function view(context = "staging") { return <ExtensionRequirements plugin={plugin} page={plugin.manifest.contributions.pages[0]} context={context} refresh={0}><div>Resource content</div></ExtensionRequirements>; }
beforeEach(()=>vi.resetAllMocks());
it("shows missing CRDs without mounting resource readers, and rechecks after installation", async()=>{
  vi.mocked(listCrds).mockResolvedValueOnce({crds:[]}).mockResolvedValue({crds:[crd]});
  render(view());
  expect(await screen.findByText("Missing requirements")).toBeTruthy();
  expect(screen.getByText("applications.argoproj.io")).toBeTruthy();
  expect(screen.queryByText("Resource content")).toBeNull();
  fireEvent.click(screen.getByText("Check again"));
  expect(await screen.findByText("Resource content")).toBeTruthy();
});
it("keeps discovery permission failures distinct from missing CRDs and allows authorized resource reads",async()=>{
  vi.mocked(listCrds).mockResolvedValue({error:"Forbidden"});
  render(view());
  expect(await screen.findByText("Could not check requirements")).toBeTruthy();
  expect(screen.queryByText("Missing requirements")).toBeNull();
  expect(screen.getByText("Resource content")).toBeTruthy();
});
it("checks the selected cluster and exact served version, not a prior cluster result",async()=>{
  vi.mocked(listCrds).mockResolvedValueOnce({crds:[crd]}).mockResolvedValue({crds:[{...crd,version:"v1",versions:["v1"]}]});
  const rendered=render(view());
  await screen.findByText("Resource content");
  rendered.rerender(view("production"));
  expect(await screen.findByText("Required version unavailable")).toBeTruthy();
  expect(screen.queryByText("Resource content")).toBeNull();
  await waitFor(()=>expect(listCrds).toHaveBeenLastCalledWith("production"));
});
it("does not discover cluster requirements when no cluster is selected",()=>{
  render(view(""));
  expect(listCrds).not.toHaveBeenCalled();
});

// A reader that lists versions (#547): the first the cluster serves is the one read.
const multi = { manifest: { id: "org.test.flux", name: "Flux", capabilities: [
  { name: "releases", target: "k8s.listCustomResource", versions: ["v2", "v2beta2"], arguments: { group: "helm.toolkit.fluxcd.io", plural: "helmreleases", kind: "HelmRelease", namespaced: true } },
  { name: "sources", target: "k8s.listCustomResource", arguments: { group: "source.toolkit.fluxcd.io", version: "v1", plural: "gitrepositories", kind: "GitRepository", namespaced: true } },
], contributions: { pages: [
  { id: "releases", capability: "releases" },
  { id: "overview", capability: "releases", dashboard: { pages: ["releases", "sources"] } },
  { id: "sources", capability: "sources" },
] } }, revision: 1 } as unknown as InstalledExtension;
const release = (versions: string[]) => ({ name: "helmreleases.helm.toolkit.fluxcd.io", group: "helm.toolkit.fluxcd.io", version: versions[0] ?? "", versions, plural: "helmreleases", kind: "HelmRelease", namespaced: true });
function multiView(page = 0) {
  return <ExtensionRequirements plugin={multi} page={multi.manifest.contributions.pages[page]} context="staging" refresh={0}><div>Resource content</div></ExtensionRequirements>;
}
const rowOf = (text: string) => screen.getByText(text).closest("tr") as HTMLElement;

it("opens a page on a cluster that serves only an older listed version", async () => {
  vi.mocked(listCrds).mockResolvedValue({ crds: [release(["v2beta2"])] });
  render(multiView());
  expect(await screen.findByText("Resource content")).toBeTruthy();
});

it("fails closed when the cluster serves none of the listed versions, and lists every one", async () => {
  vi.mocked(listCrds).mockResolvedValue({ crds: [release(["v2beta1"])] });
  render(multiView());
  expect(await screen.findByText("Required version unavailable")).toBeTruthy();
  expect(screen.queryByText("Resource content")).toBeNull();
  const row = rowOf("helmreleases.helm.toolkit.fluxcd.io");
  expect(row.textContent).toContain("v2");
  expect(row.textContent).toContain("v2beta2");
  expect(row.textContent).toContain("None served");
  expect(row.textContent).not.toContain("v2beta1");
});

it.each([
  [["v2beta2"], "v2beta2"],
  [["v2beta2", "v2"], "v2"],
])("says which listed version a cluster serving %j resolves to", async (served, resolved) => {
  // Another requirement is missing, so the table is drawn and the release row is read.
  vi.mocked(listCrds).mockResolvedValue({ crds: [release(served)] });
  render(multiView(1));
  expect(await screen.findByText("Missing requirements")).toBeTruthy();
  const row = rowOf("helmreleases.helm.toolkit.fluxcd.io");
  expect(within(row).getByLabelText("Reads")).toHaveProperty("textContent", resolved);
  expect(row.textContent).toContain("Available");
  expect(within(rowOf("gitrepositories.source.toolkit.fluxcd.io")).getByLabelText("Reads").textContent).toBe("None served");
});

it.each([true, false])("blocks a CRD whose scope disagrees with namespaced=%s", async (namespaced) => {
 const scoped=structuredClone(plugin);
 scoped.manifest.capabilities[0].arguments.namespaced=namespaced;
 vi.mocked(listCrds).mockResolvedValue({crds:[{...crd,namespaced:!namespaced}]});
 const rendered=render(<ExtensionRequirements plugin={scoped} page={scoped.manifest.contributions.pages[0]} context="staging" refresh={0}><div>Resource content</div></ExtensionRequirements>);
 expect(await screen.findByText("Scope mismatch")).toBeTruthy();
 expect(screen.queryByText("Resource content")).toBeNull();
 vi.mocked(listCrds).mockResolvedValue({crds:[{...crd,namespaced}]});
 fireEvent.click(screen.getByText("Check again"));
 expect(await screen.findByText("Resource content")).toBeTruthy();
 rendered.unmount();
});
