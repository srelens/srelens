import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async original => ({ ...(await original<typeof import("@srelens/core")>()), listCrds: vi.fn() }));
import { listCrds, type InstalledExtension } from "@srelens/core";
import { ExtensionRequirements } from "./ExtensionRequirements";
const plugin = { manifest: { id: "org.test.argo", name: "Argo CD", capabilities: [{ name: "apps", target: "k8s.listCustomResource", arguments: { group: "argoproj.io", version: "v1alpha1", plural: "applications", kind: "Application" } }], contributions: { pages: [{ id: "apps", capability: "apps" }] } }, revision: 1 } as unknown as InstalledExtension;
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
