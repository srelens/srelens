import {render,screen,fireEvent,waitFor,within} from "@testing-library/react";
import {beforeEach,it,expect,vi} from "vitest";
const clusters=vi.hoisted(()=>({contexts:[] as {name:string;stableId:string;key?:string}[],status:"loaded",error:""}));
vi.mock("@srelens/core",async original=>({...await original<typeof import("@srelens/core")>(),isTauri:()=>true,listExtensions:vi.fn(),listContexts:vi.fn()}));
vi.mock("../lib/clusters",()=>({useContexts:()=>clusters.contexts,useContextsStatus:()=>clusters.status,useContextsError:()=>clusters.error,getContexts:()=>clusters.contexts,getKubeconfigFiles:()=>[],setContexts:vi.fn()}));
vi.mock("../lib/tabsStore",()=>({openTab:vi.fn()}));
vi.mock("../extensions/ExtensionResourceDetails",()=>({ExtensionResourceDetails:({selection,fullPage}:any)=><div data-testid="detail-page">{JSON.stringify({selection,fullPage})}</div>}));
vi.mock("../extensions/ExtensionWorkspace",async()=>{
 const {useContext}=await import("react");const {ExtensionResourceNavigation}=await import("../extensions/resourceNavigation");
 return {ExtensionWorkspace:(props:{card?:string})=>{const open=useContext(ExtensionResourceNavigation);return <button data-card={props.card??""} onClick={()=>open?.({id:"org.srelens.flux",revision:1,capability:"kustomizations",context:"wrong-rail-cluster",namespace:"team",name:"apps"})}>Open resource</button>;}};
});
import {listExtensions,listContexts,extensionRoute,extensionResourceRoute,extensionClusterRoute,extensionClusterResourceRoute,type InstalledExtension} from "@srelens/core";
import {setContexts} from "../lib/clusters";
import {openTab} from "../lib/tabsStore";
import {ExtensionPage} from "./ExtensionPage";
import manifest from "../../../../examples/extensions/flux.json";
const installed:InstalledExtension={manifest:manifest as any,enabled:true,revision:1,grants:manifest.permissions,settings:{},source:"catalog",installedAt:1,history:[]};
beforeEach(()=>{vi.clearAllMocks();Object.assign(clusters,{contexts:[{name:"cluster/a",stableId:"cluster/a",key:"cluster/a"}],status:"loaded",error:""});vi.mocked(listExtensions).mockResolvedValue({schemaVersion:1,nextRevision:2,plugins:[installed]});});
const limitedTo=(id:string)=>vi.mocked(listExtensions).mockResolvedValue({schemaVersion:1,nextRevision:2,plugins:[{...installed,contexts:[id]}]});
const openKustomizations=()=>render(<ExtensionPage ported={[]} onSwitchToClassic={vi.fn()} onLocked={vi.fn()} route={extensionRoute("cluster/a",manifest.id,"kustomizations")}/>);
it("promotes a selected resource using the page's pinned cluster rather than the rail",async()=>{
 render(<ExtensionPage ported={[]} onSwitchToClassic={vi.fn()} onLocked={vi.fn()} route={extensionRoute("cluster/a",manifest.id,"kustomizations")}/>);
 fireEvent.click(await screen.findByText("Open resource"));
 expect(openTab).toHaveBeenCalledWith(extensionClusterResourceRoute("cluster/a",manifest.id,"kustomizations","team","apps"),{clusterName:"cluster/a"});
});
it("renders the independent detail route without mounting the resource list",async()=>{
 render(<ExtensionPage ported={[]} onSwitchToClassic={vi.fn()} onLocked={vi.fn()} route={extensionResourceRoute("cluster/a",manifest.id,"kustomizations","team","apps")}/>);
 const detail=JSON.parse((await screen.findByTestId("detail-page")).textContent!);
 expect(detail).toEqual({selection:{id:manifest.id,revision:1,capability:"kustomizations",context:"cluster/a",namespace:"team",name:"apps"},fullPage:true});
 expect(screen.queryByText("Open resource")).toBeNull();
});
it("waits for the clusters to be listed before saying a limited app is not enabled",async()=>{
 Object.assign(clusters,{contexts:[],status:"loading"});
 limitedTo("/kube/a.yaml#cluster/a");
 openKustomizations();
 // The title changes once the inventory is in, so the loading message is the clusters'.
 expect(await screen.findByText("Kustomizations")).toBeTruthy();
 expect(screen.getByRole("status").textContent).toContain("Loading app");
 expect(screen.queryByText(/not enabled for this cluster/)).toBeNull();
});
it("says the clusters could not be listed rather than that a limited app is not enabled, and retries",async()=>{
 Object.assign(clusters,{contexts:[],status:"failed",error:"kubeconfig unreadable"});
 limitedTo("/kube/a.yaml#cluster/a");
 const listed=[{name:"cluster/a",stableId:"/kube/a.yaml#cluster/a",key:"/kube/a.yaml#cluster/a"}];
 vi.mocked(listContexts).mockResolvedValue({contexts:listed} as any);
 openKustomizations();
 const alert=await screen.findByRole("alert");
 expect(alert.textContent).toContain("Could not list clusters");
 expect(screen.queryByText(/not enabled for this cluster/)).toBeNull();
 fireEvent.click(within(alert).getByRole("button",{name:"Retry"}));
 await waitFor(()=>expect(setContexts).toHaveBeenCalledWith(listed,""));
});
it("keys app scope on the context key, so two contexts sharing a stable ID are told apart",async()=>{
 // The route still carries the stable ID (which this page cannot disambiguate), so a shared
 // one stays unroutable; a unique stable ID with a distinct key is checked by key.
 Object.assign(clusters,{contexts:[{name:"cluster/a",stableId:"/kube/x#y#z",key:"/kube/x#y%23z"},{name:"other",stableId:"/kube/o#other",key:"/kube/o#other"}],status:"loaded"});
 limitedTo("/kube/x#y%23z");
 openKustomizations();
 expect(await screen.findByText("Open resource")).toBeTruthy();
 expect(screen.queryByText(/not enabled for this cluster/)).toBeNull();
});it("says the cluster is gone rather than that a limited app is not enabled",async()=>{
 Object.assign(clusters,{contexts:[{name:"cluster/b",stableId:"/kube/b.yaml#cluster/b",key:"/kube/b.yaml#cluster/b"}],status:"loaded"});
 limitedTo("/kube/a.yaml#cluster/a");
 openKustomizations();
 expect(await screen.findByText(/no longer in your kubeconfig files/)).toBeTruthy();
 expect(screen.queryByText(/not enabled for this cluster/)).toBeNull();
});

it("keeps a stable route on its cluster after display names change", async () => {
 const id="/kube/a.yaml#default";
 limitedTo(id);
 Object.assign(clusters,{contexts:[{name:"first/default",stableId:id,key:id},{name:"second/default",stableId:"/kube/b.yaml#default",key:"/kube/b.yaml#default"}]});
 render(<ExtensionPage ported={[]} onSwitchToClassic={vi.fn()} onLocked={vi.fn()} route={extensionClusterResourceRoute(id,manifest.id,"kustomizations","team","apps")}/>);
 const detail=JSON.parse((await screen.findByTestId("detail-page")).textContent!);
 expect(detail.selection.context).toBe(id);
 expect(screen.queryByText(/no longer in your kubeconfig files/)).toBeNull();
});
it("does not dispatch an unrestricted app when its pinned cluster is missing",async()=>{
 clusters.contexts=[];
 render(<ExtensionPage ported={[]} onSwitchToClassic={vi.fn()} onLocked={vi.fn()} route={extensionClusterRoute("/kube/a.yaml#default",manifest.id,"kustomizations")}/>);
 expect(await screen.findByText(/no longer in your kubeconfig files/)).toBeTruthy();
 expect(screen.queryByText("Open resource")).toBeNull();
});

it("never substitutes a literal name for a missing stable route identity",async()=>{
 const id="/kube/a.yaml#default";
 limitedTo(id);
 clusters.contexts=[{name:id,stableId:"/kube/impostor.yaml#literal",key:"/kube/impostor.yaml#literal"}];
 render(<ExtensionPage ported={[]} onSwitchToClassic={vi.fn()} onLocked={vi.fn()} route={extensionClusterRoute(id,manifest.id,"kustomizations")}/>);
 expect(await screen.findByText(/no longer in your kubeconfig files/)).toBeTruthy();
 expect(screen.queryByText("Open resource")).toBeNull();
});

it("retains the identity of an already-open legacy route when its name changes",async()=>{
 const id="/kube/a.yaml#default";
 limitedTo(id);
 clusters.contexts=[{name:"cluster/a",stableId:id,key:id}];
 const props={ported:[],onSwitchToClassic:vi.fn(),onLocked:vi.fn(),route:extensionRoute("cluster/a",manifest.id,"kustomizations")};
 const mounted=render(<ExtensionPage {...props}/>);
 expect(await screen.findByText("Open resource")).toBeTruthy();
 clusters.contexts=[{name:"first/cluster/a",stableId:id,key:id},{name:"second/cluster/a",stableId:"/kube/b#cluster/a",key:"/kube/b#cluster/a"}];
 mounted.rerender(<ExtensionPage {...props}/>);
 expect(screen.getByText("Open resource")).toBeTruthy();
 fireEvent.click(screen.getByText("Open resource"));
 expect(openTab).toHaveBeenCalledWith(extensionClusterResourceRoute(id,manifest.id,"kustomizations","team","apps"),{clusterName:"first/cluster/a"});
});

it("opens a dashboard card's target filtered to what the card counted, and offers the whole list", async () => {
 const {extensionCardRoute}=await import("@srelens/core");
 render(<ExtensionPage ported={[]} onSwitchToClassic={vi.fn()} onLocked={vi.fn()} route={extensionCardRoute("cluster/a",manifest.id,"kustomizations","team","suspended-kustomizations")}/>);
 expect((await screen.findByText("Open resource")).getAttribute("data-card")).toBe("suspended-kustomizations");
 const notice=screen.getByRole("status");
 expect(notice.textContent).toContain("Suspended Kustomizations");
 fireEvent.click(within(notice).getByRole("button",{name:"Show all Kustomizations"}));
 expect(openTab).toHaveBeenCalledWith(extensionClusterRoute("cluster/a",manifest.id,"kustomizations","team"),{clusterName:"cluster/a"});
});
it("says a card the app no longer declares is gone rather than showing every row as its answer", async () => {
 const {extensionCardRoute}=await import("@srelens/core");
 render(<ExtensionPage ported={[]} onSwitchToClassic={vi.fn()} onLocked={vi.fn()} route={extensionCardRoute("cluster/a",manifest.id,"kustomizations","","removed-card")}/>);
 expect(await screen.findByText(/no longer declares this dashboard card/)).toBeTruthy();
 expect(screen.queryByText("Open resource")).toBeNull();
 expect(screen.getByRole("button",{name:"Show all Kustomizations"})).toBeTruthy();
});
