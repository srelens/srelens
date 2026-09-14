import { useContext, useEffect, useRef, useState } from "react";
import { inspectExtensionResource, actOnExtensionResource, type ExtensionResourceSelection } from "@srelens/core";
import { ExtensionControls } from "./ExtensionControls";
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
function Fields({value}:{value:Record<string,unknown>}) {
  return <dl className="extension-detail-fields">{Object.entries(value).map(([key,value])=><div key={key}><dt>{key}</dt><dd>{value !== null && typeof value === "object" ? <details><summary>{Array.isArray(value)?`${value.length} entries`:"View fields"}</summary><pre>{JSON.stringify(value,null,2)}</pre></details> : String(value ?? "—")}</dd></div>)}</dl>;
}
export function ExtensionResourceDetails({selection,onClose,onChanged}:{selection:ExtensionResourceSelection;onClose():void;onChanged():void}) {
  const {Button,Tabs}=useContext(ExtensionControls);
  const data=useResource(()=>inspectExtensionResource(selection),[selection.id,selection.revision,selection.capability,selection.context,selection.namespace,selection.name]);
  const [tab,setTab]=useState("overview");
  const [pending,setPending]=useState<string|null>(null);
  const [busy,setBusy]=useState(false);
  const [error,setError]=useState("");
  const [message,setMessage]=useState("");
  const alive=useRef(true);
  const heading=useRef<HTMLHeadingElement>(null);
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
  return <section className="extension-resource-detail" onKeyDown={e=>{if(e.key==="Escape"){e.stopPropagation();if(pending)cancel();else if(!busy)onClose();}}}>
    <div className="extension-toolbar"><Button variant="secondary" disabled={busy} onClick={onClose}>Back to resources</Button><h3 ref={heading} tabIndex={-1}>{selection.name}</h3><span>{selection.namespace || "Cluster-scoped"}</span><Button variant="secondary" disabled={busy||!!pending} onClick={()=>{setError("");setMessage("");data.reload();}}>Refresh details</Button></div>
    {error&&<p role="alert" className="extension-error">{error}</p>}
    {message&&<p role="status" className="extension-message">{message}</p>}
    {data.status==="error"?<ErrorNotice cluster message={data.error} retry={data.reload}/>:data.status==="loading"?<p className="extension-message">Loading resource details…</p>:resource&&<>
      <div className="extension-toolbar"><span>{resource.kind} · {resource.apiVersion}</span>{supported.map(action=><Button key={action} variant="secondary" disabled={busy || !!pending || !resource.metadata.uid || !resource.metadata.resourceVersion || (suspended&&["reconcile","force","reset"].includes(action))} onClick={()=>{trigger.current=document.activeElement as HTMLElement;setError("");setMessage("");setPending(action);}}>{actions[action].label}</Button>)}</div>
      {pending&&<div className="extension-action-review" role="dialog" aria-label={`Review ${actions[pending].label}`} tabIndex={-1} ref={review}>
        <strong>{actions[pending].label}: {selection.namespace}/{selection.name}</strong><p>Cluster: {selection.context}</p><p>{actions[pending].description}</p>
        <div className="extension-toolbar"><Button disabled={busy} onClick={()=>void confirm()}>{busy?"Requesting…":`Confirm ${actions[pending].label}`}</Button><Button variant="secondary" disabled={busy} onClick={cancel}>Cancel</Button></div>
      </div>}
      <Tabs variant="underline" label="Resource details" tabs={[{id:"overview",label:"Overview"},{id:"manifest",label:"Manifest"}]} active={tab} onChange={setTab}/>
      {tab==="manifest"?<pre className="extension-detail-manifest">{JSON.stringify(resource,null,2)}</pre>:<>
        <h4 className="extension-detail-heading">Overview</h4><Fields value={{Name:resource.metadata.name,Namespace:resource.metadata.namespace??"—",Kind:resource.kind,API:resource.apiVersion,Created:resource.metadata.creationTimestamp??"—",...(resource.spec??{})}}/>
        <h4 className="extension-detail-heading">Conditions</h4>
        {Array.isArray(resource.status?.conditions)&&resource.status.conditions.length?<div className="extension-table-scroll"><table><thead><tr><th>Type</th><th>Status</th><th>Reason</th><th>Message</th></tr></thead><tbody>{resource.status.conditions.map((c:any,i:number)=><tr key={i}><td>{c.type}</td><td>{c.status}</td><td>{c.reason||"—"}</td><td className="extension-condition-message">{c.message||"—"}</td></tr>)}</tbody></table></div>:<p className="extension-message">No conditions reported.</p>}
        <h4 className="extension-detail-heading">Status</h4><Fields value={Object.fromEntries(Object.entries(resource.status??{}).filter(([key])=>key!=="conditions"))}/>
        <h4 className="extension-detail-heading">Events</h4>
        {data.data?.eventsError ? <ErrorNotice cluster message={data.data.eventsError} retry={data.reload}/> : data.data?.events?.length ? <div className="extension-table-scroll"><table><thead><tr><th>Type</th><th>Reason</th><th>Message</th><th>Count</th></tr></thead><tbody>{data.data.events.map((event,i)=><tr key={i}><td>{event.type}</td><td>{event.reason}</td><td className="extension-condition-message">{event.message}</td><td>{event.count??1}</td></tr>)}</tbody></table></div> : <p className="extension-message">No events reported.</p>}
        <details className="extension-detail-metadata"><summary>Labels and annotations</summary><Fields value={{labels:resource.metadata.labels??{},annotations:resource.metadata.annotations??{}}}/></details>
      </>}
    </>}
  </section>;
}
