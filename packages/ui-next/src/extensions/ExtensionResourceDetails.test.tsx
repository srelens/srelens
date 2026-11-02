import fluxManifest from "../../../../examples/extensions/flux.json";
import argoManifest from "../../../../examples/extensions/argocd.json";
const declaredMeta = Object.fromEntries([...fluxManifest.actions.filter(action=>action.resource==="helmreleases").map(action=>({...action,name:action.name.replace("helmreleases-","")})),...argoManifest.actions].map(action=>[action.name,{title:action.title,availableWhen:("availableWhen" in action?action.availableWhen:[]) as import("@srelens/core").ActionPredicate[],impact:"medium" as const,confirm:null}]));
import { act, render as bare, screen, fireEvent, waitFor, within } from "@testing-library/react";
import type { ReactElement, ReactNode } from "react";
import { afterEach, beforeEach, expect, it, onTestFinished, vi } from "vitest";
vi.mock("@srelens/core", async original => ({...await original<typeof import("@srelens/core")>(),inspectExtensionResource:vi.fn(),actOnExtensionResource:vi.fn(),listExtensions:vi.fn(),resolveExtensionPanels:vi.fn(),resolveExtensionLinks:vi.fn(),resolveExtensionReverseLinks:vi.fn()}));
import { inspectExtensionResource, actOnExtensionResource, listExtensions, resolveExtensionPanels, resolveExtensionLinks, resolveExtensionReverseLinks, EXTENSION_RESOURCE_CHANGED } from "@srelens/core";
import { ExtensionResourceDetails } from "./ExtensionResourceDetails";
import { requestExtensionAction } from "./actionRequests";
import type { ClusterContext } from "@srelens/core";
import { ConsoleProvider } from "../console";
import { askQuestion } from "../lib/kinds/rowAffordances";
import { isSectionOpen, loadSectionFolds, setSectionOpen } from "../lib/sectionFolds";
import { resetContexts, setContexts } from "../lib/clusters";
import { defaultState } from "../lib/tabs";
import { getState, setState, setWorkspaceProtected } from "../lib/tabsStore";
const selection = {id:"org.srelens.flux",revision:1,capability:"kustomizations",context:"cluster/a",namespace:"team",name:"apps"};
const detail = {resource:{apiVersion:"kustomize.toolkit.fluxcd.io/v1",kind:"Kustomization",metadata:{name:"apps",namespace:"team",uid:"uid-a",resourceVersion:"12"},spec:{suspend:false,path:"./apps",sourceRef:{kind:"GitRepository",name:"platform-config"}},status:{conditions:[{type:"Ready",status:"False",reason:"BuildFailed",message:"Missing source"}],lastAppliedRevision:"main@sha1:abcdef"}},actions:["suspend","resume","reconcile"],actionMeta:declaredMeta};
const Wrapper=({children}:{children:ReactNode})=><ConsoleProvider>{children}</ConsoleProvider>;
const render=(ui:ReactElement)=>bare(ui,{wrapper:Wrapper});
// Conditions and Status open shut on a first visit, like every titled block of a
// built-in detail. Most of these tests are about what is inside them.
const forgetFolds=()=>{localStorage.clear();loadSectionFolds();};
beforeEach(()=>{vi.resetAllMocks();vi.mocked(inspectExtensionResource).mockResolvedValue(detail);vi.mocked(actOnExtensionResource).mockResolvedValue({requested:true});forgetFolds();setSectionOpen("Kustomization","Conditions",true);setSectionOpen("Kustomization","Status",true);});
it("shows overview and conditions, then confirms the exact pinned resource before requesting an action",async()=>{
 render(<ExtensionResourceDetails selection={selection} onClose={vi.fn()}/>);
 expect(await screen.findByText("Missing source")).toBeTruthy();
 expect(screen.getByText("./apps")).toBeTruthy();
 // Like the real wrapper, an accepted action announces the resource; the view refreshes from that.
 vi.mocked(actOnExtensionResource).mockImplementation(async resource=>{window.dispatchEvent(new CustomEvent(EXTENSION_RESOURCE_CHANGED,{detail:resource}));return {requested:true};});
 expect(screen.getByText("Source ref")).toBeTruthy();
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
  render(<ExtensionResourceDetails selection={selection} fullPage/>);
  fireEvent.click(await screen.findByRole("tab",{name:"Events"}));
  expect(await screen.findByText("Source unavailable")).toBeTruthy();
  expect(screen.queryByText(/Showing the latest/)).toBeNull();
  // Both stay on screen and say why they do not apply here (#550).
  expect(screen.getByRole("button",{name:"Suspend"}).getAttribute("aria-disabled")).toBe("true");
  expect(screen.getByRole("button",{name:"Reconcile"}).getAttribute("aria-disabled")).toBe("true");
  fireEvent.click(screen.getByRole("tab",{name:"YAML"}));expect(screen.getByRole("textbox",{name:"apps manifest"}).getAttribute("contenteditable")).toBe("false");
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
  fireEvent.click(await screen.findByRole("tab",{name:"Events"}));
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
  fireEvent.click(await screen.findByRole("tab",{name:"Events"}));
  expect(await screen.findByText("No events were returned before the host stopped reading. This resource has more events that were not read.")).toBeTruthy();
  expect(screen.queryByText("No events reported.")).toBeNull();
});
it("does not claim the latest events when the host stopped before reading them all",async()=>{
  // The host reports what it actually read; the panel must not invent a total.
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,events:[{type:"Normal",reason:"Progressing",message:"Applied revision",count:1}],eventsTruncated:true,eventsPartial:true,eventsRead:5000});
  render(<ExtensionResourceDetails selection={selection}/>);
  fireEvent.click(await screen.findByRole("tab",{name:"Events"}));
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
  expect(await screen.findByText("Missing source")).toBeTruthy();
  fireEvent.keyDown(screen.getByText("Missing source"),{key:"Escape"});expect(close).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole("tab",{name:"Events"}));
  expect(screen.getByText("Events access denied")).toBeTruthy();
  fireEvent.click(screen.getByRole("button",{name:/retry/i}));
  await waitFor(()=>expect(inspectExtensionResource).toHaveBeenCalledTimes(2));
});

it("promotes a peek to its own tab while keeping the list's close control separate",async()=>{
 const {ExtensionResourceNavigation}=await import("./resourceNavigation");const open=vi.fn(),close=vi.fn();
 const view=render(<ExtensionResourceNavigation.Provider value={open}><ExtensionResourceDetails selection={selection} onClose={close}/></ExtensionResourceNavigation.Provider>);
 fireEvent.click(await screen.findByRole("button",{name:"Open tab"}));expect(open).toHaveBeenCalledWith(selection);expect(close).not.toHaveBeenCalled();
 fireEvent.click(screen.getByRole("button",{name:"Close inspector"}));expect(close).toHaveBeenCalledOnce();
 view.rerender(<ExtensionResourceNavigation.Provider value={open}><ExtensionResourceDetails fullPage selection={selection}/></ExtensionResourceNavigation.Provider>);
 expect(screen.queryByRole("button",{name:"Open tab"})).toBeNull();expect(screen.queryByRole("button",{name:"Close inspector"})).toBeNull();
});

// #550: an action's availability is a declared predicate over the resource,
// evaluated by the same rules the host applies before it writes. A control the
// rules exclude stays on screen, disabled, saying why — a button that vanishes
// teaches nothing, and a button that is merely grey teaches no more.
/** The description a screen reader reads for `control`, as the tree resolves it. */
function describedBy(control:HTMLElement) {
  const ids=(control.getAttribute("aria-describedby")??"").split(/\s+/).filter(Boolean);
  return ids.map(id=>document.getElementById(id)?.textContent??"").join(" ");
}
it("disables an action its availability rules exclude and gives the reason as its tooltip",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,resource:{...detail.resource,spec:{suspend:true}}});
  render(<ExtensionResourceDetails selection={selection} fullPage/>);
  const reconcile=await screen.findByRole("button",{name:"Reconcile"});
  expect(reconcile.getAttribute("aria-disabled")).toBe("true");
  expect(reconcile.getAttribute("title")).toBe("Resume this resource before requesting reconciliation");
  // Excluded means no review opens, so nothing can be confirmed into a write.
  fireEvent.click(reconcile);
  expect(screen.queryByRole("dialog")).toBeNull();
  const suspend=screen.getByRole("button",{name:"Suspend"});
  expect(suspend.getAttribute("aria-disabled")).toBe("true");
  expect(suspend.getAttribute("title")).toBe("This resource is already suspended");
  // Resume is the one that applies, and carries no excuse.
  const resume=screen.getByRole("button",{name:"Resume"});
  expect(resume.getAttribute("aria-disabled")).toBeNull();
  expect(resume.getAttribute("title")).toBeNull();
  fireEvent.click(resume);
  expect(screen.getByRole("dialog")).toBeTruthy();
});
// `title` draws a tooltip on hover and is only a *fallback* description, so a
// sighted keyboard user reaches the dimmed control and is told nothing. The
// reason is an element the control names, which is what a focus ring can
// reveal and a screen reader always reads. (#668 review)
it("names the reason from the control, so focus reaches it without a pointer",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,resource:{...detail.resource,spec:{suspend:true}},actions:["suspend","resume","reconcile","force","reset"]});
  render(<ExtensionResourceDetails selection={selection} fullPage/>);
  const reconcile=await screen.findByRole("button",{name:"Reconcile"});
  expect(describedBy(reconcile)).toBe("Resume this resource before requesting reconciliation");
  expect(describedBy(screen.getByRole("button",{name:"Suspend"}))).toBe("This resource is already suspended");
  // The reason is in the accessibility tree, not hidden from it.
  const note=document.getElementById(reconcile.getAttribute("aria-describedby")!)!;
  expect(note.getAttribute("aria-hidden")).toBeNull();
  expect(note.hasAttribute("hidden")).toBe(false);
  // A control that applies describes nothing: there is nothing to say.
  const resume=screen.getByRole("button",{name:"Resume"});
  expect(resume.getAttribute("aria-describedby")).toBeNull();
  // Each unavailable control names its own reason, never a shared one.
  const ids=["Suspend","Reconcile","Force reconcile"].map(name=>screen.getByRole("button",{name}).getAttribute("aria-describedby"));
  expect(new Set(ids).size).toBe(3);
});
it("does not offer an Argo CD sync while an operation is already running",async()=>{
  // The host refuses this (`gitops.rs`); stating it as a predicate is what
  // lets the surface say so before a person asks for the write.
  const application={apiVersion:"argoproj.io/v1alpha1",kind:"Application",metadata:{name:"apps",namespace:"team",uid:"uid-a",resourceVersion:"12"},spec:{},status:{},operation:{sync:{revision:"HEAD"}}};
  vi.mocked(inspectExtensionResource).mockResolvedValue({resource:application,actions:["sync","refresh"],actionMeta:declaredMeta});
  render(<ExtensionResourceDetails selection={selection}/>);
  const sync=await screen.findByRole("button",{name:"Sync"});
  expect(sync.getAttribute("aria-disabled")).toBe("true");
  expect(sync.getAttribute("title")).toBe("An Argo CD operation is already in progress");
  expect(screen.getByRole("button",{name:"Refresh status"}).getAttribute("aria-disabled")).toBeNull();
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

it("renders a declared app panel after the host overview sections", async () => {
 const app={...fluxApp,manifest:{...fluxApp.manifest,contributions:{...fluxApp.manifest.contributions,detailPanels:[{
  id:"summary",title:"App summary",forKinds:["kustomize.toolkit.fluxcd.io/Kustomization"],
  sections:[{type:"fields",fields:[{label:"Path",jsonPath:".spec.path"}]}],
 }]}}};
 installedApps([app]);
 vi.mocked(resolveExtensionPanels).mockResolvedValue({panels:[{id:"summary",title:"App summary",sections:[
  {type:"fields",fields:[{label:"Path",value:"./apps"},{label:"Missing",value:null}]},
 ]}]});
 render(<ExtensionResourceDetails selection={selection}/>);
 expect(await screen.findByRole("heading",{name:"App summary"})).toBeTruthy();
 expect(screen.getByText("Missing").parentElement?.textContent).toContain("—");
 expect(resolveExtensionPanels).toHaveBeenCalledWith("org.srelens.flux",1,"cluster/a","team",
  "kustomize.toolkit.fluxcd.io/Kustomization",detail.resource);
});
it("shows the app resource's Related links after its host sections (#545)", async () => {
 const app={...fluxApp,manifest:{...fluxApp.manifest,
  capabilities:[{name:"gitrepositories",title:"Git repositories",target:"k8s.listCustomResource",
   arguments:{group:"source.toolkit.fluxcd.io",kind:"GitRepository"},inputs:["context","namespace"]}],
  contributions:{...fluxApp.manifest.contributions,
   pages:[{id:"gitrepositories",title:"Git repositories",capability:"gitrepositories"}],
   resourceLinks:[{id:"source",from:"kustomize.toolkit.fluxcd.io/Kustomization",to:"source.toolkit.fluxcd.io/GitRepository",
    relation:"references",match:{label:"example.io/source"}}]}}};
 installedApps([app]);
 // The page's context is listed: its links' routes carry its key (#695).
 const {resetContexts,setContexts}=await import("../lib/clusters");
 setContexts([{name:"cluster/a",stableId:"cluster/a",key:"cluster/a"} as import("@srelens/core").ClusterContext]);
 onTestFinished(resetContexts);
 vi.mocked(resolveExtensionPanels).mockResolvedValue({panels:[]});
 vi.mocked(resolveExtensionLinks).mockResolvedValue({
  from:{kind:"kustomize.toolkit.fluxcd.io/Kustomization",namespace:"team",name:"apps"},
  links:[{id:"source",relation:"references",to:"source.toolkit.fluxcd.io/GitRepository",capability:"gitrepositories",
   targets:[{namespace:"team",name:"platform-config",exists:true}]}]});
 render(<ExtensionResourceDetails selection={selection}/>);
 const related = await screen.findByRole("region",{name:"Related"});
 expect(await screen.findByRole("button",{name:"References GitRepository team/platform-config"})).toBeTruthy();
 expect(related.textContent).toContain("team/platform-config");
 // Only identity and metadata go to the resolver; the Kustomization's spec does not.
 expect(resolveExtensionLinks).toHaveBeenCalledWith("org.srelens.flux",1,"cluster/a","team",
  "kustomize.toolkit.fluxcd.io/Kustomization",
  {apiVersion:detail.resource.apiVersion,kind:detail.resource.kind,metadata:detail.resource.metadata});
});
it("shows what names the app resource, read the other way round, on its page (#728)", async () => {
 const app={...fluxApp,manifest:{...fluxApp.manifest,
  contributions:{...fluxApp.manifest.contributions,
   resourceLinks:[{id:"kustomization",from:"apps/Deployment",to:"kustomize.toolkit.fluxcd.io/Kustomization",
    relation:"managedBy",match:{label:"kustomize.toolkit.fluxcd.io/name",namespaceLabel:"kustomize.toolkit.fluxcd.io/namespace"}}]}}};
 installedApps([app]);
 const {resetContexts,setContexts}=await import("../lib/clusters");
 setContexts([{name:"cluster/a",stableId:"cluster/a",key:"cluster/a"} as import("@srelens/core").ClusterContext]);
 onTestFinished(resetContexts);
 vi.mocked(resolveExtensionPanels).mockResolvedValue({panels:[]});
 vi.mocked(resolveExtensionReverseLinks).mockResolvedValue({
  to:{kind:"kustomize.toolkit.fluxcd.io/Kustomization",namespace:"team",name:"apps"},
  links:[{id:"kustomization",relation:"managedBy",from:"apps/Deployment",capability:"",truncated:false,
   sources:[{namespace:"team",name:"api",exists:true}]}]});
 render(<ExtensionResourceDetails selection={selection}/>);
 const manages = await screen.findByRole("group",{name:"Manages"});
 expect(manages.textContent).toContain("Deployment team/api");
 // No link starts from a Kustomization here, so only the reverse view is asked, with identity alone.
 expect(resolveExtensionLinks).not.toHaveBeenCalled();
 expect(resolveExtensionReverseLinks).toHaveBeenCalledWith("org.srelens.flux",1,"cluster/a","team",
  "kustomize.toolkit.fluxcd.io/Kustomization",
  {apiVersion:detail.resource.apiVersion,kind:detail.resource.kind,metadata:detail.resource.metadata});
});
const withMeta={...detail,actionMeta:{...declaredMeta,suspend:{...declaredMeta.suspend,impact:"high" as const,confirm:"Suspend[ {resource}][ in cluster {cluster}]?"}}};
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

it("offers a newly declared action title and predicates without a host GitOps name table", async () => {
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail, actions:["request-review"], actionMeta:{"request-review":{title:"Request review", impact:"medium", confirm:"Request review[ of {resource}]?", availableWhen:[{jsonPath:".spec.suspend", equals:true, reason:"Suspend before requesting review"}]}}} as any);
  render(<ExtensionResourceDetails selection={selection}/>);
  const action = await screen.findByRole("button", {name:"Request review"});
  expect(action.getAttribute("aria-disabled")).toBe("true");
  fireEvent.click(action);
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(action.getAttribute("title")).toBe("Suspend before requesting review");
});

it("draws conditions as a built-in resource's, with the tone of each",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,resource:{...detail.resource,status:{conditions:[{type:"Ready",status:"True",reason:"Applied",message:"All good"},{type:"Stalled",status:"True",reason:"Stuck"}]}}});
  render(<ExtensionResourceDetails selection={selection}/>);
  const ready=await screen.findByText("Ready");
  expect(ready.closest(".status")?.getAttribute("data-kind")).toBe("success");
  expect(screen.getByText("Stalled").closest(".status")?.getAttribute("data-kind")).toBe("danger");
  expect(screen.getByText("All good")).toBeTruthy();
});
// Argo CD writes {type, message, lastTransitionTime} and no status, which the
// bounded catalog refused whole: "Could not display Conditions" (#1014).
it("shows an operator's condition that carries no status",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,resource:{...detail.resource,status:{conditions:[{type:"ComparisonError",message:"rpc error",lastTransitionTime:"2026-09-22T12:00:00Z"}]}}});
  render(<ExtensionResourceDetails selection={selection}/>);
  expect(await screen.findByText("rpc error")).toBeTruthy();
  expect(screen.getByText("ComparisonError")).toBeTruthy();
  expect(screen.queryByRole("alert")).toBeNull();
});

// #101: on a protected cluster the review's Confirm waits for the cluster's
// name, typed. An app page names its cluster by its pinned ID, which is what
// this selection carries.
it("holds Confirm on a protected cluster until its name is typed, and forgets it with the review",async()=>{
  const ctx:ClusterContext={name:"cluster/a",stableId:"a",key:"a",pinnedId:"srelens-context:/home/u/.kube/config#cluster%2Fa",cluster:"a",server:"",isCurrent:true,sourceFile:"/home/u/.kube/config",authKind:"token"};
  setContexts([ctx]);setState(defaultState([ctx]));setWorkspaceProtected(getState().currentId,true);
  onTestFinished(()=>{resetContexts();setState(defaultState([]));});
  const pinned={...selection,context:ctx.pinnedId!};
  render(<ExtensionResourceDetails selection={pinned}/>);
  fireEvent.click(await screen.findByRole("button",{name:"Suspend"}));
  expect(screen.getByText("cluster/a is protected (workspace Default).")).toBeTruthy();
  const confirm=()=>screen.getByRole("button",{name:"Confirm Suspend"}) as HTMLButtonElement;
  const typed=()=>screen.getByRole("textbox",{name:"Type cluster/a to confirm"}) as HTMLInputElement;
  expect(confirm().disabled).toBe(true);
  fireEvent.change(typed(),{target:{value:"cluster/a"}});
  expect(confirm().disabled).toBe(false);
  // Cancelled and asked again: the name was for that review.
  fireEvent.click(screen.getByRole("button",{name:"Cancel"}));
  fireEvent.click(screen.getByRole("button",{name:"Suspend"}));
  expect(typed().value).toBe("");
  expect(confirm().disabled).toBe(true);
  fireEvent.change(typed(),{target:{value:"cluster/a"}});
  fireEvent.click(confirm());
  await waitFor(()=>expect(actOnExtensionResource).toHaveBeenCalledWith(pinned,"suspend","uid-a","12"));
});
it("asks again when a palette request replaces the open review",async()=>{
  // The palette can raise a review while another is open (#544), so the review
  // is not always closed between two actions: the name typed for Suspend does
  // not answer Reconcile.
  const ctx:ClusterContext={name:"cluster/a",stableId:"a",key:"a",pinnedId:"srelens-context:/home/u/.kube/config#cluster%2Fa",cluster:"a",server:"",isCurrent:true,sourceFile:"/home/u/.kube/config",authKind:"token"};
  setContexts([ctx]);setState(defaultState([ctx]));setWorkspaceProtected(getState().currentId,true);
  onTestFinished(()=>{resetContexts();setState(defaultState([]));});
  const pinned={...selection,context:ctx.pinnedId!};
  render(<ExtensionResourceDetails selection={pinned} fullPage/>);
  fireEvent.click(await screen.findByRole("button",{name:"Suspend"}));
  const typed=()=>screen.getByRole("textbox",{name:"Type cluster/a to confirm"}) as HTMLInputElement;
  fireEvent.change(typed(),{target:{value:"cluster/a"}});
  expect((screen.getByRole("button",{name:"Confirm Suspend"}) as HTMLButtonElement).disabled).toBe(false);
  act(()=>requestExtensionAction({id:pinned.id,capability:pinned.capability,context:pinned.context,namespace:pinned.namespace,name:pinned.name,action:"reconcile"}));
  const confirm=await screen.findByRole("button",{name:"Confirm Reconcile"}) as HTMLButtonElement;
  expect(typed().value).toBe("");
  expect(confirm.disabled).toBe(true);
  expect(actOnExtensionResource).not.toHaveBeenCalled();
});

// ---- #1001: an app's resource, in the built-in detail frame ----

const anHourAgo=()=>new Date(Date.now()-3_600_000).toISOString();
const stamped=()=>({...detail.resource,metadata:{...detail.resource.metadata,creationTimestamp:anHourAgo()}});
const failing={status:"error" as const,label:"Failing",reason:"Build failed"};
const peek=()=>render(<ExtensionResourceDetails selection={selection} onClose={vi.fn()}/>);
const slot=(name:string)=>document.querySelector(`[data-slot="${name}"]`);

it("heads a peek with the copyable name, the kind and namespace, and the status its list row shows",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,resource:stamped(),status:failing});
  peek();
  const pill=await screen.findByText("Failing");
  expect(pill.closest(".status")?.getAttribute("data-kind")).toBe("danger");
  expect(screen.getByRole("button",{name:"Copy name apps"})).toBeTruthy();
  expect(screen.getByText("Kustomization · team")).toBeTruthy();
  expect(screen.getByText("Age").parentElement?.textContent).toContain("1h");
});
it("names a cluster-scoped resource by its kind alone",async()=>{
  render(<ExtensionResourceDetails selection={{...selection,namespace:""}} onClose={vi.fn()}/>);
  expect(await screen.findByText("Kustomization")).toBeTruthy();
  expect(screen.queryByText(/Cluster-scoped/)).toBeNull();
});
it("claims no status for an app with no resolver, and gives its age as a fact",async()=>{
  forgetFolds();
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,resource:stamped()});
  peek();
  const created=await screen.findByText("Created");
  expect(created.parentElement?.textContent).toContain("1h ago");
  expect(screen.queryByText("Age")).toBeNull();
  expect(document.querySelector(".status")).toBeNull();
});
it("leads with only what the header has not already said",async()=>{
  const resource=stamped();
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,status:failing,resource:{...resource,metadata:{...resource.metadata,ownerReferences:[{kind:"Application",name:"root"}]}}});
  peek();
  await screen.findByText("Failing");
  for(const said of ["Name","Kind","Namespace","Created"])expect(screen.queryByText(said)).toBeNull();
  expect(screen.getByText("Controlled by").parentElement?.textContent).toContain("Application/root");
  expect(screen.getByText("Reason").parentElement?.textContent).toContain("Build failed");
  expect(screen.getByText("API").parentElement?.textContent).toContain("kustomize.toolkit.fluxcd.io/v1");
});
it("offers the same views, in one order, before the resource has loaded",()=>{
  vi.mocked(inspectExtensionResource).mockReturnValue(new Promise(()=>{}));
  peek();
  expect(screen.getAllByRole("tab").map(tab=>tab.textContent)).toEqual(["Overview","Events","YAML"]);
});
it("says a field in sentence case, leaves an acronym be, and keeps a map's own keys verbatim",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,resource:{...detail.resource,status:{},spec:{repoURL:"https://example.test",lastAppliedRevision:"main","example.com/owner":"team",values:{replicaCount:2,"image.tag":"v1"}}}});
  peek();
  expect(await screen.findByText("Repo URL")).toBeTruthy();
  expect(screen.getByText("Last applied revision")).toBeTruthy();
  expect(screen.getByText("example.com/owner")).toBeTruthy();
  expect(screen.getByText("Values")).toBeTruthy();
  expect(screen.getByText("replicaCount")).toBeTruthy();
  expect(screen.getByText("image.tag")).toBeTruthy();
});
it("folds deep data and long lists with the kit's section, never a bare disclosure",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,resource:{...detail.resource,spec:{source:{helm:{parameters:{image:"v1"}}}}}});
  peek();
  const fold=await screen.findByRole("button",{name:"Parameters"});
  expect(screen.queryByText(/"image"/)).toBeNull();
  fireEvent.click(fold);
  expect(screen.getByText(/"image": "v1"/)).toBeTruthy();
  expect(document.querySelector(".extension-resource-detail details")).toBeNull();
});
it("opens the spec on a first visit, shuts what built-in details shut, and remembers it per kind",async()=>{
  forgetFolds();
  peek();
  expect(await screen.findByText("./apps")).toBeTruthy();
  expect(screen.queryByText("Missing source")).toBeNull();
  expect(screen.queryByText("main@sha1:abcdef")).toBeNull();
  fireEvent.click(screen.getByRole("button",{name:"Conditions"}));
  expect(screen.getByText("Missing source")).toBeTruthy();
  expect(isSectionOpen("Kustomization","Conditions")).toBe(true);
});
it("draws labels and annotations as a built-in resource's, before the app's own panels",async()=>{
  const app={...fluxApp,manifest:{...fluxApp.manifest,contributions:{...fluxApp.manifest.contributions,detailPanels:[{
   id:"summary",title:"App summary",forKinds:["kustomize.toolkit.fluxcd.io/Kustomization"],sections:[],
  }]}}};
  installedApps([app]);
  vi.mocked(resolveExtensionPanels).mockResolvedValue({panels:[{id:"summary",title:"App summary",sections:[]}]});
  setSectionOpen("Kustomization","Labels",true);setSectionOpen("Kustomization","Annotations",true);
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,resource:{...detail.resource,metadata:{...detail.resource.metadata,labels:{"app.kubernetes.io/name":"apps"},annotations:{"kubectl.kubernetes.io/last-applied-configuration":"{\"kind\":\"the-whole-manifest\"}",owner:"team"}}}});
  peek();
  const panel=await screen.findByRole("heading",{name:"App summary"});
  expect(screen.getByText(/app\.kubernetes\.io\/name/)).toBeTruthy();
  expect(screen.getByText(/owner/)).toBeTruthy();
  expect(screen.queryByText(/the-whole-manifest/)).toBeNull();
  const labels=screen.getByRole("button",{name:"Labels"});
  expect(labels.compareDocumentPosition(panel)&Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});
it("lists events in the standard table, with their age",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,events:[{type:"Warning",reason:"BuildFailed",message:"Source unavailable",count:2,time:new Date(Date.now()-120_000).toISOString()}]});
  peek();
  fireEvent.click(await screen.findByRole("tab",{name:"Events"}));
  const row=(await screen.findByText("Source unavailable")).closest("tr")!;
  expect(row.textContent).toContain("BuildFailed");
  expect(row.textContent).toContain("Kustomization/apps");
  expect(row.textContent).toContain("2m");
  expect(within(row).getByText("Warning").closest(".status")).toBeTruthy();
});
it("says when no events were reported",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,events:[]});
  peek();
  fireEvent.click(await screen.findByRole("tab",{name:"Events"}));
  expect(await screen.findByText("No events")).toBeTruthy();
});
it("offers Ask beside the actions, asking after a flagged resource's trouble",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,status:failing});
  peek();
  await screen.findByText("Failing");
  expect(screen.getByRole("button",{name:`Ask: ${askQuestion("apps",true)}`})).toBeTruthy();
});
it("keeps two actions on a peek's bar and folds the rest, and Refresh, into More",async()=>{
  peek();
  expect(await screen.findByRole("button",{name:"Suspend"})).toBeTruthy();
  expect(screen.queryByRole("button",{name:"Reconcile"})).toBeNull();
  fireEvent.click(screen.getByRole("button",{name:"More actions"}));
  expect(await screen.findByRole("button",{name:"Reconcile"})).toBeTruthy();
  fireEvent.click(screen.getByRole("button",{name:"Refresh details"}));
  await waitFor(()=>expect(inspectExtensionResource).toHaveBeenCalledTimes(2));
});
it("keeps what it is showing through a Refresh, until the new read answers",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValueOnce({...detail,status:failing});
  peek();
  fireEvent.click(await screen.findByRole("button",{name:"More actions"}));
  let answer:(next:typeof detail&{status:typeof failing})=>void=()=>{};
  vi.mocked(inspectExtensionResource).mockReturnValueOnce(new Promise(resolve=>{answer=resolve;}));
  fireEvent.click(await screen.findByRole("button",{name:"Refresh details"}));
  await waitFor(()=>expect(inspectExtensionResource).toHaveBeenCalledTimes(2));
  expect(screen.getByText("Kustomization · team")).toBeTruthy();
  expect(screen.getByText("Failing")).toBeTruthy();
  expect(screen.queryByText(/^Loading/)).toBeNull();
  answer({...detail,status:{...failing,label:"Still failing"}});
  expect(await screen.findByText("Still failing")).toBeTruthy();
});
it("stays where it is when another view's action changes the resource",async()=>{
  peek();
  await screen.findByText("Kustomization · team");
  vi.mocked(inspectExtensionResource).mockReturnValueOnce(new Promise(()=>{}));
  act(()=>{window.dispatchEvent(new CustomEvent(EXTENSION_RESOURCE_CHANGED,{detail:selection}));});
  await waitFor(()=>expect(inspectExtensionResource).toHaveBeenCalledTimes(2));
  expect(screen.getByText("Kustomization · team")).toBeTruthy();
  expect(screen.queryByText(/^Loading/)).toBeNull();
});
it("rules the metadata row off from whatever follows it",async()=>{
  render(<ExtensionResourceDetails selection={selection} fullPage/>);
  await screen.findByRole("heading",{level:1,name:"apps"});
  await waitFor(()=>expect(slot("metadata-pair")).toBeTruthy());
  // The row is a wrapper, so `.section + .section` does not reach the block after it.
  expect(slot("metadata-pair")?.className).toContain("[&+.section]:border-t");
});
it("draws a high-impact action in the danger tone",async()=>{
  vi.mocked(inspectExtensionResource).mockResolvedValue(withMeta);
  peek();
  expect((await screen.findByRole("button",{name:"Suspend"})).getAttribute("data-variant")).toBe("danger");
  expect(screen.getByRole("button",{name:"Resume"}).getAttribute("data-variant")).not.toBe("danger");
});
it("still offers Refresh while the read has failed",async()=>{
  vi.mocked(inspectExtensionResource).mockRejectedValue(new Error("Access denied"));
  peek();
  expect(await screen.findByRole("button",{name:"Refresh details"})).toBeTruthy();
});
// A row in More is unmounted by the pick, so there is nothing to go back to.
it("brings focus back to the view when a review opened from More is cancelled",async()=>{
  peek();
  fireEvent.click(await screen.findByRole("button",{name:"More actions"}));
  const row=await screen.findByRole("button",{name:"Reconcile"});
  row.focus();
  fireEvent.click(row);
  fireEvent.click(await screen.findByRole("button",{name:"Cancel"}));
  await waitFor(()=>expect(row.isConnected).toBe(false));
  expect(document.activeElement?.classList.contains("extension-resource-detail")).toBe(true);
});
it("gives a resource's tab the heading, strip, facts and metadata row of a built-in one",async()=>{
  const resource=stamped();
  vi.mocked(inspectExtensionResource).mockResolvedValue({...detail,status:{status:"healthy",label:"Healthy"},resource:{...resource,metadata:{...resource.metadata,labels:{a:"b"},annotations:{c:"d"}}}});
  render(<ExtensionResourceDetails selection={selection} fullPage clusterLabel="Production"/>);
  expect(await screen.findByText("Healthy")).toBeTruthy();
  expect(screen.getByRole("heading",{level:1,name:"apps"})).toBeTruthy();
  expect(screen.getByRole("button",{name:"Copy name apps"})).toBeTruthy();
  expect(slot("resource-tab")?.textContent).toContain("Production");
  expect(slot("metric-strip")?.textContent).toContain("Status");
  expect(slot("metric-strip")?.textContent).toContain("1h");
  expect(slot("fact-grid")?.textContent).toContain("API");
  expect(slot("tab-actions")?.textContent).toContain("Suspend");
  expect(slot("metadata-pair")?.textContent).toContain("Labels");
  expect(slot("metadata-pair")?.textContent).toContain("Annotations");
  expect(screen.queryByRole("button",{name:"Open tab"})).toBeNull();
});
it("keeps a resource tab's header through loading and failure",async()=>{
  const header=()=>{
    expect(screen.getByRole("heading",{level:1,name:"apps"})).toBeTruthy();
    expect(screen.getByRole("button",{name:"Copy name apps"})).toBeTruthy();
    expect(slot("resource-tab")?.textContent).toContain("Production");
    expect(screen.getAllByRole("tab").map(tab=>tab.textContent)).toEqual(["Overview","Events","YAML"]);
  };
  vi.mocked(inspectExtensionResource).mockReturnValue(new Promise(()=>{}));
  const view=render(<ExtensionResourceDetails selection={selection} fullPage clusterLabel="Production"/>);
  header();
  view.unmount();
  vi.mocked(inspectExtensionResource).mockRejectedValue(new Error("Access denied"));
  render(<ExtensionResourceDetails selection={selection} fullPage clusterLabel="Production"/>);
  expect(await screen.findByText("Access denied")).toBeTruthy();
  header();
});
it("keeps to the kit's tokens and headings in its stylesheet",async()=>{
  const {readFileSync}=await import("node:fs");
  const {join}=await import("node:path");
  const css=readFileSync(join(__dirname,"extensions.css"),"utf8");
  expect(css).not.toContain("var(--danger");
  expect(css).not.toContain(".extension-detail-heading");
});
