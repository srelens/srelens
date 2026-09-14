import {render,screen,fireEvent} from "@testing-library/react";
import {beforeEach,it,expect,vi} from "vitest";
vi.mock("@srelens/core",async original=>({...await original<typeof import("@srelens/core")>(),isTauri:()=>true,listExtensions:vi.fn()}));
vi.mock("../lib/clusters",()=>({useContexts:()=>[{name:"cluster/a",stableId:"cluster/a"}]}));
vi.mock("../lib/tabsStore",()=>({openTab:vi.fn()}));
vi.mock("../extensions/ExtensionResourceDetails",()=>({ExtensionResourceDetails:({selection,fullPage}:any)=><div data-testid="detail-page">{JSON.stringify({selection,fullPage})}</div>}));
vi.mock("../extensions/ExtensionWorkspace",async()=>{
 const {useContext}=await import("react");const {ExtensionResourceNavigation}=await import("../extensions/resourceNavigation");
 return {ExtensionWorkspace:()=>{const open=useContext(ExtensionResourceNavigation);return <button onClick={()=>open?.({id:"org.srelens.flux",revision:1,capability:"kustomizations",context:"wrong-rail-cluster",namespace:"team",name:"apps"})}>Open resource</button>;}};
});
import {listExtensions,extensionRoute,extensionResourceRoute} from "@srelens/core";
import {openTab} from "../lib/tabsStore";
import {ExtensionPage} from "./ExtensionPage";
import manifest from "../../../../examples/extensions/flux.json";
beforeEach(()=>{vi.clearAllMocks();vi.mocked(listExtensions).mockResolvedValue({schemaVersion:1,nextRevision:2,plugins:[{manifest:manifest as any,enabled:true,revision:1,grants:manifest.permissions,settings:{}}]});});
it("promotes a selected resource using the page's pinned cluster rather than the rail",async()=>{
 render(<ExtensionPage ported={[]} onSwitchToClassic={vi.fn()} onLocked={vi.fn()} route={extensionRoute("cluster/a",manifest.id,"kustomizations")}/>);
 fireEvent.click(await screen.findByText("Open resource"));
 expect(openTab).toHaveBeenCalledWith(extensionResourceRoute("cluster/a",manifest.id,"kustomizations","team","apps"),{clusterName:"cluster/a"});
});
it("renders the independent detail route without mounting the resource list",async()=>{
 render(<ExtensionPage ported={[]} onSwitchToClassic={vi.fn()} onLocked={vi.fn()} route={extensionResourceRoute("cluster/a",manifest.id,"kustomizations","team","apps")}/>);
 const detail=JSON.parse((await screen.findByTestId("detail-page")).textContent!);
 expect(detail).toEqual({selection:{id:manifest.id,revision:1,capability:"kustomizations",context:"cluster/a",namespace:"team",name:"apps"},fullPage:true});
 expect(screen.queryByText("Open resource")).toBeNull();
});
