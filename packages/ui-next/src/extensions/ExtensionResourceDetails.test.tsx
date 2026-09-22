import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async original => ({...await original<typeof import("@srelens/core")>(),inspectExtensionResource:vi.fn(),actOnExtensionResource:vi.fn(),listExtensions:vi.fn()}));
import { inspectExtensionResource, actOnExtensionResource, listExtensions, EXTENSION_RESOURCE_CHANGED } from "@srelens/core";
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
  // A read-only manifest a reader opens in order to take it away. (#656 review)
  expect(screen.getByRole("button",{name:"Copy"})).toBeTruthy();
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
it("confirms against the version the reader reviewed even after another view refreshes it",async()=>{
  render(<ExtensionResourceDetails selection={selection}/>);
  fireEvent.click(await screen.findByRole("button",{name:"Suspend"}));
  // Another view acts on the same resource while this review is still open.
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,resource:{...detail.resource,metadata:{...detail.resource.metadata,resourceVersion:"13"}}});
  window.dispatchEvent(new CustomEvent(EXTENSION_RESOURCE_CHANGED,{detail:selection}));
  await waitFor(()=>expect(inspectExtensionResource).toHaveBeenCalledTimes(2));
  fireEvent.click(await screen.findByRole("button",{name:"Confirm Suspend"}));
  // The reviewed version goes to the host, whose stale-review guard then rejects it.
  await waitFor(()=>expect(actOnExtensionResource).toHaveBeenCalledWith(selection,"suspend","uid-a","12"));
});
it("does not report no events when the host stopped before reading every page",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,events:[],eventsTruncated:true,eventsPartial:true,eventsRead:0});
  render(<ExtensionResourceDetails selection={selection}/>);
  expect(await screen.findByText("No events were returned before the host stopped reading. This resource has more events that were not read.")).toBeTruthy();
  expect(screen.queryByText("No events reported.")).toBeNull();
});
it("does not claim the latest events when the host stopped before reading them all",async()=>{
  // The host reports what it actually read; the panel must not invent a total.
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,events:[{type:"Normal",reason:"Progressing",message:"Applied revision",count:1}],eventsTruncated:true,eventsPartial:true,eventsRead:5000});
  render(<ExtensionResourceDetails selection={selection}/>);
  expect(await screen.findByText("Applied revision")).toBeTruthy();
  expect(screen.getByText("Showing the newest 1 of 5,000 events read. This resource has more events that were not read.")).toBeTruthy();
  expect(screen.queryByText("Showing the latest 100 events.")).toBeNull();
});
it("refreshes when any view reports an accepted action on this same resource",async()=>{
  render(<ExtensionResourceDetails selection={selection} fullPage/>);
  expect(await screen.findByText("Missing source")).toBeTruthy();
  const changed=(detail:object)=>window.dispatchEvent(new CustomEvent(EXTENSION_RESOURCE_CHANGED,{detail}));
  // Same app, cluster, namespace and name, but a different kind (e.g. a GitRepository).
  changed({...selection,capability:"gitrepositories"});
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

// ---- #552: one host-owned confirmation, for this write and the agent's -----

/**
 * The inventory the requester line is read from. `useExtensions` polls the
 * host only under a Tauri runtime, so the marker goes up with the stub.
 */
function installedApps(plugins:unknown[]) {
 (window as unknown as Record<string,unknown>).__TAURI_INTERNALS__={};
 vi.mocked(listExtensions).mockResolvedValue({schemaVersion:1,nextRevision:1,plugins} as never);
}
const fluxApp={manifest:{id:"org.srelens.flux",name:"Flux Tools",version:"1.0.0",srelensApiVersion:"1",kind:"declarative",permissions:[],capabilities:[],contributions:{pages:[],detailTabs:[],detailLinks:[]}},enabled:true,revision:1,grants:[],settings:{},source:"local",installedAt:0,history:[]};
const withMeta={...detail,actionMeta:{suspend:{impact:"high" as const,confirm:"Suspend[ {resource}][ in cluster {cluster}]?"}}};
afterEach(()=>{delete (window as unknown as Record<string,unknown>).__TAURI_INTERNALS__;});

it("asks the host's own sentence and level, not a description written in the UI",async()=>{
 vi.mocked(inspectExtensionResource).mockResolvedValue(withMeta);
 render(<ExtensionResourceDetails selection={selection}/>);
 fireEvent.click(await screen.findByRole("button",{name:"Suspend"}));
 const dialog=screen.getByRole("dialog");
 expect(dialog.textContent).toContain("Suspend Kustomization team/apps in cluster cluster/a?");
 expect(dialog.textContent).toContain("High impact");
 // The words the UI used to keep in a constant beside the buttons are gone.
 expect(dialog.textContent).not.toContain("Pause future reconciliation");
});

it("names the pinned cluster and the resource under the question",async()=>{
 vi.mocked(inspectExtensionResource).mockResolvedValue(withMeta);
 render(<ExtensionResourceDetails selection={selection}/>);
 fireEvent.click(await screen.findByRole("button",{name:"Suspend"}));
 expect(screen.getByTestId("host-confirm-cluster").textContent).toBe("cluster/a");
 expect(screen.getByTestId("host-confirm-target").textContent).toBe("team/apps");
});

it("says which app asked and that it carries no signature",async()=>{
 installedApps([fluxApp]);
 vi.mocked(inspectExtensionResource).mockResolvedValue(withMeta);
 render(<ExtensionResourceDetails selection={selection}/>);
 fireEvent.click(await screen.findByRole("button",{name:"Suspend"}));
 await waitFor(()=>expect(screen.getByTestId("host-confirm-requester").textContent)
   .toBe("Requested by app Flux Tools (unsigned)"));
});

it("will not let an app's own name reorder or overflow the question",async()=>{
 installedApps([{...fluxApp,manifest:{...fluxApp.manifest,name:`Flux‮${"x".repeat(300)}`}}]);
 vi.mocked(inspectExtensionResource).mockResolvedValue(withMeta);
 render(<ExtensionResourceDetails selection={selection}/>);
 fireEvent.click(await screen.findByRole("button",{name:"Suspend"}));
 await waitFor(()=>expect(screen.queryByTestId("host-confirm-app-name")).toBeTruthy());
 const drawn=screen.getByTestId("host-confirm-app-name").textContent ?? "";
 expect(drawn).not.toContain("‮");
 expect([...drawn].length).toBe(80);
 expect(screen.getByRole("dialog").textContent).toContain("Suspend Kustomization team/apps in cluster cluster/a?");
});

it("falls back to no sentence rather than half of one when the host authored none",async()=>{
 render(<ExtensionResourceDetails selection={selection}/>);
 fireEvent.click(await screen.findByRole("button",{name:"Suspend"}));
 const dialog=screen.getByRole("dialog");
 expect(screen.queryByTestId("host-confirm-question")).toBeNull();
 // Absences alone would pass against the OLD review too — it had no
 // `host-confirm-question` either, and it also printed the context. So the
 // test is anchored on what only the new one does: the facts are drawn as
 // the one confirmation's own rows, and the description the UI used to keep
 // in a constant beside the buttons is gone.
 expect(screen.getByTestId("host-confirm-cluster").textContent).toBe("cluster/a");
 expect(screen.getByTestId("host-confirm-target").textContent).toBe("team/apps");
 expect(dialog.textContent).not.toContain("Pause future reconciliation");
 expect(dialog.textContent).not.toMatch(/undefined|null/);
});

/**
 * The app the reader is told asked must be the one the host will actually run
 * the action as. An update while this review is open moves the installed
 * revision past the one the selection pinned, and `resolve`
 * (`crates/registry/src/extensions/resource.rs`) would refuse that action —
 * so the line is dropped rather than naming a version that did not ask.
 */
it("names no app once it has been replaced under an open review",async()=>{
 installedApps([{...fluxApp,revision:9}]);
 vi.mocked(inspectExtensionResource).mockResolvedValue(withMeta);
 render(<ExtensionResourceDetails selection={selection}/>);
 fireEvent.click(await screen.findByRole("button",{name:"Suspend"}));
 const dialog=screen.getByRole("dialog");
 // The new confirmation IS drawn, so this is about attribution and not about
 // the review failing to render.
 expect(dialog.textContent).toContain("Suspend Kustomization team/apps in cluster cluster/a?");
 await waitFor(()=>expect(listExtensions).toHaveBeenCalled());
 expect(screen.queryByTestId("host-confirm-requester")).toBeNull();
 expect(dialog.textContent).not.toContain("Flux Tools");
});
