import { act, render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async original => ({...await original<typeof import("@srelens/core")>(),inspectExtensionResource:vi.fn(),actOnExtensionResource:vi.fn(),listExtensions:vi.fn(),resolveExtensionPanels:vi.fn()}));
import { inspectExtensionResource, actOnExtensionResource, type ActionPredicate, type ExtensionResourceDetail } from "@srelens/core";
import { ExtensionResourceDetails } from "./ExtensionResourceDetails";
import { requestExtensionAction, takeExtensionAction } from "./actionRequests";

// A palette action command (#544) reaches the write only through this view's
// own review: the same dialog, the same pinned UID and resourceVersion.
const selection = {id:"org.srelens.flux",revision:1,capability:"helmreleases",context:"c-1",namespace:"team",name:"web"};
const request = {id:selection.id,capability:selection.capability,context:selection.context,namespace:selection.namespace,name:selection.name,action:"reconcile"};
const detail = (availableWhen: ActionPredicate[] = []): ExtensionResourceDetail => ({
  resource:{apiVersion:"helm.toolkit.fluxcd.io/v2",kind:"HelmRelease",metadata:{name:"web",namespace:"team",uid:"uid-w",resourceVersion:"7"},spec:{suspend:false},status:{}},
  actions:["reconcile"],actionMeta:{reconcile:{title:"Reconcile",availableWhen,impact:"medium",confirm:null}},
});
beforeEach(()=>{
  vi.resetAllMocks();
  takeExtensionAction(selection);
  vi.mocked(inspectExtensionResource).mockResolvedValue(detail());
  vi.mocked(actOnExtensionResource).mockResolvedValue({requested:true});
});

it("opens the host review for a request made before the resource's tab mounted",async()=>{
  requestExtensionAction(request);
  render(<ExtensionResourceDetails fullPage selection={selection}/>);
  const review=await screen.findByRole("dialog",{name:"Review Reconcile"});
  expect(review.textContent).toContain("c-1");
  expect(actOnExtensionResource).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button",{name:"Confirm Reconcile"}));
  await waitFor(()=>expect(actOnExtensionResource).toHaveBeenCalledWith(selection,"reconcile","uid-w","7"));
});

it("opens the review in a resource tab that is already showing",async()=>{
  render(<ExtensionResourceDetails fullPage selection={selection}/>);
  await screen.findByRole("button",{name:"Reconcile"});
  expect(screen.queryByRole("dialog")).toBeNull();
  act(()=>requestExtensionAction(request));
  expect(await screen.findByRole("dialog",{name:"Review Reconcile"})).toBeTruthy();
  expect(actOnExtensionResource).not.toHaveBeenCalled();
});

it("leaves a peek beside a list alone, so one request raises one review",async()=>{
  render(<ExtensionResourceDetails selection={selection} onClose={vi.fn()}/>);
  await screen.findByRole("button",{name:"Reconcile"});
  act(()=>requestExtensionAction(request));
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("says why instead of reviewing an action the resource's own rules exclude",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue(detail([{jsonPath:".spec.suspend",equals:true,reason:"Resume it first"}]));
  requestExtensionAction(request);
  render(<ExtensionResourceDetails fullPage selection={selection}/>);
  expect((await screen.findByRole("alert")).textContent).toContain("Resume it first");
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("says so when the host no longer offers the requested action here",async()=>{
  requestExtensionAction({...request,action:"suspend"});
  render(<ExtensionResourceDetails fullPage selection={selection}/>);
  expect((await screen.findByRole("alert")).textContent).toMatch(/not offered/);
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("drops the request when the read fails, so a later Retry does not open a stale review",async()=>{
  vi.mocked(inspectExtensionResource).mockRejectedValueOnce(new Error("Access denied"));
  requestExtensionAction(request);
  render(<ExtensionResourceDetails fullPage selection={selection}/>);
  expect(await screen.findByText("Access denied")).toBeTruthy();
  fireEvent.click(screen.getByRole("button",{name:/retry/i}));
  expect(await screen.findByRole("button",{name:"Reconcile"})).toBeTruthy();
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(actOnExtensionResource).not.toHaveBeenCalled();
});

it("says why the review cannot open when the resource has no identity to pin",async()=>{
  const bare=detail();
  vi.mocked(inspectExtensionResource).mockResolvedValue({...bare,resource:{...bare.resource,metadata:{...bare.resource.metadata,uid:""}}});
  requestExtensionAction(request);
  render(<ExtensionResourceDetails fullPage selection={selection}/>);
  expect((await screen.findByRole("alert")).textContent).toMatch(/no UID or resourceVersion/);
  expect(screen.queryByRole("dialog")).toBeNull();
});
