import { Button } from "@srelens/ui-kit";
import { contributionKind, describeError, extensionClusterResourceRoute, extensionEnabledFor, resolveExtensionLinks,
  type ExtensionLinkRelation, type ExtensionResolvedLink, type InstalledExtension } from "@srelens/core";
import { NativeComponent } from "../native-components/NativeComponent";
import { plainText } from "./displayText";
import { useExtensions } from "./inventoryStore";
import { useContextLookup, refreshContextIds } from "./contextIds";
import { useResource } from "../lib/useResource";
import { openTab } from "../lib/tabsStore";
import { useContexts } from "../lib/clusters";

const RELATION: Record<ExtensionLinkRelation, string> = {
  ownedBy: "Owned by", managedBy: "Managed by", exposedBy: "Exposed by", references: "References",
};

type LinkResource = { apiVersion: string; kind: string;
  metadata: { name: string; namespace?: string; uid?: string; resourceVersion?: string } };

function linkResource(value: unknown): value is LinkResource {
  if (!value || typeof value !== "object") return false;
  const resource = value as Partial<LinkResource>;
  return typeof resource.kind === "string" && typeof resource.apiVersion === "string"
    && !!resource.metadata && typeof resource.metadata.name === "string";
}

/** What one app answered: its links, or why the call failed. */
type AppLinks = { plugin: InstalledExtension; links?: ExtensionResolvedLink[]; error?: string };

/** The app page a link target opens on: the reader's own list page, not a dashboard over it. */
function pageFor(plugin: InstalledExtension, capability: string) {
  const pages = plugin.manifest.contributions.pages.filter(page => page.capability === capability);
  return (pages.find(page => !page.dashboard) ?? pages[0])?.id;
}

function LinkRow({ plugin, link, contextKey, clusterName }: {
  plugin: InstalledExtension; link: ExtensionResolvedLink; contextKey?: string; clusterName?: string;
}) {
  const kind = link.to.split("/").pop() ?? link.to;
  const relation = RELATION[link.relation] ?? link.relation;
  const page = pageFor(plugin, link.capability);
  if (link.error) return <li className="extension-related-error">
    {relation} {plainText(kind)}: Couldn’t read — {plainText(describeError(link.error).detail)}
  </li>;
  return <>{link.targets.map(target => {
    const where = target.namespace ? `${target.namespace}/${target.name}` : target.name;
    const text = `${relation} ${kind} ${where}`;
    // One name can come back twice — a live owner and a deleted one the
    // resource still references — and the host dedupes whole targets, so
    // the state is what tells two rows of one name apart.
    const state = target.unverified ? "unverified" : target.exists ? "found" : "missing";
    const key = `${where}\u0000${state}`;
    // Not looked up is not missing: say why the host did not look.
    if (target.unverified) return <li key={key}>{plainText(text)} — {plainText(target.unverified)}</li>;
    if (!target.exists) return <li key={key}>{plainText(text)} — not found on this cluster</li>;
    // No page, or no listed context to name the tab by: the target as plain text.
    if (!page || contextKey === undefined) return <li key={key}>{plainText(text)}</li>;
    return <li key={key}><Button type="button" variant="ghost" size="sm"
      onClick={() => openTab(extensionClusterResourceRoute(contextKey, plugin.manifest.id, page, target.namespace ?? "", target.name),
        { clusterName })}>{plainText(text)}</Button></li>;
  })}</>;
}

function RelatedLinks({ plugins, context, kind, resource }: {
  plugins: InstalledExtension[]; context: string; kind: string; resource: LinkResource;
}) {
  const contexts = useContexts();
  // The Inspector names its cluster by display name; an app's resource page by the pinned ID it
  // reads by. A pinned ID first: the host never resolves its reserved form to a context merely
  // named so. (While both are listed, it resolves such a string to neither, and the links fail
  // there.) The route carries the key (#695).
  const cluster = contexts.find(c => c.pinnedId === context) ?? contexts.find(c => c.name === context);
  const namespace = resource.metadata.namespace ?? "";
  // The resolver reads the resource's identity and metadata and nothing else:
  // a ConfigMap's data, a spec or a Secret's values never leave the Inspector.
  const identity = { apiVersion: resource.apiVersion, kind: resource.kind, metadata: resource.metadata };
  const data = useResource<AppLinks[]>(() => Promise.all(plugins.map(plugin =>
    resolveExtensionLinks(plugin.manifest.id, plugin.revision, context, namespace, kind, identity).then(
      answer => ({ plugin, links: answer.links }),
      (error: unknown) => ({ plugin, error: describeError(error).detail }),
    ))), [plugins.map(p => `${p.manifest.id}/${p.revision}`).join(","), context, namespace, kind,
    resource.metadata.name, resource.metadata.uid, resource.metadata.resourceVersion], () => false);
  if (data.status === "loading") return <NativeComponent label="Related resources" state={{ status:"loading" }}/>;
  if (data.status === "error") return <NativeComponent label="Related resources"
    state={{ status:"error", error:data.error ?? "" }} onRetry={data.reload}/>;
  const answers = data.data ?? [];
  const failed = answers.some(answer => answer.error || answer.links?.some(link => link.error));
  const none = !failed && answers.every(answer => answer.links?.every(link => link.targets.length === 0));
  return <>
    {none ? <p className="extension-message">No related resources.</p> : <ul className="extension-related">
      {answers.map(answer => answer.error
        ? <li key={answer.plugin.manifest.id} className="extension-related-error">
          Couldn’t read related resources from {plainText(answer.plugin.manifest.name)}: {plainText(answer.error)}</li>
        : answer.links?.map(link => <LinkRow key={`${answer.plugin.manifest.id}/${link.id}`} plugin={answer.plugin}
          link={link} contextKey={cluster?.key} clusterName={cluster?.name}/>))}
    </ul>}
    {failed && <Button type="button" variant="ghost" size="sm" onClick={data.reload}>Retry related resources</Button>}
  </>;
}

/**
 * The Inspector's "Related" section (#545): the resources installed apps say
 * this one is owned by, managed by, exposed by or references. Each target
 * opens its app's resource route, which carries the cluster and the app page
 * whose reader pins the target's group and kind, so two targets are two tabs.
 */
export function ExtensionRelatedSlot({ context, resource }: { context: string; resource: unknown }) {
  const inventory = useExtensions();
  const lookup = useContextLookup(context);
  if (!linkResource(resource)) return null;
  const group = resource.apiVersion.includes("/") ? resource.apiVersion.split("/")[0] : "";
  const kind = contributionKind(resource.kind, group);
  if (inventory.status === "error") return <section className="section" aria-label="Related">
    <h4 className="extension-detail-heading">Related</h4>
    <NativeComponent label="Related resources" state={{ status:"error", error:inventory.error ?? "" }} onRetry={inventory.reload}/>
  </section>;
  // Until the inventory answers, whether an app links from this kind is
  // unknown: say so, rather than render the same nothing as "none does".
  if (inventory.status === "loading" && !inventory.data) return <section className="section" aria-label="Related">
    <h4 className="extension-detail-heading">Related</h4>
    <NativeComponent label="Related resources" state={{ status:"loading" }}/>
  </section>;
  const offered = (inventory.data?.plugins ?? []).filter(plugin => plugin.enabled
    && !plugin.quarantined && !plugin.policyBlocked
    && plugin.manifest.contributions.resourceLinks?.some(link => link.from === kind));
  if (!offered.length) return null;
  const contextId = lookup.status === "found" ? lookup.id : undefined;
  const plugins = offered.filter(plugin => extensionEnabledFor(plugin, contextId));
  const limited = offered.some(plugin => plugin.contexts && !extensionEnabledFor(plugin, contextId));
  if (!plugins.length && !(limited && (lookup.status === "failed" || lookup.status === "loading"))) return null;
  return <section className="section extension-detail-panel" aria-label="Related">
    <h4 className="extension-detail-heading">Related</h4>
    {limited && lookup.status === "failed" && <NativeComponent label="Related resources"
      state={{ status:"error", error:`Could not list clusters: ${lookup.error}` }} onRetry={() => void refreshContextIds()}/>}
    {limited && lookup.status === "loading" && <NativeComponent label="Related resources" state={{ status:"loading" }}/>}
    {plugins.length > 0 && <RelatedLinks plugins={plugins} context={context} kind={kind} resource={resource}/>}
  </section>;
}
