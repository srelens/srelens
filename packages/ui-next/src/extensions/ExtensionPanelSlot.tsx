import { NativeComponent } from "../native-components/NativeComponent";
import { Button } from "@srelens/ui-kit";
import { contributionKind, extensionEnabledFor, resolveExtensionPanels,
  type ExtensionPanelFormat, type ExtensionResolvedPanel, type InstalledExtension } from "@srelens/core";
import { plainText } from "./displayText";
import { useExtensions } from "./inventoryStore";
import { useContextLookup, refreshContextIds } from "./contextIds";
import { useResource } from "../lib/useResource";

function shown(value: string | null, format?: ExtensionPanelFormat): string {
  if (value === null) return "—";
  if (format === "number") {
    const number = Number(value);
    if (Number.isFinite(number)) return new Intl.NumberFormat().format(number);
  }
  if (format === "date") {
    const date = new Date(value);
    if (!Number.isNaN(date.getTime())) return new Intl.DateTimeFormat(undefined, { dateStyle:"medium", timeStyle:"short" }).format(date);
  }
  if (format === "duration") {
    const seconds = Number(value);
    if (Number.isFinite(seconds) && seconds >= 0) return seconds >= 3600
      ? `${Math.floor(seconds / 3600)}h ${Math.floor(seconds % 3600 / 60)}m`
      : seconds >= 60 ? `${Math.floor(seconds / 60)}m ${Math.floor(seconds % 60)}s` : `${seconds}s`;
  }
  return plainText(value);
}

export function ResolvedPanelView({ panel, onRetry }: { panel: ExtensionResolvedPanel; onRetry: () => void }) {
  return <section className="section extension-detail-panel" aria-label={panel.title}>
    <h4 className="extension-detail-heading">{plainText(panel.title)}</h4>
    {panel.sections.map((section, index) => section.type === "fields"
      ? <div key={index}>
        <NativeComponent label={`${panel.title} fields ${index + 1}`}
          payload={{ version:1, type:"KeyValue", data:{ items: section.fields.map(field => ({
            label: plainText(field.label), value: field.error ? `Couldn’t read: ${plainText(field.error)}` : shown(field.value, field.format),
          })) } }} onRetry={onRetry}/>
        {section.fields.some(field => field.error) && <Button type="button" variant="ghost" size="sm"
          onClick={onRetry}>Retry panel fields</Button>}
      </div>
      : <NativeComponent key={index} label={`${panel.title} conditions ${index + 1}`}
          payload={{ version:1, type:"Conditions", data:{ items:section.items } }}
          state={section.error ? { status:"error", error:section.error } : undefined} onRetry={onRetry}/>
    )}
  </section>;
}

type PanelResource = {
  apiVersion: string;
  kind: string;
  metadata: { name: string; namespace?: string; uid?: string; resourceVersion?: string };
  [key: string]: unknown;
};

function panelResource(value: unknown): value is PanelResource {
  if (!value || typeof value !== "object") return false;
  const resource = value as Partial<PanelResource>;
  return typeof resource.kind === "string" && typeof resource.apiVersion === "string"
    && !!resource.metadata && typeof resource.metadata.name === "string";
}

function PluginPanels({ plugin, context, kind, resource }: {
  plugin: InstalledExtension; context: string; kind: string; resource: PanelResource;
}) {
  const namespace = resource.metadata.namespace ?? "";
  const data = useResource(() => resolveExtensionPanels(
    plugin.manifest.id, plugin.revision, context, namespace, kind, resource,
  ), [plugin.manifest.id, plugin.revision, context, namespace, kind,
    resource.metadata.name, resource.metadata.uid, resource.metadata.resourceVersion]);
  const label = `${plugin.manifest.name} panels`;
  if (data.status === "loading") return <section className="section"><NativeComponent label={label} state={{status:"loading"}}/></section>;
  if (data.status === "error") return <section className="section"><NativeComponent label={label} state={{status:"error",error:data.error}} onRetry={data.reload}/></section>;
  return <>{data.data?.panels.map(panel => <ResolvedPanelView key={panel.id} panel={panel} onRetry={data.reload}/>)}</>;
}

/** Adds installed, enabled app panels after the Inspector's host-owned sections. */
export function ExtensionPanelSlot({ context, resource }: { context: string; resource: unknown }) {
  const inventory = useExtensions();
  const lookup = useContextLookup(context);
  if (inventory.status === "error") return <section className="section"><NativeComponent label="App panels"
    state={{status:"error",error:inventory.error}} onRetry={inventory.reload}/></section>;
  if (!panelResource(resource)) return null;
  const group = resource.apiVersion.includes("/") ? resource.apiVersion.split("/")[0] : "";
  const kind = contributionKind(resource.kind, group);
  const offered = (inventory.data?.plugins ?? []).filter(plugin => plugin.enabled
    && !plugin.quarantined && !plugin.policyBlocked
    && plugin.manifest.contributions.detailPanels?.some(panel => panel.forKinds.includes(kind)));
  const contextId = lookup.status === "found" ? lookup.id : undefined;
  const plugins = offered.filter(plugin => extensionEnabledFor(plugin, contextId));
  const limited = offered.some(plugin => plugin.contexts && !extensionEnabledFor(plugin, contextId));
  return <>
    {limited && lookup.status === "failed" && <section className="section"><NativeComponent label="App panels"
      state={{status:"error",error:`Could not list clusters: ${lookup.error}`}}
      onRetry={() => void refreshContextIds()}/></section>}
    {limited && lookup.status === "loading" && <section className="section"><NativeComponent label="App panels" state={{status:"loading"}}/></section>}
    {plugins.map(plugin => <PluginPanels key={`${plugin.manifest.id}/${plugin.revision}`}
      plugin={plugin} context={context} kind={kind} resource={resource}/>)}
  </>;
}
