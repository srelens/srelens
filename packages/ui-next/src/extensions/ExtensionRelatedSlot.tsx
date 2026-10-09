import { Button } from "@srelens/ui-kit";
import { contributionKind, describeError, extensionClusterResourceRoute, extensionEnabledFor, resolveExtensionLinks,
  resolveExtensionReverseLinks, type ExtensionLinkRelation, type ExtensionLinkTarget, type ExtensionResolvedLink,
  type ExtensionReverseLink, type InstalledExtension } from "@srelens/core";
import { NativeComponent } from "../native-components/NativeComponent";
import { plainText } from "./displayText";
import { useExtensions } from "./inventoryStore";
import { useContextLookup, refreshContextIds } from "./contextIds";
import { useResource } from "../lib/useResource";
import { openTab } from "../lib/tabsStore";
import { useActiveContext, useContexts } from "../lib/clusters";
import { detailRoute } from "../lib/detailRoute";

const RELATION: Record<ExtensionLinkRelation, string> = {
  ownedBy: "Owned by", managedBy: "Managed by", exposedBy: "Exposed by", references: "References",
};

/** The same relation read from its target (#728): what this resource is to the ones that name it. */
const REVERSE: Record<ExtensionLinkRelation, string> = {
  ownedBy: "Owns", managedBy: "Manages", exposedBy: "Exposes", references: "Referenced by",
};

type LinkResource = { apiVersion: string; kind: string;
  metadata: { name: string; namespace?: string; uid?: string; resourceVersion?: string } };

function linkResource(value: unknown): value is LinkResource {
  if (!value || typeof value !== "object") return false;
  const resource = value as Partial<LinkResource>;
  return typeof resource.kind === "string" && typeof resource.apiVersion === "string"
    && !!resource.metadata && typeof resource.metadata.name === "string";
}

/** What one app answered: its links each way, or why a call failed. */
type AppLinks = { plugin: InstalledExtension; links?: ExtensionResolvedLink[]; reverse?: ExtensionReverseLink[];
  error?: string; reverseError?: string };

/** The app page a link target opens on: the reader's own list page, not a dashboard over it. */
function pageFor(plugin: InstalledExtension, capability: string) {
  const pages = plugin.manifest.contributions.pages.filter(page => page.capability === capability);
  return (pages.find(page => !page.dashboard) ?? pages[0])?.id;
}

/** Where the Inspector's links open: the cluster it shows, and whether the rail shows it too. */
type Opener = { contextKey?: string; clusterName?: string; onRail: boolean };

/**
 * The route a linked resource opens on, or none when it cannot open truthfully.
 *
 * An app's kind opens its app page, whose route carries the cluster's key (#695). A
 * built-in kind (`capability` empty, #728) opens the host's own Inspector, whose detail
 * route follows the rail — so only when the rail shows the Inspector's cluster, or the
 * tab would show the rail cluster's namesake.
 */
function routeFor(plugin: InstalledExtension, kind: string, capability: string, target: ExtensionLinkTarget, at: Opener) {
  if (at.contextKey === undefined) return undefined;
  if (!capability) return at.onRail ? detailRoute(kind, target.namespace, target.name) : undefined;
  const page = pageFor(plugin, capability);
  return page && extensionClusterResourceRoute(at.contextKey, plugin.manifest.id, page, target.namespace ?? "", target.name);
}

function TargetRow({ text, route, target, clusterName }: {
  text: string; route?: string; target: ExtensionLinkTarget; clusterName?: string;
}) {
  // Not looked up is not missing: say why the host did not look.
  if (target.unverified) return <li>{plainText(text)} — {plainText(target.unverified)}</li>;
  if (!target.exists) return <li>{plainText(text)} — not found on this cluster</li>;
  // No page, or no cluster to open it on truthfully: the target as plain text.
  if (!route) return <li>{plainText(text)}</li>;
  return <li><Button type="button" variant="ghost" size="sm" onClick={() => openTab(route, { clusterName })}>{plainText(text)}</Button></li>;
}

/** One row's key: one name can come back twice, a live resource and a gone one, so the state tells them apart. */
function targetKey(target: ExtensionLinkTarget) {
  const where = target.namespace ? `${target.namespace}/${target.name}` : target.name;
  return `${where}\u0000${target.unverified ? "unverified" : target.exists ? "found" : "missing"}`;
}

function where(target: ExtensionLinkTarget) {
  return target.namespace ? `${target.namespace}/${target.name}` : target.name;
}

function LinkRow({ plugin, link, at }: { plugin: InstalledExtension; link: ExtensionResolvedLink; at: Opener }) {
  const kind = link.to.split("/").pop() ?? link.to;
  const relation = RELATION[link.relation] ?? link.relation;
  if (link.error) return <li className="extension-related-error">
    {relation} {plainText(kind)}: Couldn’t read — {plainText(describeError(link.error).detail)}
  </li>;
  return <>{link.targets.map(target => <TargetRow key={targetKey(target)} text={`${relation} ${kind} ${where(target)}`}
    route={routeFor(plugin, kind, link.capability, target, at)} target={target} clusterName={at.clusterName}/>)}</>;
}

/** One reverse link's rows under its relation's group: the sources, and what the host could not say. */
function ReverseRows({ plugin, link, at }: { plugin: InstalledExtension; link: ExtensionReverseLink; at: Opener }) {
  const kind = link.from.split("/").pop() ?? link.from;
  // The group's label says the relation; the row says which kind could not be read.
  if (link.error) return <li className="extension-related-error">
    {plainText(kind)}: Couldn’t read — {plainText(describeError(link.error).detail)}
  </li>;
  return <>
    {link.sources.map(source => <TargetRow key={targetKey(source)} text={`${kind} ${where(source)}`}
      route={routeFor(plugin, kind, link.capability, source, at)} target={source} clusterName={at.clusterName}/>)}
    {link.truncated && <li className="extension-related-note">
      Found among the first 2,000 {plainText(kind)} resources the host read; there may be more.</li>}
    {link.unreadable && <li className="extension-related-note">{plainText(link.unreadable)}</li>}
  </>;
}

function RelatedLinks({ plugins, context, kind, resource }: {
  plugins: InstalledExtension[]; context: string; kind: string; resource: LinkResource;
}) {
  const contexts = useContexts();
  const active = useActiveContext();
  // The Inspector names its cluster by display name; an app's resource page by the pinned ID it
  // reads by. A pinned ID first: the host never resolves its reserved form to a context merely
  // named so. (While both are listed, it resolves such a string to neither, and the links fail
  // there.) The route carries the key (#695).
  const cluster = contexts.find(c => c.pinnedId === context) ?? contexts.find(c => c.name === context);
  const at: Opener = { contextKey: cluster?.key, clusterName: cluster?.name,
    onRail: !!cluster && active?.key === cluster.key };
  const namespace = resource.metadata.namespace ?? "";
  // The resolvers read the resource's identity and metadata and nothing else: a ConfigMap's
  // data, a spec or a Secret's values never leave the Inspector. A path link's resource body
  // is the host's own read, through the app's reader (#728).
  const identity = { apiVersion: resource.apiVersion, kind: resource.kind, metadata: resource.metadata };
  const data = useResource<AppLinks[]>(() => Promise.all(plugins.map(async plugin => {
    const declared = plugin.manifest.contributions.resourceLinks ?? [];
    const [forward, reverse] = await Promise.all([
      declared.some(link => link.from === kind)
        ? resolveExtensionLinks(plugin.manifest.id, plugin.revision, context, namespace, kind, identity).then(
          answer => ({ links: answer.links }), (error: unknown) => ({ error: describeError(error).detail }))
        : {},
      declared.some(link => link.to === kind)
        ? resolveExtensionReverseLinks(plugin.manifest.id, plugin.revision, context, namespace, kind, identity).then(
          answer => ({ reverse: answer.links }), (error: unknown) => ({ reverseError: describeError(error).detail }))
        : {},
    ]);
    return { plugin, ...forward, ...reverse };
  })), [plugins.map(p => `${p.manifest.id}/${p.revision}`).join(","), context, namespace, kind,
    resource.metadata.name, resource.metadata.uid, resource.metadata.resourceVersion], () => false);
  if (data.status === "loading") return <NativeComponent label="Related resources" state={{ status:"loading" }}/>;
  if (data.status === "error") return <NativeComponent label="Related resources"
    state={{ status:"error", error:data.error ?? "" }} onRetry={data.reload}/>;
  const answers = data.data ?? [];
  const failed = answers.some(answer => answer.error || answer.reverseError
    || answer.links?.some(link => link.error) || answer.reverse?.some(link => link.error));
  const none = !failed
    && answers.every(answer => answer.links?.every(link => link.targets.length === 0) ?? true)
    && answers.every(answer => answer.reverse?.every(link => link.sources.length === 0 && !link.truncated
      && !link.unreadable) ?? true);
  // The reverse links, grouped under the word each relation reads as from here, in a fixed order.
  const groups = (Object.keys(REVERSE) as ExtensionLinkRelation[]).map(relation => ({ relation,
    links: answers.flatMap(answer => (answer.reverse ?? []).filter(link => link.relation === relation)
      .map(link => ({ plugin: answer.plugin, link }))) }))
    .filter(group => group.links.some(({ link }) => link.error || link.sources.length || link.truncated || link.unreadable));
  const callFailures = answers.flatMap(answer => [answer.error, answer.reverseError].filter(Boolean)
    .map((why, index) => <li key={`${answer.plugin.manifest.id}/${index}`} className="extension-related-error">
      Couldn’t read related resources from {plainText(answer.plugin.manifest.name)}: {plainText(why ?? "")}</li>));
  // A link that named nothing draws no row, so it does not hold the list open.
  const forward = answers.flatMap(answer => (answer.links ?? []).filter(link => link.error || link.targets.length)
    .map(link => <LinkRow
    key={`${answer.plugin.manifest.id}/${link.id}`} plugin={answer.plugin} link={link} at={at}/>));
  return <>
    {none ? <p className="extension-message">No related resources.</p> : <>
      {(callFailures.length > 0 || forward.length > 0) && <ul className="extension-related">{callFailures}{forward}</ul>}
      {groups.map(group => <div key={group.relation} role="group" aria-label={REVERSE[group.relation]}>
        <p className="extension-related-group">{REVERSE[group.relation]}</p>
        <ul className="extension-related">
          {group.links.map(({ plugin, link }) => <ReverseRows key={`${plugin.manifest.id}/${link.id}`} plugin={plugin}
            link={link} at={at}/>)}
        </ul>
      </div>)}
    </>}
    {failed && <Button type="button" variant="ghost" size="sm" onClick={data.reload}>Retry related resources</Button>}
  </>;
}

/**
 * The Inspector's "Related" section (#545): the resources installed apps say
 * this one is owned by, managed by, exposed by or references, and — read from
 * the same declarations the other way round (#728) — the ones that say so of it.
 * Each app resource opens its app's resource route, which carries the cluster
 * and the app page whose reader pins its group and kind, so two targets are
 * two tabs; a built-in resource opens the host's own Inspector.
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
    && plugin.manifest.contributions.resourceLinks?.some(link => link.from === kind || link.to === kind));
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
