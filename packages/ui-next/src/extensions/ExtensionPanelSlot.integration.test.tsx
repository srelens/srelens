import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async original => ({ ...await original<typeof import("@srelens/core")>(), resolveExtensionPanels:vi.fn() }));
vi.mock("./inventoryStore", () => ({ useExtensions:vi.fn() }));
vi.mock("./contextIds", () => ({ useContextLookup:vi.fn() }));
import { resolveExtensionPanels, type InstalledExtension } from "@srelens/core";
import { useExtensions } from "./inventoryStore";
import { useContextLookup } from "./contextIds";
import { ExtensionPanelSlot } from "./ExtensionPanelSlot";

const plugin = {
  manifest:{ id:"org.example.cert", name:"Certificates", contributions:{ detailPanels:[{
    id:"certificate", title:"Certificate", forKinds:["cert-manager.io/Certificate"],
    sections:[{type:"fields",fields:[{label:"Issuer",jsonPath:".spec.issuerRef.name"}]}],
  }] } }, enabled:true, revision:4, contexts:["cluster-key"],
} as InstalledExtension;
const resource = { apiVersion:"cert-manager.io/v1", kind:"Certificate",
  metadata:{name:"web",namespace:"team",uid:"uid-1",resourceVersion:"9"},
  spec:{issuerRef:{name:"letsencrypt"}} };

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(useExtensions).mockReturnValue({ status:"ready", data:{ plugins:[plugin] }, reload:vi.fn() } as never);
  vi.mocked(useContextLookup).mockReturnValue({ status:"found", id:"cluster-key" });
  vi.mocked(resolveExtensionPanels).mockResolvedValue({ panels:[{
    id:"certificate", title:"Certificate", sections:[{type:"fields",fields:[{label:"Issuer",value:"letsencrypt"}]}],
  }] });
});

it("loads an enabled matching app panel in the resource Inspector", async () => {
  render(<ExtensionPanelSlot context="cluster/a" resource={resource}/>);
  expect(await screen.findByText("letsencrypt")).toBeTruthy();
  await waitFor(() => expect(resolveExtensionPanels).toHaveBeenCalledWith(
    "org.example.cert", 4, "cluster/a", "team", "cert-manager.io/Certificate", resource,
  ));
});

it("does not load a panel when the app is not enabled on this cluster", () => {
  vi.mocked(useContextLookup).mockReturnValue({ status:"found", id:"other-cluster" });
  render(<ExtensionPanelSlot context="cluster/a" resource={resource}/>);
  expect(resolveExtensionPanels).not.toHaveBeenCalled();
});
