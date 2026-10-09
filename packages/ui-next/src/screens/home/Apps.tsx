import { extensionClusterRoute, extensionEnabledFor, type InstalledExtension } from "@srelens/core";
import { Badge, Button, LoadingState, type BadgeTone } from "@srelens/ui-kit";
import { ExtensionLogo } from "../../extensions/ExtensionLogo";
import { extensionLabel, useExtensions } from "../../extensions/inventoryStore";
import { useActiveContext } from "../../lib/clusters";
import { FailureAlert } from "../../lib/errorCopy";
import { openSettings } from "../../lib/settingsRequest";
import { openTab } from "../../lib/tabsStore";

/**
 * The one word the inventory supports for an app, and its tone. Only what
 * `extensions.list` reports: a quarantine, a policy block, and the switch.
 * Whether a sidecar is running, an update is waiting or a grant needs consent
 * is not in the inventory, so no chip claims it.
 */
function state(plugin: InstalledExtension): { word: string; tone: BadgeTone } {
  if (plugin.quarantined) return { word: "Quarantined", tone: "sev" };
  if (plugin.policyBlocked) return { word: "Blocked", tone: "warn" };
  return plugin.enabled ? { word: "On", tone: "ok" } : { word: "Off", tone: "muted" };
}

/**
 * Home's "Apps": what is installed, what the inventory says about each, a way
 * to each app's page, and the catalog.
 *
 * An app page is about a cluster, and Home is not, so "Open" lands on the
 * app's first page for the cluster in focus — named on the control. An app
 * with no page there (off, quarantined, not enabled for this cluster, or with
 * no pages at all) opens its entry in Settings › Apps instead.
 */
export function Apps() {
  const inventory = useExtensions();
  const active = useActiveContext();
  const plugins = inventory.data?.plugins ?? [];

  return (
    <section className="home-side-section" aria-labelledby="home-apps-title">
      <h2 id="home-apps-title" className="home-section-heading">
        Apps {plugins.length > 0 && <span className="text-muted">{plugins.length}</span>}
      </h2>
      {inventory.status === "loading" ? (
        <LoadingState label="Reading your apps…" />
      ) : inventory.status === "error" ? (
        <FailureAlert title="Could not read your apps" error={inventory.error} className="home-section-alert" />
      ) : plugins.length === 0 ? (
        <p className="home-note">No apps installed</p>
      ) : (
        <ul className="home-side-group home-pick-list" aria-label="Installed apps">
          {plugins.map((plugin) => {
            const name = extensionLabel(plugin);
            const { word, tone } = state(plugin);
            const page = plugin.manifest.contributions.pages[0];
            const here = active && page && plugin.enabled && !plugin.quarantined && extensionEnabledFor(plugin, active.key);
            return (
              <li key={plugin.manifest.id} className="home-live-row">
                <button
                  type="button"
                  className="home-live-go home-app-go"
                  aria-label={here ? `Open ${name} on ${active.name}` : `${name} in Settings › Apps`}
                  onClick={() => (here
                    ? openTab(extensionClusterRoute(active.key, plugin.manifest.id, page.id), { clusterName: active.name })
                    : openSettings("extensions"))}
                >
                  <ExtensionLogo icon={plugin.icon} name={name} size={20} />
                  <span className="min-w-0 flex-1 truncate font-medium">{name}</span>
                </button>
                <Badge tone={tone}>{word}</Badge>
              </li>
            );
          })}
        </ul>
      )}
      <div className="home-section-foot">
        <Button variant="secondary" size="sm" onClick={() => openSettings("extensions", "catalog")}>Browse catalog</Button>
      </div>
    </section>
  );
}
