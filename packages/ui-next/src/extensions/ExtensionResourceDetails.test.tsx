import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async original => ({...await original<typeof import("@srelens/core")>(),inspectExtensionResource:vi.fn(),actOnExtensionResource:vi.fn()}));
import { inspectExtensionResource, actOnExtensionResource, EXTENSION_RESOURCE_CHANGED } from "@srelens/core";
import { ExtensionResourceDetails } from "./ExtensionResourceDetails";
const selection = {id:"org.srelens.flux",revision:1,capability:"kustomizations",context:"cluster/a",namespace:"team",name:"apps"};
const detail = {resource:{apiVersion:"kustomize.toolkit.fluxcd.io/v1",kind:"Kustomization",metadata:{name:"apps",namespace:"team",uid:"uid-a",resourceVersion:"12"},spec:{suspend:false,path:"./apps",sourceRef:{kind:"GitRepository",name:"platform-config"}},status:{conditions:[{type:"Ready",status:"False",reason:"BuildFailed",message:"Missing source"}],lastAppliedRevision:"main@sha1:abcdef"}},actions:["suspend","resume","reconcile"]};
beforeEach(()=>{vi.resetAllMocks();vi.mocked(inspectExtensionResource).mockResolvedValue(detail);vi.mocked(actOnExtensionResource).mockResolvedValue({requested:true});});
it("shows overview and conditions, then confirms the exact pinned resource before requesting an action",async()=>{
 render(<ExtensionResourceDetails selection={selection} onClose={vi.fn()}/>);
 expect(await screen.findByText("Missing source")).toBeTruthy();
 expect(screen.getByText("./apps")).toBeTruthy();
 // Like the real wrapper, an accepted action announces the resource; the view refreshes from that.
 vi.mocked(actOnExtensionResource).mockImplementation(async resource=>{window.dispatchEvent(new CustomEvent(EXTENSION_RESOURCE_CHANGED,{detail:resource}));return {requested:true};});
 expect(screen.getByText("Source reference")).toBeTruthy();
 expect(screen.getByText("platform-config")).toBeTruthy();
 expect(screen.queryByText("View fields")).toBeNull();
 fireEvent.click(screen.getByRole("button",{name:"Suspend"}));
 expect(actOnExtensionResource).not.toHaveBeenCalled();
 expect(screen.getByRole("dialog").textContent).toContain("cluster/a");
 fireEvent.click(screen.getByRole("button",{name:"Confirm Suspend"}));
 await waitFor(()=>expect(actOnExtensionResource).toHaveBeenCalledWith(selection,"suspend","uid-a","12"));
 expect((await screen.findByRole("status")).textContent).toContain("Request accepted");
 await waitFor(()=>expect(inspectExtensionResource).toHaveBeenCalledTimes(2));
});
it("cancels without writing and keeps failed writes distinct from success",async()=>{
 render(<ExtensionResourceDetails selection={selection} onClose={vi.fn()}/>);
 fireEvent.click(await screen.findByRole("button",{name:"Suspend"}));
 fireEvent.click(screen.getByRole("button",{name:"Cancel"}));expect(actOnExtensionResource).not.toHaveBeenCalled();
 vi.mocked(actOnExtensionResource).mockRejectedValue(new Error("Resource changed; refresh"));
 fireEvent.click(screen.getByRole("button",{name:"Suspend"}));fireEvent.click(screen.getByRole("button",{name:"Confirm Suspend"}));
 expect(await screen.findByText("Resource changed; refresh")).toBeTruthy();expect(screen.queryByText(/Request accepted/)).toBeNull();
});
it("does not interpret a failed detail request as a missing resource",async()=>{
 vi.mocked(inspectExtensionResource).mockRejectedValue(new Error("Access denied"));
 render(<ExtensionResourceDetails selection={selection} onClose={vi.fn()}/>);
 expect(await screen.findByText("Access denied")).toBeTruthy();expect(screen.queryByRole("button",{name:"Suspend"})).toBeNull();
});

it("shows the manifest and events, offers Resume for a suspended resource, and rejects unacknowledged writes",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,resource:{...detail.resource,spec:{suspend:true}},events:[{type:"Warning",reason:"Error",message:"Source unavailable",count:2}]});
  render(<ExtensionResourceDetails selection={selection} onClose={vi.fn()}/>);
  expect(await screen.findByText("Source unavailable")).toBeTruthy();
  expect(screen.queryByText(/Showing the latest/)).toBeNull();
  expect(screen.queryByRole("button",{name:"Suspend"})).toBeNull();
  expect((screen.getByRole("button",{name:"Reconcile"}) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole("tab",{name:"Manifest"}));expect(screen.getByRole("textbox",{name:"apps manifest"}).getAttribute("contenteditable")).toBe("false");
  expect(screen.getByRole("textbox",{name:"apps manifest"}).textContent).toContain("kind: Kustomization");
  vi.mocked(actOnExtensionResource).mockResolvedValue({requested:false});
  fireEvent.click(screen.getByRole("button",{name:"Resume"}));fireEvent.click(screen.getByRole("button",{name:"Confirm Resume"}));
  expect(await screen.findByText(/not acknowledged/)).toBeTruthy();
});
it("says when the host returned only the latest events",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,events:[{type:"Normal",reason:"Progressing",message:"Applied revision",count:1}],eventsTruncated:true});
  render(<ExtensionResourceDetails selection={selection}/>);
  expect(await screen.findByText("Applied revision")).toBeTruthy();
  expect(screen.getByText("Showing the latest 100 events.")).toBeTruthy();
});
it("refreshes when any view reports an accepted action on this same resource",async()=>{
  render(<ExtensionResourceDetails selection={selection} fullPage/>);
  expect(await screen.findByText("Missing source")).toBeTruthy();
  const changed=(detail:object)=>window.dispatchEvent(new CustomEvent(EXTENSION_RESOURCE_CHANGED,{detail}));
  changed({...selection,name:"other"});
  changed({...selection,context:"cluster/b"});
  changed({...selection,id:"org.other.app"});
  changed({...selection,namespace:"other"});
  expect(inspectExtensionResource).toHaveBeenCalledTimes(1);
  changed(selection);
  await waitFor(()=>expect(inspectExtensionResource).toHaveBeenCalledTimes(2));
});
it("preserves the overview when event access fails and closes with Escape",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,eventsError:"Events access denied"});
  const close=vi.fn();render(<ExtensionResourceDetails selection={selection} onClose={close}/>);
  expect(await screen.findByText("Missing source")).toBeTruthy();expect(screen.getByText("Events access denied")).toBeTruthy();
  fireEvent.keyDown(screen.getByText("Missing source"),{key:"Escape"});expect(close).toHaveBeenCalledOnce();
});

it("promotes a peek to its own tab while keeping the list's close control separate",async()=>{
 const {ExtensionResourceNavigation}=await import("./resourceNavigation");const open=vi.fn(),close=vi.fn();
 const view=render(<ExtensionResourceNavigation.Provider value={open}><ExtensionResourceDetails selection={selection} onClose={close}/></ExtensionResourceNavigation.Provider>);
 fireEvent.click(await screen.findByRole("button",{name:"Open tab"}));expect(open).toHaveBeenCalledWith(selection);expect(close).not.toHaveBeenCalled();
 fireEvent.click(screen.getByRole("button",{name:"Close inspector"}));expect(close).toHaveBeenCalledOnce();
 view.rerender(<ExtensionResourceNavigation.Provider value={open}><ExtensionResourceDetails fullPage selection={selection}/></ExtensionResourceNavigation.Provider>);
 expect(screen.queryByRole("button",{name:"Open tab"})).toBeNull();expect(screen.queryByRole("button",{name:"Close inspector"})).toBeNull();
});

it("renders inventory entries as a full-width table instead of a JSON block",async()=>{
 vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,resource:{...detail.resource,status:{inventory:{entries:[{id:"team_service__Service",v:"v1"},{id:"team_api_apps_Deployment",v:"v1"}]}}}});
 render(<ExtensionResourceDetails selection={selection}/>);
 const disclosure=await screen.findByText("Entries · 2 entries");
 fireEvent.click(disclosure);
 expect(await screen.findByRole("table",{name:"Entries"})).toBeTruthy();
 expect(screen.getByRole("columnheader",{name:"ID"})).toBeTruthy();
 expect(screen.getByRole("columnheader",{name:"Version"})).toBeTruthy();
 expect(screen.getByRole("cell",{name:"team_service__Service"})).toBeTruthy();
 expect(screen.getByRole("cell",{name:"team_api_apps_Deployment"})).toBeTruthy();
});
