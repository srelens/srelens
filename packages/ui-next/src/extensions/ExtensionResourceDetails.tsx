import { ExtensionResourceNavigation } from "./resourceNavigation";
import { useContext, useEffect, useRef, useState } from "react";
import { inspectExtensionResource, actOnExtensionResource, formatResourceManifest, type ExtensionResourceSelection } from "@srelens/core";
import { Inspector, Button, CodeEditor, KV } from "@srelens/ui-kit";
import { Icons } from "../lib/icons";
import { ErrorNotice } from "./ExtensionResults";
import { useResource } from "../lib/useResource";
const actions: Record<string,{label:string;description:string}> = {
  suspend:{label:"Suspend",description:"Pause future reconciliation. Workloads already running are not stopped."},
  resume:{label:"Resume",description:"Allow the controller to reconcile this resource again."},
  reconcile:{label:"Reconcile",description:"Ask Flux to reconcile now using the configured source."},
  force:{label:"Force reconcile",description:"Force a Helm install or upgrade, even if the chart and values have not changed."},
  reset:{label:"Reset retries",description:"Reset Helm remediation retries and request reconciliation."},
  refresh:{label:"Refresh status",description:"Ask Argo CD to refresh this application's status."},
  "hard-refresh":{label:"Hard refresh",description:"Invalidate Argo CD's manifest cache and refresh this application."},
  sync:{label:"Sync",description:"Apply the application's desired resources with Argo CD. This request does not enable pruning. Application sync options and hooks still apply."},
};
const fieldLabels: Record<string,string> = {sourceRef:"Source reference",suspend:"Suspended",prune:"Prune",wait:"Wait for readiness",force:"Force",apiVersion:"API version"};
function fieldLabel(key:string) {
  const words=key.replace(/([a-z0-9])([A-Z])/g,"$1 $2").replace(/_/g," ");
  return Object.hasOwn(fieldLabels,key) ? fieldLabels[key] : words.charAt(0).toUpperCase()+words.slice(1);
}
function Fields({value,depth=0}:{value:Record<string,unknown>;depth?:number}) {
  return <div className="extension-detail-fields">{Object.entries(value).map(([key,value])=>{
    const label=fieldLabel(key);
    if(value !== null && typeof value === "object") {
      if(depth>=2 || Array.isArray(value)) return <KV key={key} k={label} v={<details><summary>{Array.isArray(value)?`${value.length} entries`:"Show data"}</summary><pre>{JSON.stringify(value,null,2)}</pre></details>}/>;
      return <div key={key} className="extension-nested-fields"><h5>{label}</h5><Fields value={value as Record<string,unknown>} depth={depth+1}/></div>;
    }
    return <KV key={key} k={label} v={<span className="extension-field-value">{typeof value === "boolean" ? value ? "Yes" : "No" : String(value ?? "—")}</span>}/>;
  })}</div>;
}
export function ExtensionResourceDetails({selection,onClose,onChanged,fullPage=false}:{selection:ExtensionResourceSelection;onClose?():void;onChanged():void;fullPage?:boolean}) {
  const openResource=useContext(ExtensionResourceNavigation);

  const data=useResource(()=>inspectExtensionResource(selection),[selection.id,selection.revision,selection.capability,selection.context,selection.namespace,selection.name]);
  const [tab,setTab]=useState("overview");
  const [pending,setPending]=useState<string|null>(null);
  const [busy,setBusy]=useState(false);
  const [error,setError]=useState("");
  const [message,setMessage]=useState("");
  const alive=useRef(true);
  const heading=useRef<HTMLDivElement>(null);
  const review=useRef<HTMLDivElement>(null);
  const trigger=useRef<HTMLElement|null>(null);
  useEffect(()=>{alive.current=true;heading.current?.focus();return()=>{alive.current=false;};},[]);
  useEffect(()=>{if(pending)review.current?.focus();},[pending]);
  const cancel=()=>{if(!busy){setPending(null);trigger.current?.focus();}};
  const resource=data.data?.resource;
  const confirm=async()=>{
    if(!pending || !resource || busy)return;
    setBusy(true);setError("");setMessage("");
    try {
      const result=await actOnExtensionResource(selection,pending,resource.metadata.uid,resource.metadata.resourceVersion);
      if(!result.requested)throw new Error("The action was not acknowledged; refresh to check the resource.");
      if(alive.current){setPending(null);setMessage("Request accepted. The controller will report progress in resource status.");data.reload();onChanged();}
    }catch(e){if(alive.current){setError(e instanceof Error?e.message:String(e));setPending(null);}}
    finally{if(alive.current)setBusy(false);}
  };
  const suspended=resource?.spec?.suspend === true;
  const supported=(data.data?.actions??[]).filter(a=>Object.hasOwn(actions,a) && (a!=="suspend"||!suspended) && (a!=="resume"||suspended));
  const OpenIcon=Icons.openTab;
  return <div className="extension-resource-detail" ref={heading} tabIndex={-1} onKeyDownCapture={e=>{if(e.key==="Escape" && (pending || busy)){e.preventDefault();e.stopPropagation();if(pending)cancel();}}}>
    <Inspector
      name={selection.name}
      subtitle={`${resource?.kind ?? "Resource"} · ${selection.namespace || "Cluster-scoped"}`}
      onClose={onClose ? ()=>{if(!busy)onClose();} : undefined}
      actions={!fullPage && openResource ? <Button variant="outline" size="xs" disabled={busy} onClick={()=>openResource(selection)}><OpenIcon size={12} aria-hidden="true"/>Open tab</Button> : undefined}
      tabs={[{id:"overview",label:"Overview"},{id:"manifest",label:"Manifest"}]}
      activeTab={tab} onTabChange={setTab} tabsLabel="Resource views"
      footer={<div className="flex flex-wrap items-center gap-1.5">
        <Button variant="outline" size="xs" disabled={busy||!!pending} onClick={()=>{setError("");setMessage("");data.reload();}}>Refresh details</Button>
        {data.status==="ready" && supported.map(action=><Button key={action} variant="outline" size="xs" disabled={busy || !!pending || !resource?.metadata.uid || !resource?.metadata.resourceVersion || (suspended&&["reconcile","force","reset"].includes(action))} onClick={()=>{trigger.current=document.activeElement as HTMLElement;setError("");setMessage("");setPending(action);}}>{actions[action].label}</Button>)}
      </div>}
    >
    {error&&<p role="alert" className="extension-error">{error}</p>}
    {message&&<p role="status" className="extension-message">{message}</p>}
    {data.status==="error"?<ErrorNotice cluster message={data.error} retry={data.reload}/>:data.status==="loading"?<p className="extension-message">Loading resource details…</p>:resource&&<>
      {pending&&<div className="extension-action-review" role="dialog" aria-label={`Review ${actions[pending].label}`} tabIndex={-1} ref={review}>
        <strong>{actions[pending].label}: {selection.namespace}/{selection.name}</strong><p>Cluster: {selection.context}</p><p>{actions[pending].description}</p>
        <div className="extension-toolbar"><Button disabled={busy} onClick={()=>void confirm()}>{busy?"Requesting…":`Confirm ${actions[pending].label}`}</Button><Button variant="outline" disabled={busy} onClick={cancel}>Cancel</Button></div>
      </div>}
      {tab==="manifest"?<div className="flex h-full min-h-0 flex-col"><div className="min-h-0 flex-1"><CodeEditor value={formatResourceManifest(resource)} readOnly language="yaml" fill ariaLabel={`${selection.name} manifest`}/></div></div>:<>
        <h4 className="extension-detail-heading">Overview</h4><Fields value={{Name:resource.metadata.name,Namespace:resource.metadata.namespace??"—",Kind:resource.kind,API:resource.apiVersion,Created:resource.metadata.creationTimestamp??"—",...(resource.spec??{})}}/>
        <h4 className="extension-detail-heading">Conditions</h4>
        {Array.isArray(resource.status?.conditions)&&resource.status.conditions.length?<div className="extension-condition-list">{resource.status.conditions.map((c:any,i:number)=><article key={i}><strong>{c.type} · {c.status}</strong><div>{c.reason||"—"}</div><p>{c.message||"—"}</p></article>)}</div>:<p className="extension-message">No conditions reported.</p>}
        <h4 className="extension-detail-heading">Status</h4><Fields value={Object.fromEntries(Object.entries(resource.status??{}).filter(([key])=>key!=="conditions"))}/>
        <h4 className="extension-detail-heading">Events</h4>
        {data.data?.eventsError ? <ErrorNotice cluster message={data.data.eventsError} retry={data.reload}/> : data.data?.events?.length ? <div className="extension-condition-list">{data.data.events.map((event,i)=><article key={i}><strong>{event.type} · {event.reason}</strong><p>{event.message}</p><span>Count: {event.count??1}</span></article>)}</div> : <p className="extension-message">No events reported.</p>}
        <details className="extension-detail-metadata"><summary>Labels and annotations</summary><Fields value={{labels:resource.metadata.labels??{},annotations:resource.metadata.annotations??{}}}/></details>
      </>}
    </>}
    </Inspector>
  </div>;
}
