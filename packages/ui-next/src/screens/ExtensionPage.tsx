import { extensionRoute, parseExtensionRoute } from "@srelens/core";
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
  const plugin = inventory.data?.developerMode
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
      title={page?.title ?? "Extension"}
      eyebrow={getContextLabel(cluster?.stableId ?? "", target.context)}
      fill
    >
      <div className="scroll min-h-0 min-w-0 flex-1">
        {inventory.status === "loading" ? (
          <p className="extension-message" role="status">
            Loading extension…
          </p>
        ) : inventory.status === "error" ? (
          <div role="alert" className="extension-message">
            {inventory.error}
            <Button onClick={inventory.reload}>Retry</Button>
          </div>
        ) : plugin && page ? (
          <ExtensionWorkspace
            plugin={plugin}
            page={page}
            onPage={(id) =>
              openTab(
                extensionRoute(target.context, target.id, id, target.namespace),
              )
            }
            context={target.context}
            namespace={target.namespace}
          />
        ) : (
          <p className="extension-message">
            This extension page is disabled, removed, or no longer available.
            Manage it in Settings → Extensions.
          </p>
        )}
      </div>
    </Screen>
  );
}
