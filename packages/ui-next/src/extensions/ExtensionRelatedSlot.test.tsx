import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async original => ({ ...await original<typeof import("@srelens/core")>(), resolveExtensionLinks:vi.fn() }));
vi.mock("./inventoryStore", () => ({ useExtensions:vi.fn() }));
vi.mock("./contextIds", () => ({ useContextLookup:vi.fn(), refreshContextIds:vi.fn() }));
vi.mock("../lib/tabsStore", () => ({ openTab:vi.fn() }));
vi.mock("../lib/clusters", () => ({ useContexts:vi.fn() }));
import { describeError, extensionClusterResourceRoute, resolveExtensionLinks, type ExtensionResolvedLink, type InstalledExtension } from "@srelens/core";
import { useExtensions } from "./inventoryStore";
import { useContextLookup } from "./contextIds";
import { openTab } from "../lib/tabsStore";
import { useContexts } from "../lib/clusters";
import { ExtensionRelatedSlot } from "./ExtensionRelatedSlot";

const plugin = {
  manifest:{ id:"org.example.argocd", name:"Argo CD",
    capabilities:[{name:"applications",title:"Applications",target:"k8s.listCustomResource",
      arguments:{group:"argoproj.io",kind:"Application"},inputs:["context","namespace"]}],
    contributions:{
      pages:[{id:"applications",title:"Applications",capability:"applications"}],
      detailTabs:[], detailLinks:[],
      resourceLinks:[{id:"argocd-owner",from:"apps/Deployment",to:"argoproj.io/Application",
        relation:"managedBy",match:{annotation:"argocd.argoproj.io/tracking-id",parse:"argocd-tracking-id"}}],
    } }, enabled:true, revision:4,
} as unknown as InstalledExtension;
const resource = { apiVersion:"apps/v1", kind:"Deployment",
  metadata:{ name:"api", namespace:"team", uid:"uid-1", resourceVersion:"9" } };
const link = (overrides: Partial<ExtensionResolvedLink>): ExtensionResolvedLink => ({
  id:"argocd-owner", relation:"managedBy", to:"argoproj.io/Application", capability:"applications",
  targets:[], ...overrides,
});
const answer = (links: ExtensionResolvedLink[]) => ({ from:{ kind:"apps/Deployment", namespace:"team", name:"api" }, links });

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(useExtensions).mockReturnValue({ status:"ready", data:{ plugins:[plugin] }, reload:vi.fn() } as never);
  vi.mocked(useContextLookup).mockReturnValue({ status:"found", id:"cluster-key" });
  vi.mocked(useContexts).mockReturnValue([{ name:"prod", stableId:"file/prod", key:"cluster-key" }] as never);
});

it("links each related resource to a route carrying its cluster and app kind", async () => {
  vi.mocked(resolveExtensionLinks).mockResolvedValue(answer([link({ targets:[
    { namespace:"argocd", name:"guestbook", exists:true },
    { namespace:"apps", name:"guestbook", exists:true },
  ] })]));
  render(<ExtensionRelatedSlot context="prod" resource={resource}/>);
  expect(screen.getByRole("heading", { name:"Related" })).toBeTruthy();
  const first = await screen.findByRole("button", { name:/Managed by.*Application.*argocd\/guestbook/ });
  await waitFor(() => expect(resolveExtensionLinks).toHaveBeenCalledWith(
    "org.example.argocd", 4, "prod", "team", "apps/Deployment", resource));
  await userEvent.click(first);
  await userEvent.click(screen.getByRole("button", { name:/apps\/guestbook/ }));
  const routes = vi.mocked(openTab).mock.calls.map(([route]) => route);
  expect(routes).toEqual([
    extensionClusterResourceRoute("cluster-key", "org.example.argocd", "applications", "argocd", "guestbook"),
    extensionClusterResourceRoute("cluster-key", "org.example.argocd", "applications", "apps", "guestbook"),
  ]);
  // Two targets, two tabs: the routes differ.
  expect(new Set(routes).size).toBe(2);
  expect(vi.mocked(openTab).mock.calls[0][1]).toEqual({ clusterName:"prod" });
});

// `/kube/a` declaring `b#c` and `/kube/a#b` declaring `c` share the stable ID `/kube/a#b#c` (#623).
const shared = [
  { name:"b#c", stableId:"/kube/a#b#c", key:"/kube/a#b%23c" },
  { name:"c", stableId:"/kube/a#b#c", key:"/kube/a%23b#c" },
];

it("opens a link from each of two contexts that share a stable ID in a tab of its own (#695)", async () => {
  vi.mocked(useContexts).mockReturnValue(shared as never);
  vi.mocked(resolveExtensionLinks).mockResolvedValue(answer([link({ targets:[{ namespace:"argocd", name:"guestbook", exists:true }] })]));
  for (const context of shared) {
    const view = render(<ExtensionRelatedSlot context={context.name} resource={resource}/>);
    await userEvent.click(await screen.findByRole("button", { name:/argocd\/guestbook/ }));
    expect(resolveExtensionLinks).toHaveBeenLastCalledWith("org.example.argocd", 4, context.name, "team", "apps/Deployment", resource);
    view.unmount();
  }
  expect(vi.mocked(openTab).mock.calls).toEqual([
    ["/extension-contexts/%2Fkube%2Fa%23b%2523c/org.example.argocd/applications/argocd/guestbook", { clusterName:"b#c" }],
    ["/extension-contexts/%2Fkube%2Fa%2523b%23c/org.example.argocd/applications/argocd/guestbook", { clusterName:"c" }],
  ]);
});

it("links from an app resource page by the key that page reads its cluster by", async () => {
  vi.mocked(useContexts).mockReturnValue(shared as never);
  vi.mocked(resolveExtensionLinks).mockResolvedValue(answer([link({ targets:[{ namespace:"argocd", name:"guestbook", exists:true }] })]));
  render(<ExtensionRelatedSlot context="/kube/a%23b#c" resource={resource}/>);
  await userEvent.click(await screen.findByRole("button", { name:/argocd\/guestbook/ }));
  expect(vi.mocked(openTab).mock.calls[0]).toEqual([
    "/extension-contexts/%2Fkube%2Fa%2523b%23c/org.example.argocd/applications/argocd/guestbook", { clusterName:"c" },
  ]);
});

it("says no related resources only when every link answered", async () => {
  vi.mocked(resolveExtensionLinks).mockResolvedValue(answer([link({})]));
  render(<ExtensionRelatedSlot context="prod" resource={resource}/>);
  expect(await screen.findByText("No related resources.")).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
});

it("shows a failed link as a failure with a retry, never as no related resources", async () => {
  // The host's raw reason goes through describeError, as every other failure
  // in the app does, so a refused or unreachable cluster says what to do.
  const raw = "handler error: list joined custom resources: Forbidden: applications.argoproj.io is forbidden";
  const detail = describeError(raw).detail;
  expect(detail).not.toBe(raw);
  vi.mocked(resolveExtensionLinks).mockResolvedValue(answer([link({ error:raw })]));
  render(<ExtensionRelatedSlot context="prod" resource={resource}/>);
  const row = await screen.findByText(/Couldn’t read/);
  expect(row.textContent).toContain(detail);
  expect(screen.queryByText("No related resources.")).toBeNull();
  vi.mocked(resolveExtensionLinks).mockResolvedValue(answer([link({})]));
  await userEvent.click(screen.getByRole("button", { name:/Retry/ }));
  expect(await screen.findByText("No related resources.")).toBeTruthy();
});

it("shows a failed resolver call as a failure too", async () => {
  vi.mocked(resolveExtensionLinks).mockRejectedValue(new Error("Extension was disabled or updated; refresh the view"));
  render(<ExtensionRelatedSlot context="prod" resource={resource}/>);
  expect(await screen.findByText(/Couldn’t read related resources from Argo CD/)).toBeTruthy();
  expect(screen.queryByText("No related resources.")).toBeNull();
});

it("names a target the cluster does not have without linking to it", async () => {
  vi.mocked(resolveExtensionLinks).mockResolvedValue(answer([link({ targets:[
    { namespace:null, name:"gone", exists:false },
  ] })]));
  render(<ExtensionRelatedSlot context="prod" resource={resource}/>);
  expect(await screen.findByText(/gone.*not found on this cluster/)).toBeTruthy();
  expect(screen.queryByRole("button", { name:/gone/ })).toBeNull();
});

it("names a target the host did not look up, with why, and does not call it missing", async () => {
  vi.mocked(resolveExtensionLinks).mockResolvedValue(answer([link({ targets:[
    { namespace:null, name:"guestbook", exists:false,
      unverified:"namespace unknown: the app declares no defaultNamespace for a bare name" },
  ] })]));
  render(<ExtensionRelatedSlot context="prod" resource={resource}/>);
  expect(await screen.findByText(/guestbook.*namespace unknown/)).toBeTruthy();
  expect(screen.queryByText(/not found on this cluster/)).toBeNull();
  expect(screen.queryByText("No related resources.")).toBeNull();
  expect(screen.queryByRole("button", { name:/guestbook/ })).toBeNull();
  expect(screen.queryByText(/Couldn’t read/)).toBeNull();
});

it("renders a live and a stale target of one name as two rows with distinct keys", async () => {
  // Two owner references to team/web with different uids: one is the live
  // owner, the other a deleted one the cluster no longer has.
  const errors = vi.spyOn(console, "error").mockImplementation(() => {});
  vi.mocked(resolveExtensionLinks).mockResolvedValue(answer([link({ targets:[
    { namespace:"team", name:"web", exists:true },
    { namespace:"team", name:"web", exists:false },
  ] })]));
  render(<ExtensionRelatedSlot context="prod" resource={resource}/>);
  expect(await screen.findByRole("button", { name:/team\/web/ })).toBeTruthy();
  expect(screen.getByText(/team\/web.*not found on this cluster/)).toBeTruthy();
  const duplicate = errors.mock.calls.some(call => call.some(arg => String(arg).includes("same key")));
  errors.mockRestore();
  expect(duplicate).toBe(false);
});

it("sends the resolver only the resource's identity and metadata, never its data or spec", async () => {
  // The resolver reads apiVersion, kind and metadata. A large ConfigMap's data,
  // a spec, or a Secret's values are nothing it needs, so none of it is sent.
  const linked = (from: string) => ({ ...plugin, manifest:{ ...plugin.manifest, contributions:{
    ...plugin.manifest.contributions, resourceLinks:[{ id:"owner", from, to:"argoproj.io/Application",
      relation:"ownedBy", match:{ label:"example.io/app" } }] } } }) as unknown as InstalledExtension;
  const metadata = { name:"cfg", namespace:"team", uid:"u-1", resourceVersion:"3", labels:{ "example.io/app":"guestbook" } };
  const cases = [
    { from:"/ConfigMap", resource:{ apiVersion:"v1", kind:"ConfigMap", metadata, data:{ big:"x".repeat(100_000) } } },
    { from:"/Secret", resource:{ apiVersion:"v1", kind:"Secret", metadata, type:"Opaque", data:{ token:"c2VjcmV0" } } },
    { from:"apps/Deployment", resource:{ apiVersion:"apps/v1", kind:"Deployment", metadata,
      spec:{ replicas:3 }, status:{ readyReplicas:3 } } },
  ];
  for (const { from, resource: sent } of cases) {
    vi.mocked(resolveExtensionLinks).mockReset().mockResolvedValue(answer([link({})]));
    vi.mocked(useExtensions).mockReturnValue({ status:"ready", data:{ plugins:[linked(from)] }, reload:vi.fn() } as never);
    const view = render(<ExtensionRelatedSlot context="prod" resource={sent}/>);
    await waitFor(() => expect(resolveExtensionLinks).toHaveBeenCalledOnce());
    const payload = vi.mocked(resolveExtensionLinks).mock.calls[0][5];
    expect(payload).toEqual({ apiVersion:sent.apiVersion, kind:sent.kind, metadata });
    expect(JSON.stringify(payload)).not.toMatch(/c2VjcmV0|xxxx|replicas/);
    view.unmount();
  }
});

it("says it is loading while the app inventory is, rather than showing nothing", () => {
  // Nothing yet is not "no app offers links": that is only known once the
  // inventory has answered.
  vi.mocked(useExtensions).mockReturnValue({ status:"loading", reload:vi.fn() } as never);
  render(<ExtensionRelatedSlot context="prod" resource={resource}/>);
  const related = screen.getByRole("region", { name:"Related" });
  expect(related.textContent).toMatch(/Loading Related resources/);
  expect(resolveExtensionLinks).not.toHaveBeenCalled();
});

it("shows a failed app inventory as a failure with a retry, not as no links", async () => {
  const reload = vi.fn();
  vi.mocked(useExtensions).mockReturnValue({ status:"error", error:"inventory unreadable", reload } as never);
  render(<ExtensionRelatedSlot context="prod" resource={resource}/>);
  expect(screen.getByRole("region", { name:"Related" }).textContent).toContain("inventory unreadable");
  await userEvent.click(screen.getByRole("button", { name:/Retry/ }));
  expect(reload).toHaveBeenCalledOnce();
});

it("renders nothing for a kind no app links from", () => {
  const { container } = render(<ExtensionRelatedSlot context="prod" resource={{ ...resource, kind:"StatefulSet" }}/>);
  expect(container.textContent).toBe("");
  expect(resolveExtensionLinks).not.toHaveBeenCalled();
});

it("does not resolve links for an app not enabled on this cluster", () => {
  vi.mocked(useExtensions).mockReturnValue({ status:"ready", data:{ plugins:[{ ...plugin, contexts:["other"] }] }, reload:vi.fn() } as never);
  render(<ExtensionRelatedSlot context="prod" resource={resource}/>);
  expect(resolveExtensionLinks).not.toHaveBeenCalled();
});

it("names a target as plain text when its cluster is not listed, rather than open a tab for no cluster", async () => {
  vi.mocked(useContexts).mockReturnValue([] as never);
  vi.mocked(resolveExtensionLinks).mockResolvedValue(answer([link({ targets:[{ namespace:"argocd", name:"guestbook", exists:true }] })]));
  render(<ExtensionRelatedSlot context="prod" resource={resource}/>);
  expect(await screen.findByText(/Managed by.*Application.*argocd\/guestbook/)).toBeTruthy();
  expect(screen.queryByRole("button", { name:/argocd\/guestbook/ })).toBeNull();
});
