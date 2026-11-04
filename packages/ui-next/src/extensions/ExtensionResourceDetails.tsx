import { ExtensionPanelSlot } from "./ExtensionPanelSlot";
import { ExtensionRelatedSlot } from "./ExtensionRelatedSlot";
import { ExtensionPodSlot } from "./ExtensionPodTools";
import { ExtensionResourceNavigation } from "./resourceNavigation";
import { useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { ageFromTimestamp, asArray, inspectExtensionResource, actOnExtensionResource, formatResourceManifest, onExtensionResourceChanged, renderConfirmTemplate, unmetPredicate, type Condition, type EventSummary, type ExtensionResourceDetail, type ExtensionResourceSelection, type K8sObject, type ResolvedStatus } from "@srelens/core";
import { Alert, Inspector, Button, CodeEditor, KV, LoadingState, Section as Block, StatusPill, SubHead, type ActionBarAction } from "@srelens/ui-kit";
import { plainText } from "./displayText";
import { statusKind, statusWord } from "./AppStatus";
import { AgeCell } from "../lib/ageCell";
import { FailureAlert, FailureState } from "../lib/errorCopy";
import { CopyNameButton } from "../screens/detail/CopyNameButton";
import { DetailActionRow } from "../screens/detail/DetailActions";
import { EventsTable, describeTarget } from "../screens/detail/detailData";
import type { DetailFact } from "../screens/detail/facts";
import { identityFacts } from "../screens/detail/GenericBody";
import { OpenTabButton, PeekFacts, ResourceTabFrame, TabFacts, TabMetadata, type Tile } from "../screens/detail/ResourceFrame";
import { AnnotationsSection, ConditionsSection, LabelsSection } from "../screens/detail/sections";
import { Section, SectionMemory } from "../screens/detail/Section";
import { useResource } from "../lib/useResource";
import { HostConfirmation } from "../confirm/HostConfirmation";
import { useProtectedConfirm } from "../lib/protectedConfirm";
import { useConfirmationApp } from "../confirm/confirmationApp";
import { confirmFields } from "../confirm/confirmRequest";
import { onExtensionActionRequested, takeExtensionAction } from "./actionRequests";
const TABS=[{id:"overview",label:"Overview"},{id:"events",label:"Events"},{id:"yaml",label:"YAML"}];
/** A field's key in sentence case: `sourceRef` reads "Source ref", `repoURL` "Repo URL". A key that is no identifier is someone's data, and reads as they wrote it. */
function fieldLabel(key:string) {
  if(!/^[A-Za-z][A-Za-z0-9_]*$/.test(key))return key;
  const words=key.replace(/([a-z0-9])([A-Z])/g,"$1 $2").replace(/_/g," ").replace(/ ([A-Z])(?=[a-z])/g,(_,capital:string)=>` ${capital.toLowerCase()}`);
  return words.charAt(0).toUpperCase()+words.slice(1);
}
/** Long or deep data, shut until asked for. Held here rather than remembered: a path in one resource says nothing about the next. */
function Fold({title,className,children}:{title:string;className?:string;children:ReactNode}) {
  const [open,setOpen]=useState(false);
  return <Block title={title} className={className} open={open} onToggle={setOpen} padded={false}>{children}</Block>;
}
function Entries({label,items}:{label:string;items:unknown[]}) {
  const rows=items.map(item=>item !== null && typeof item === "object" && !Array.isArray(item) ? item as Record<string,unknown> : {value:item});
  const columns=[...new Set(rows.flatMap(row=>Object.keys(row)))];
  const cell=(value:unknown)=>value == null ? "—" : typeof value === "object" ? JSON.stringify(value) : String(value);
  return <Fold title={`${label} · ${items.length} ${items.length===1?"entry":"entries"}`} className="extension-entries">
    {items.length ? <div className="extension-entries-scroll" tabIndex={0} role="region" aria-label={`${label} list`}>
      <table aria-label={label}><thead><tr>{columns.map(column=><th scope="col" key={column}>{column==="id"?"ID":column==="v"?"Version":fieldLabel(column)}</th>)}</tr></thead>
      <tbody>{rows.map((row,index)=><tr key={index}>{columns.map(column=><td key={column}>{cell(row[column])}</td>)}</tr>)}</tbody></table>
    </div> : <p className="extension-message">No entries.</p>}
  </Fold>;
}
function Fields({value,depth=0}:{value:Record<string,unknown>;depth?:number}) {
  return <>{Object.entries(value).map(([key,value])=>{
    const label=fieldLabel(key);
    if(value !== null && typeof value === "object") {
      if(Array.isArray(value)) return <Entries key={key} label={label} items={value}/>;
      if(depth>=2) return <Fold key={key} title={label}><pre>{JSON.stringify(value,null,2)}</pre></Fold>;
      return <div key={key} className="extension-nested-fields"><SubHead>{label}</SubHead><Fields value={value as Record<string,unknown>} depth={depth+1}/></div>;
    }
    // A nested key is often a map's own (`values`, `matchLabels`), so it stays as written.
    return <KV key={key} k={depth?key:label} v={<span className="extension-field-value">{typeof value === "boolean" ? value ? "Yes" : "No" : String(value ?? "—")}</span>}/>;
  })}</>;
}
/** Says what the host read: an empty list is only "no events" when every page was read. */
function Events({detail,kind,selection}:{detail?:ExtensionResourceDetail;kind:string;selection:ExtensionResourceSelection}) {
  const events=detail?.events??[];
  const count=(n:number)=>n.toLocaleString("en-US");
  const unread="This resource has more events that were not read.";
  const notice=detail?.eventsPartial
    ? events.length
      ? `Showing the newest ${count(events.length)}${detail.eventsRead===undefined?"":` of ${count(detail.eventsRead)}`} events read. ${unread}`
      : `No events were returned before the host stopped reading. ${unread}`
    : detail?.eventsTruncated ? "Showing the latest 100 events." : "";
  const alert=notice?<Alert tone="info" title={notice}/>:undefined;
  if(detail?.eventsPartial&&!events.length)return alert;
  const rows=events.map((event,index):EventSummary=>({name:String(index),namespace:selection.namespace,type:event.type??"",reason:event.reason??"",object:`${kind}/${selection.name}`,message:event.message??"",created:event.time,age:event.time?ageFromTimestamp(event.time):"—",count:event.count??1}));
  return <EventsTable events={rows} notice={alert}/>;
}
/**
 * The Overview pane, in the order a built-in detail reads: what the header has
 * not said, the spec, the status and metadata blocks, then what apps add. A flat
 * run of siblings, because `.section + .section` draws the rules between them.
 *
 * The blocks that fold open shut on a first visit and are remembered per kind,
 * as on a built-in detail. The spec is untitled so it cannot: it is why an app
 * page exists, and Overview must not open showing nothing.
 */
function Overview({selection,resource,verdict,fullPage}:{selection:ExtensionResourceSelection;resource:ExtensionResourceDetail["resource"];verdict?:ResolvedStatus;fullPage:boolean}) {
  const created=resource.metadata.creationTimestamp;
  const kind=resource.kind??"";
  const facts:DetailFact[]=[
    // The strip always carries the age; the peek's header only beside a verdict.
    ...(created&&!fullPage&&!verdict?[{label:"Created",value:<><AgeCell created={created}/> ago</>}]:[]),
    // One wording for an owner. Namespace and Created are the header's here.
    ...identityFacts({kind,object:resource as K8sObject}).filter(fact=>fact.label==="Controlled by"),
    ...(verdict?.reason?[{label:"Reason",value:plainText(verdict.reason)}]:[]),
    ...(resource.apiVersion?[{label:"API",value:resource.apiVersion,mono:true}]:[]),
  ];
  const spec=resource.spec??{};
  // Conditions have their block; a key of that name holding anything else is the app's own field.
  const status=Object.fromEntries(Object.entries(resource.status??{}).filter(([key,value])=>key!=="conditions"||!Array.isArray(value)));
  const labels=<LabelsSection labels={resource.metadata.labels??{}}/>;
  const annotations=<AnnotationsSection kind={kind} annotations={resource.metadata.annotations??{}}/>;
  return <>
    {fullPage?<TabFacts facts={facts}/>:<PeekFacts facts={facts}/>}
    {Object.keys(spec).length>0&&<Section><Fields value={spec}/></Section>}
    <SectionMemory kind={resource.kind??selection.capability}>
      <ConditionsSection conditions={asArray(resource.status?.conditions) as unknown as Condition[]}/>
      {Object.keys(status).length>0&&<Section title="Status"><Fields value={status}/></Section>}
      {fullPage?<TabMetadata labels={labels} annotations={annotations}/>:<>{labels}{annotations}</>}
    </SectionMemory>
    <ExtensionPanelSlot context={selection.context} resource={resource}/>
    <ExtensionRelatedSlot context={selection.context} resource={resource}/>
    <ExtensionPodSlot context={selection.context} resource={resource}/>
  </>;
}
/**
 * The host's sentence for one action on one resource, or `null`.
 *
 * Rendered HERE from the template the host sent for THIS action
 * (`actionMeta`, #548) against the fields the host's own vocabulary allows —
 * the same template, the same vocabulary and the same escaping the backend
 * uses when an agent asks for the identical write. `null` when the host
 * authored no template, or when the template names something this call has no
 * value for: the confirmation then shows the facts and the buttons without a
 * sentence, rather than a sentence with a hole in it.
 */
function hostQuestion(detail:ExtensionResourceDetail|undefined,action:string,selection:ExtensionResourceSelection,kind:string|undefined):string|null {
  const template=detail?.actionMeta?.[action]?.confirm;
  if(!template)return null;
  return renderConfirmTemplate(template,confirmFields({
    action,
    cluster:selection.context,
    kind,
    namespace:selection.namespace,
    name:selection.name,
  }));
}
export function ExtensionResourceDetails({selection,onClose,fullPage=false,clusterLabel}:{selection:ExtensionResourceSelection;onClose?():void;fullPage?:boolean;
  /** Where a full page's trail starts: the cluster, in the words the reader knows it by. */
  clusterLabel?:string}) {
  const openResource=useContext(ExtensionResourceNavigation);
  // Who the host says asked: read from its own installed inventory, never
  // from the app. See `confirmationApp.ts`.
  const app=useConfirmationApp({id:selection.id,revision:selection.revision});

  const data=useResource(()=>inspectExtensionResource(selection),[selection.id,selection.revision,selection.capability,selection.context,selection.namespace,selection.name]);
  const {refresh}=data;
  // An accepted action from this view or any other view of the same resource refreshes it,
  // in place: the reply replaces what is on screen rather than blanking it first.
  useEffect(()=>onExtensionResourceChanged(changed=>{
    // The binding is part of the identity: one app can expose a GitRepository and a
    // Kustomization with the same namespace and name.
    if(changed.id===selection.id && changed.capability===selection.capability && changed.context===selection.context && changed.namespace===selection.namespace && changed.name===selection.name) refresh();
  }),[selection.id,selection.capability,selection.context,selection.namespace,selection.name,refresh]);
  const [tab,setTab]=useState("overview");
  // The review pins the UID and resourceVersion the reader saw. A refresh while it is open
  // must not swap in a newer version, or the host's stale-review guard would pass it.
  const [pending,setPending]=useState<{action:string;uid:string;resourceVersion:string}|null>(null);
  const [busy,setBusy]=useState(false);
  const [error,setError]=useState("");
  const [message,setMessage]=useState("");
  // On a protected cluster Confirm waits for the cluster's name, typed (#101).
  const typed=useProtectedConfirm(selection.context);
  // Each review is its own question: a name typed for one answers nothing else.
  // Cleared DURING render when the review changes, not in an effect, so no
  // render of the next review is ever committed with the last one's name typed
  // and Confirm live (see AgentConsent). `pending` is a new object per review.
  const [typedFor,setTypedFor]=useState(pending);
  if(typedFor!==pending){setTypedFor(pending);typed.reset();}
  const alive=useRef(true);
  const heading=useRef<HTMLDivElement>(null);
  const review=useRef<HTMLDivElement>(null);
  const trigger=useRef<HTMLElement|null>(null);
  useEffect(()=>{alive.current=true;heading.current?.focus();return()=>{alive.current=false;};},[]);
  useEffect(()=>{if(pending)review.current?.focus();},[pending]);
  const cancel=()=>{if(!busy){setPending(null);(trigger.current?.isConnected?trigger.current:heading.current)?.focus();}};
  const resource=data.data?.resource;
  const confirm=async()=>{
    if(!pending || busy)return;
    setBusy(true);setError("");setMessage("");
    try {
      const result=await actOnExtensionResource(selection,pending.action,pending.uid,pending.resourceVersion);
      if(!result.requested)throw new Error("The action was not acknowledged; refresh to check the resource.");
      if(alive.current){setPending(null);setMessage("Request accepted. The controller will report progress in resource status.");}
    }catch(e){if(alive.current){setError(e instanceof Error?e.message:String(e));setPending(null);}}
    finally{if(alive.current)setBusy(false);}
  };
  const supported=(data.data?.actions??[]).filter(action=>!!data.data?.actionMeta?.[action]?.title);
  const label=(action:string)=>plainText(data.data?.actionMeta?.[action]?.title??action);
  // The declared reason is drawn through `plainText` because #551 makes these
  // predicates a manifest's, and an app's sentence must not be able to reorder
  // or hide the host's words around it.
  const excuse=(action:string)=>{
    const unmet=resource&&unmetPredicate(data.data?.actionMeta?.[action]?.availableWhen??[],resource);
    return unmet?plainText(unmet.reason):undefined;
  };
  // A palette action command (#544) asks for this view's own review, never for
  // the write. Only a resource's own tab answers: a peek of the same resource
  // beside a list would otherwise raise a second review for one request.
  const [requested,setRequested]=useState<string|null>(null);
  useEffect(()=>{
    if(!fullPage)return;
    const take=()=>{const action=takeExtensionAction(selection);if(action)setRequested(action);};
    take();
    return onExtensionActionRequested(take);
  // The selection's fields are its identity; the object itself is rebuilt every render.
  },[fullPage,selection.id,selection.capability,selection.context,selection.namespace,selection.name]);
  useEffect(()=>{
    if(!requested)return;
    // A failed read answers the request: the error is on screen, and a Retry
    // the reader presses later must not open a review they did not ask for then.
    if(data.status==="error"){setRequested(null);return;}
    if(data.status!=="ready" || !resource || busy)return;
    setRequested(null);
    if(!supported.includes(requested)){setError(`${plainText(requested)} is not offered for this resource.`);return;}
    const unavailable=excuse(requested);
    if(unavailable){setError(`${label(requested)} is unavailable: ${unavailable}`);return;}
    // The review pins the UID and resourceVersion the reader saw; without them
    // there is nothing to pin, and a request dropped without a word reads as
    // the palette doing nothing.
    if(!resource.metadata.uid || !resource.metadata.resourceVersion){
      setError(`${label(requested)} cannot be reviewed: the host returned this resource with no UID or resourceVersion to pin the write to. Refresh details and try again.`);
      return;
    }
    trigger.current=document.activeElement as HTMLElement;
    setError("");setMessage("");
    setPending({action:requested,uid:resource.metadata.uid,resourceVersion:resource.metadata.resourceVersion});
  // Once per request, against the resource shown when it can be answered.
  },[requested,data.status,resource,busy]);
  const kind=resource?.kind ?? "Resource";
  const verdict=data.data?.status;
  const word=verdict&&statusWord(verdict);
  const tone=verdict&&statusKind(verdict);
  const created=resource?.metadata.creationTimestamp;
  const target=describeTarget(kind,selection.namespace||null,selection.name);
  // Blocked with a reason rather than `disabled`, so focus can reach the sentence.
  const held=busy?"Waiting for the request to finish":pending?"Finish the open review first":undefined;
  const unpinned=resource && (!resource.metadata.uid || !resource.metadata.resourceVersion)
    ? "The host returned this resource with no UID or resourceVersion to pin a write to":undefined;
  const actions:ActionBarAction[]=[
    // Prefixed, because a manifest names its own actions and can name one `refresh`.
    ...(data.status==="ready" && resource ? supported.map(action=>({
      id:`action:${action}`,
      label:label(action),
      danger:data.data?.actionMeta?.[action]?.impact==="high",
      disabledReason:held ?? excuse(action) ?? unpinned,
      onSelect:()=>{trigger.current=document.activeElement as HTMLElement;setError("");setMessage("");setPending({action,uid:resource.metadata.uid,resourceVersion:resource.metadata.resourceVersion});},
    })) : []),
    {id:"refresh-details",label:"Refresh details",disabledReason:held,onSelect:()=>{setError("");setMessage("");refresh();}},
  ];
  const actionRow=<DetailActionRow context={selection.context} subject={target} flagged={tone==="danger"||tone==="warning"} actions={actions} label={`${kind} actions`} host={fullPage?"tab":"peek"}/>;
  const pane=<>
    {error&&<FailureAlert tone="sev" title="Could not run this action" error={error}/>}
    {message&&<p role="status" className="extension-message">{message}</p>}
    {data.status==="error"?<FailureState title={`Could not load ${target}`} error={data.error} onRetry={data.reload}/>:data.status==="loading"?<LoadingState label={`Loading ${target}`}/>:resource&&<>
      {pending&&<div className="extension-action-review" role="dialog" aria-label={`Review ${label(pending.action)}`} tabIndex={-1} ref={review}>
        {/* The one host confirmation (#552), in this screen's own frame. The
            frame is all this surface supplies: the sentence, the level, the
            cluster, the resource and the requester are the component's, and
            they are identical to what an agent asking for this same write is
            shown. */}
        <HostConfirmation
          question={hostQuestion(data.data,pending.action,selection,resource?.kind)}
          impact={data.data?.actionMeta?.[pending.action]?.impact ?? null}
          cluster={selection.context}
          subject={{kind:"object",namespace:selection.namespace||null,name:selection.name}}
          app={app}
          details={typed.field}
          actions={<>
            <Button disabled={busy||!typed.ready} onClick={()=>void confirm()}>{busy?"Requesting…":`Confirm ${label(pending.action)}`}</Button>
            <Button variant="outline" disabled={busy} onClick={cancel}>Cancel</Button>
          </>}
        />
      </div>}
      {tab==="yaml"?<div className="flex h-full min-h-0 flex-col"><div className="min-h-0 flex-1"><CodeEditor value={formatResourceManifest(resource)} readOnly language="yaml" fill copy ariaLabel={`${selection.name} manifest`}/></div></div>
      :tab==="events"?data.data?.eventsError ? <FailureState title="Could not list events" error={data.data.eventsError} onRetry={data.reload}/> : <Events detail={data.data} kind={kind} selection={selection}/>
      :<Overview selection={selection} resource={resource} verdict={verdict} fullPage={fullPage}/>}
    </>}
  </>;
  const tiles:Tile[]=[
    ...(verdict?[{key:"status",label:"Status",value:<StatusPill status={word!} kind={tone!} tinted/>}]:[]),
    ...(created?[{key:"age",label:"Age",value:<AgeCell created={created}/>}]:[]),
  ];
  return <div className={`extension-resource-detail${fullPage?" h-full min-h-0":""}`} ref={heading} tabIndex={-1} onKeyDownCapture={e=>{if(e.key==="Escape" && (pending || busy)){e.preventDefault();e.stopPropagation();if(pending)cancel();}}}>
    {fullPage
      ? <ResourceTabFrame name={selection.name} cluster={clusterLabel ?? selection.context} namespace={selection.namespace||null} kind={kind} actions={actionRow} tabs={TABS} active={tab} onTabChange={setTab} tiles={tiles}>{pane}</ResourceTabFrame>
      : <Inspector
      name={selection.name}
      nameAction={<CopyNameButton name={selection.name}/>}
      subtitle={selection.namespace ? `${kind} · ${selection.namespace}` : kind}
      status={word}
      statusKind={tone}
      flagged={tone==="danger"||tone==="warning"}
      // Only beside a verdict: a lone age reads as the rest having gone missing, so it is a fact below instead.
      facts={verdict&&created?[{label:"Age",value:<AgeCell created={created}/>,tone:"muted"}]:undefined}
      onClose={onClose ? ()=>{if(!busy)onClose();} : undefined}
      actions={openResource ? <OpenTabButton disabled={busy} onClick={()=>openResource(selection)}/> : undefined}
      tabs={TABS}
      activeTab={tab} onTabChange={setTab} tabsLabel="Resource views"
      footer={actionRow}
    >{pane}</Inspector>}
  </div>;
}
