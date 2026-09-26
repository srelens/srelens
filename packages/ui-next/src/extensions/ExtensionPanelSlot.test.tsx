import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async original => ({ ...await original<typeof import("@srelens/core")>(),
  listContexts:vi.fn(), resolveExtensionPanels:vi.fn() }));
vi.mock("./inventoryStore", () => ({ useExtensions:vi.fn() }));
import { listContexts, resolveExtensionPanels, type InstalledExtension } from "@srelens/core";
import { useExtensions } from "./inventoryStore";
import { ExtensionPanelSlot, ResolvedPanelView } from "./ExtensionPanelSlot";

it("renders a declared cert-manager panel with missing fields and native conditions", () => {
  render(<ResolvedPanelView panel={{
    id: "certificate", title: "Certificate", sections: [
      { type: "fields", fields: [
        { label: "Issuer", value: "letsencrypt" },
        { label: "Renewal time", value: null },
      ] },
      { type: "conditions", items: [{ type: "Ready", status: "True", reason: "Issued" }] },
    ],
  }} onRetry={() => {}} />);
  expect(screen.getByRole("heading", { name: "Certificate" })).toBeTruthy();
  expect(screen.getByRole("heading", { name: "Certificate" }).closest("section")?.classList.contains("section")).toBe(true);
  expect(screen.getByText("letsencrypt")).toBeTruthy();
  expect(screen.getByText("Renewal time").parentElement?.textContent).toContain("—");
  expect(screen.getByText("Issued")).toBeTruthy();
});

it("preserves readable fields and offers Retry for a failed joined field", async () => {
  const onRetry = vi.fn();
  render(<ResolvedPanelView panel={{
    id:"application", title:"Application", sections:[{type:"fields",fields:[
      {label:"Name",value:"app"},
      {label:"Related",value:null,error:"Related resource read failed"},
    ]}],
  }} onRetry={onRetry}/>);
  expect(screen.getByText("app")).toBeTruthy();
  expect(screen.getByText(/Related resource read failed/)).toBeTruthy();
  await userEvent.click(screen.getByRole("button", {name:/Retry/}));
  expect(onRetry).toHaveBeenCalledOnce();
});

describe("an app limited to chosen clusters (Settings → Apps)", () => {
  // `/kube/a` declaring `b#c` and `/kube/a#b` declaring `c` share the stable ID `/kube/a#b#c` (#623).
  const shared = [
    { name:"b#c", stableId:"/kube/a#b#c", key:"/kube/a#b%23c", pinnedId:"srelens-context:/kube/a#b%23c" },
    { name:"c", stableId:"/kube/a#b#c", key:"/kube/a%23b#c", pinnedId:"srelens-context:/kube/a%23b#c" },
  ];
  const plugin = {
    manifest:{ id:"org.example.cert", name:"Certificates", contributions:{ detailPanels:[{
      id:"certificate", title:"Certificate", forKinds:["cert-manager.io/Certificate"],
      sections:[{type:"fields",fields:[{label:"Issuer",jsonPath:".spec.issuerRef.name"}]}],
    }] } }, enabled:true, revision:4, contexts:["/kube/a%23b#c"],
  } as InstalledExtension;
  const certificate = { apiVersion:"cert-manager.io/v1", kind:"Certificate",
    metadata:{name:"web",namespace:"team",uid:"uid-1",resourceVersion:"9"}, spec:{issuerRef:{name:"letsencrypt"}} };

  beforeEach(() => {
    vi.mocked(listContexts).mockReset().mockResolvedValue({ contexts:shared } as never);
    vi.mocked(resolveExtensionPanels).mockReset().mockResolvedValue({ panels:[{
      id:"certificate", title:"Certificate", sections:[{type:"fields",fields:[{label:"Issuer",value:"letsencrypt"}]}],
    }] });
    vi.mocked(useExtensions).mockReturnValue({ status:"ready", data:{ plugins:[plugin] }, reload:vi.fn() } as never);
  });

  it("shows its panels on its own resource page, which asks the host by pinned ID (#695)", async () => {
    render(<ExtensionPanelSlot context="srelens-context:/kube/a%23b#c" resource={certificate}/>);
    expect(await screen.findByText("letsencrypt")).toBeTruthy();
    expect(resolveExtensionPanels).toHaveBeenCalledWith(
      "org.example.cert", 4, "srelens-context:/kube/a%23b#c", "team", "cert-manager.io/Certificate", certificate);
  });

  it("shows its panels in the Inspector, which names the cluster by display name", async () => {
    render(<ExtensionPanelSlot context="c" resource={certificate}/>);
    expect(await screen.findByText("letsencrypt")).toBeTruthy();
  });

  it("shows nothing on the other context of the pair, or for the stable ID the two share", async () => {
    for (const context of ["srelens-context:/kube/a#b%23c", "b#c", "/kube/a#b#c"]) {
      const view = render(<ExtensionPanelSlot context={context} resource={certificate}/>);
      // Once the clusters are listed, not before: until then it says it is loading.
      await waitFor(() => expect(view.container.textContent).toBe(""));
      view.unmount();
    }
    expect(resolveExtensionPanels).not.toHaveBeenCalled();
  });
});
