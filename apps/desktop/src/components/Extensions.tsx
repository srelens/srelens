import {
  ExtensionManager as Manager,
  ExtensionResourceSlot as ResourceSlot,
  ExtensionControlsProvider,
} from "@srelens/ui-next/extensions";
import { Button, Combobox, Tabs } from "../ui";
import type { ComponentProps } from "react";
import { LayoutGrid } from "lucide-react";

// Keep each design's controls and stylesheet; only the extension behavior is shared.
const controls = { Button, Combobox, Tabs };
export function ExtensionManager() {
  return (
    <ExtensionControlsProvider value={controls}>
      <Manager />
    </ExtensionControlsProvider>
  );
}
export function ExtensionResourceSlot(
  props: ComponentProps<typeof ResourceSlot>,
) {
  return (
    <ExtensionControlsProvider value={controls}>
      <ResourceSlot {...props} />
    </ExtensionControlsProvider>
  );
}

import { ErrorNotice, ExtensionWorkspace, ExtensionLogo, useExtensions, useContextLookup, refreshContextIds, SHARED_CONTEXT_ID_MESSAGE, ExtensionResourceNavigation, ExtensionResourceDetails } from "@srelens/ui-next/extensions";
import { extensionEnabledFor } from "@srelens/core";
export function ClassicAppsNav({context,onOpen}:{context:string;onOpen(context:string,id:string,page:string):void}) {
  const inventory=useExtensions();
  // App scope keys on the context's stable ID, not its name (#265).
  const lookup=useContextLookup(context);
  const contextId=lookup.status==="found"?lookup.id:undefined;
  const withPages=inventory.data?.plugins.filter(p=>p.enabled && p.manifest.contributions.pages.length)??[];
  const apps=withPages.filter(p=>extensionEnabledFor(p,contextId));
  // A failed lookup hides limited apps; say why instead of dropping them silently.
  const failure=lookup.status==="failed"&&withPages.some(p=>p.contexts)?lookup.error:undefined;
  if(!apps.length&&failure===undefined)return null;
  return <ExtensionControlsProvider value={controls}>{failure!==undefined&&<div className="pl-3 py-1 text-sm"><ErrorNotice title="Could not list clusters" message={failure} retry={()=>void refreshContextIds()}/></div>}{apps.length>0&&<details className="pl-3 py-1 text-sm"><summary className="cursor-pointer text-muted-foreground"><LayoutGrid size={16} className="inline-block align-middle mr-1" aria-hidden="true"/>Apps</summary>{apps.map(app=><details key={app.manifest.id} className="pl-2 py-1"><summary className="cursor-pointer"><ExtensionLogo id={app.manifest.id} name={app.manifest.name} size={16}/> {app.manifest.name}</summary>{app.manifest.contributions.pages.map(page=><button type="button" key={page.id} aria-label={`Open ${page.title}`} className="block w-full truncate px-3 py-1 text-left hover:bg-muted" onClick={()=>onOpen(context,app.manifest.id,page.id)}>{page.group?`${page.group} · ${page.title}`:page.title}</button>)}</details>)}</details>}</ExtensionControlsProvider>;
}
export function ClassicAppPage({context,id,page,namespace="",resourceName,onOpenResource,onPage,onNamespace}:{context:string;id:string;page:string;namespace?:string;resourceName?:string;onOpenResource?(name:string,namespace:string):void;onPage(page:string,namespace?:string):void;onNamespace?(namespace:string):void}) {
  const inventory=useExtensions();
  const lookup=useContextLookup(context);
  const contextId=lookup.status==="found"?lookup.id:undefined;
  const plugin=inventory.data?.plugins.find(p=>p.enabled && p.manifest.id===id);
  const contribution=plugin?.manifest.contributions.pages.find(p=>p.id===page);
  // Only an app limited to some clusters waits on the lookup, and a failed lookup is not a denial.
  const limited=Boolean(plugin?.contexts);
  return <ExtensionControlsProvider value={controls}><div className="flex min-h-0 flex-1 flex-col overflow-auto">
    {inventory.status==="loading"||(limited&&lookup.status==="loading")?<p className="p-3">Loading app…</p>:inventory.status==="error"?<div role="alert" className="p-3">{inventory.error}<Button onClick={inventory.reload}>Retry</Button></div>:limited&&lookup.status==="failed"?<div className="p-3"><ErrorNotice title="Could not list clusters" message={lookup.error} retry={()=>void refreshContextIds()}/></div>:limited&&lookup.status==="shared"?<p className="p-3">{SHARED_CONTEXT_ID_MESSAGE}</p>:limited&&lookup.status==="missing"?<p className="p-3">This cluster is no longer in your kubeconfig files, so its apps cannot be opened here. Manage your kubeconfig files in Settings → Contexts.</p>:plugin&&contribution&&!extensionEnabledFor(plugin,contextId)?<p className="p-3">This app is not enabled for this cluster. Manage it in Settings → Apps.</p>:plugin&&contribution?<ExtensionResourceNavigation.Provider value={onOpenResource ? resource=>onOpenResource(resource.name,resource.namespace):undefined}>{resourceName ? <ExtensionResourceDetails fullPage selection={{id,revision:plugin.revision,capability:contribution.capability,context,namespace,name:resourceName}}/> : <ExtensionWorkspace key={`${context}/${id}/${plugin.revision}`} context={context} plugin={plugin} page={contribution} namespace={namespace} onPage={onPage} onNamespace={onNamespace}/>}</ExtensionResourceNavigation.Provider>:<p className="p-3">This app page is unavailable. Manage it in Settings → Apps.</p>}
  </div></ExtensionControlsProvider>;
}
