import { useState } from "react";
import { ExtensionResourceDetails } from "../extensions/ExtensionResourceDetails";
import { ErrorNotice } from "../extensions/ExtensionResults";
import { SHARED_CONTEXT_ID_MESSAGE } from "../extensions/contextIds";
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
  const [legacyPin, setLegacyPin] = useState<{ route: string; id: string } | null>(null);
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
  // An already-open legacy name route learns its identity once; a later rename or
  // another context inheriting the old name must not move that tab.
  const identity = target.clusterId ?? (legacyPin?.route === route ? legacyPin.id : undefined);
  const cluster = identity
    ? contexts.find((c) => c.stableId === identity)
    : contexts.find((c) => c.name === target.context);
  if (!target.clusterId && cluster && legacyPin?.route !== route) {
    setLegacyPin({ route, id: cluster.stableId });
  }
  const clusterId = cluster?.stableId ?? target.context;
  // An app can only be opened once its cluster is listed, and a
  // listing that failed says nothing about whether the app is enabled there.
  const unchecked = Boolean(plugin) && !cluster;
  // A stable ID two contexts share does not say which was chosen; the host refuses both.
  const shared =
    !!plugin && !!cluster && contexts.filter((c) => c.stableId === cluster.stableId).length > 1;
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
        ) : unchecked ? (
          <p className="extension-message">
            This cluster is no longer in your kubeconfig files, so its apps cannot be opened here.
            Manage your kubeconfig files in Settings → Contexts.
          </p>
        ) : plugin && page && !extensionEnabledFor(plugin, cluster?.key) ? (
          <p className="extension-message">
            This app is not enabled for this cluster. Manage it in Settings → Apps.
          </p>
        ) : plugin && page ? (
          <ExtensionResourceNavigation.Provider value={resource=>openTab(extensionResourceRoute(clusterId,target.id,target.page,resource.namespace,resource.name),{clusterName:cluster?.name})}>
          {target.resourceName ? <ExtensionResourceDetails fullPage key={route} selection={{id:target.id,revision:plugin.revision,capability:page.capability,context:clusterId,namespace:target.namespace,name:target.resourceName}}/> : <ExtensionWorkspace
            plugin={plugin}
            page={page}
            onPage={(id, namespace) =>
              openTab(
                extensionRoute(clusterId, target.id, id, namespace),
                { clusterName: cluster?.name },
              )
            }
            context={clusterId}
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
