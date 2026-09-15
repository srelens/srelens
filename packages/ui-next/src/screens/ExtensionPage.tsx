import { ExtensionResourceDetails } from "../extensions/ExtensionResourceDetails";
import { ExtensionResourceNavigation } from "../extensions/resourceNavigation";
import { extensionEnabledFor, extensionRoute, extensionResourceRoute, parseExtensionRoute } from "@srelens/core";
import { Button, Screen } from "@srelens/ui-kit";
import { useExtensions } from "../extensions/Extensions";
import { ExtensionWorkspace } from "../extensions/ExtensionWorkspace";
import { openTab } from "../lib/tabsStore";
import { useContexts } from "../lib/clusters";
import { getContextLabel } from "../lib/marks";
import type { RoutedScreenProps } from "../lib/routes";

/** Extension destinations carry their cluster in the route, independent of the rail. */
export function ExtensionPage({ route }: RoutedScreenProps) {
  const target = parseExtensionRoute(route);
  const inventory = useExtensions();
  const contexts = useContexts();
  if (!target) return null;
  const plugin = inventory.data
    ? inventory.data.plugins.find(
        (p) => p.enabled && p.manifest.id === target.id,
      )
    : undefined;
  const page = plugin?.manifest.contributions.pages.find(
    (p) => p.id === target.page,
  );
  const cluster = contexts.find((c) => c.name === target.context);
  return (
    <Screen
      title={target.resourceName ?? page?.title ?? "App"}
      eyebrow={getContextLabel(cluster?.stableId ?? "", target.context)}
      fill
    >
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        {inventory.status === "loading" ? (
          <p className="extension-message" role="status">
            Loading app…
          </p>
        ) : inventory.status === "error" ? (
          <div role="alert" className="extension-message">
            {inventory.error}
            <Button onClick={inventory.reload}>Retry</Button>
          </div>
        ) : plugin && page && !extensionEnabledFor(plugin, cluster?.stableId) ? (
          <p className="extension-message">
            This app is not enabled for this cluster. Manage it in Settings → Apps.
          </p>
        ) : plugin && page ? (
          <ExtensionResourceNavigation.Provider value={resource=>openTab(extensionResourceRoute(target.context,target.id,target.page,resource.namespace,resource.name),{clusterName:target.context})}>
          {target.resourceName ? <ExtensionResourceDetails fullPage key={route} selection={{id:target.id,revision:plugin.revision,capability:page.capability,context:target.context,namespace:target.namespace,name:target.resourceName}}/> : <ExtensionWorkspace
            plugin={plugin}
            page={page}
            onPage={(id, namespace) =>
              openTab(
                extensionRoute(target.context, target.id, id, namespace),
              )
            }
            context={target.context}
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
