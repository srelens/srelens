import { useState } from "react";
import { ExtensionResourceDetails } from "../extensions/ExtensionResourceDetails";
import { ErrorNotice } from "../extensions/ExtensionResults";
import { NO_PINNED_ID_MESSAGE, SHARED_CONTEXT_ID_MESSAGE } from "../extensions/contextIds";
import { plainText } from "../extensions/displayText";
import { ExtensionResourceNavigation } from "../extensions/resourceNavigation";
import { extensionEnabledFor, extensionClusterRoute as extensionRoute, extensionClusterResourceRoute as extensionResourceRoute, listContexts, parseExtensionRoute } from "@srelens/core";
import { Button, Screen } from "@srelens/ui-kit";
import { useExtensions } from "../extensions/Extensions";
import { ExtensionWorkspace } from "../extensions/ExtensionWorkspace";
import { openTab } from "../lib/tabsStore";
import { getContexts, getKubeconfigFiles, setContexts, useContexts, useContextsError, useContextsStatus } from "../lib/clusters";
import { getContextLabel } from "../lib/marks";
import type { RoutedScreenProps } from "../lib/routes";

/** Lists the contexts again and writes the answer back through the store, as Connections does. */
async function relistContexts() {
  const outcome = await listContexts(getKubeconfigFiles());
  setContexts([...(outcome.contexts ?? getContexts())], outcome.error ?? "");
}

/** Extension destinations carry their cluster in the route, independent of the rail. */
export function ExtensionPage({ route }: RoutedScreenProps) {
  const target = parseExtensionRoute(route);
  const [legacyPin, setLegacyPin] = useState<{ route: string; key: string } | null>(null);
  const inventory = useExtensions();
  const contexts = useContexts();
  const contextsStatus = useContextsStatus();
  const contextsError = useContextsError();
  if (!target) return null;
  const plugin = inventory.data
    ? inventory.data.plugins.find(
        (p) => p.enabled && p.manifest.id === target.id,
      )
    : undefined;
  const page = plugin?.manifest.contributions.pages.find(
    (p) => p.id === target.page,
  );
  // The route names its cluster by context key (#695), which no two contexts share. A tab
  // opened before names it by stable ID, which two can share (#623): that one resolves only
  // while a single context carries it. Older still, a display name: an already-open legacy
  // route learns its key once, so a later rename or another context inheriting the old name
  // must not move that tab.
  const key = target.contextKey ?? (legacyPin?.route === route ? legacyPin.key : undefined);
  const holders = target.clusterId === undefined ? [] : contexts.filter((c) => c.stableId === target.clusterId);
  const cluster = key !== undefined
    ? contexts.find((c) => c.key === key)
    : target.clusterId !== undefined
      ? (holders.length === 1 ? holders[0] : undefined)
      : contexts.find((c) => c.name === target.context);
  if (target.contextKey === undefined && target.clusterId === undefined && cluster && legacyPin?.route !== route) {
    setLegacyPin({ route, key: cluster.key });
  }
  // What this page's own links carry: the route identity, the key. Without a cluster nothing
  // below reads or links.
  const contextKey = cluster?.key ?? "";
  // What the host is asked: the pinned ID, which names this context alone. A key is not
  // enough there: it can be spelled the same as another context's stable ID.
  const hostContext = cluster?.pinnedId ?? "";
  // An app can only be opened once its cluster is listed, and a
  // listing that failed says nothing about whether the app is enabled there.
  const unchecked = Boolean(plugin) && !cluster;
  // A stable ID two contexts share does not say which this tab was opened for.
  const shared = !!plugin && holders.length > 1;
  // A dashboard card's target (#540): the same page, narrowed to what the card
  // counted. Only a card that still names this page narrows it.
  const card = target.card
    ? plugin?.manifest.contributions.dashboardCards?.find((c) => c.id === target.card && c.target?.page === target.page)
    : undefined;
  const showAll = () =>
    openTab(extensionRoute(contextKey, target.id, target.page, target.namespace), { clusterName: cluster?.name });
  return (
    <Screen
      title={target.resourceName ?? page?.title ?? "App"}
      eyebrow={getContextLabel(cluster?.stableId ?? "", cluster?.name ?? target.context)}
      fill
    >
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        {inventory.status === "loading" || (unchecked && contextsStatus === "loading") ? (
          <p className="extension-message" role="status">
            Loading app…
          </p>
        ) : inventory.status === "error" ? (
          <div role="alert" className="extension-message">
            {inventory.error}
            <Button onClick={inventory.reload}>Retry</Button>
          </div>
        ) : unchecked && contextsStatus === "failed" ? (
          <ErrorNotice
            title="Could not list clusters"
            message={contextsError}
            retry={() => void relistContexts()}
          />
        ) : shared ? (
          <p className="extension-message">{SHARED_CONTEXT_ID_MESSAGE}</p>
        ) : plugin && cluster && !cluster.pinnedId ? (
          <p className="extension-message">{NO_PINNED_ID_MESSAGE}</p>
        ) : unchecked ? (
          <p className="extension-message">
            This cluster is no longer in your kubeconfig files, so its apps cannot be opened here.
            Manage your kubeconfig files in Settings → Contexts.
          </p>
        ) : plugin && page && !extensionEnabledFor(plugin, cluster?.key) ? (
          <p className="extension-message">
            This app is not enabled for this cluster. Manage it in Settings → Apps.
          </p>
        ) : plugin && page && target.card && !card ? (
          // A card the app has since dropped: filtering by it is impossible, and
          // showing every row would present the whole list as the card's answer.
          <div className="extension-message">
            <p>{plainText(plugin.manifest.name)} no longer declares this dashboard card, so there is nothing to narrow this list to.</p>
            <Button variant="secondary" onClick={showAll}>Show all {plainText(page.title)}</Button>
          </div>
        ) : plugin && page ? (
          <ExtensionResourceNavigation.Provider value={resource=>openTab(extensionResourceRoute(contextKey,target.id,target.page,resource.namespace,resource.name),{clusterName:cluster?.name})}>
          {card && (
            <div role="status" className="extension-card-filter">
              <span>
                Showing the {plainText(page.title)} counted by <strong>{plainText(card.title)}</strong>
                {target.namespaces && ` in ${target.namespaces.slice(0, -1).join(", ")} and ${target.namespaces[target.namespaces.length - 1]}`}
              </span>
              <Button variant="ghost" size="sm" onClick={showAll}>Show all {plainText(page.title)}</Button>
            </div>
          )}
          {target.resourceName ? <ExtensionResourceDetails fullPage key={route} selection={{id:target.id,revision:plugin.revision,capability:page.capability,context:hostContext,namespace:target.namespace,name:target.resourceName}}/> : <ExtensionWorkspace
            card={card?.id}
            cardNamespaces={target.namespaces}
            onLeaveCard={(namespace) =>
              openTab(extensionRoute(contextKey, target.id, target.page, namespace), { clusterName: cluster?.name })
            }
            plugin={plugin}
            page={page}
            onPage={(id, namespace) =>
              openTab(
                extensionRoute(contextKey, target.id, id, namespace),
                { clusterName: cluster?.name },
              )
            }
            context={hostContext}
            namespace={target.namespace}
          />}
          </ExtensionResourceNavigation.Provider>
        ) : (
          <p className="extension-message">
            This app page is disabled, removed, or no longer available.
            Manage it in Settings → Apps.
          </p>
        )}
      </div>
    </Screen>
  );
}
