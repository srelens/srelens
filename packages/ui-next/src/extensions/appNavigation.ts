import { extensionClusterRoute, extensionOperationRoute, extensionEnabledFor, type CrdRef, type InstalledExtension } from "@srelens/core";
import type { ResourceNode } from "@srelens/ui-kit";
import { extensionLogoIcon, extensionPageIcon } from "./ExtensionLogo";
import { pageReaders, readersServed } from "./requirements";

/**
 * The sidebar's entries for one cluster's apps (#1001): one per enabled app
 * that is on for it and contributes a page, standing beside the built-in rows
 * rather than under an "Apps" heading. An app with one place to go is a row
 * named after the app; one with more is a group of its pages, pages that share
 * a `group` nested under it in their first page's place. Empty when no app
 * contributes a page here. `crds` is what the cluster serves: a page whose
 * readers need a CRD it lacks is left out, and an app left with nothing to open
 * is not listed. Undefined means unknown, which leaves nothing out (#1024).
 *
 * `contextKey` is both what each page's route carries and what an app's cluster
 * scope names: the one identity no two contexts share (#623, #695). Pure, so
 * the budget test (#581) times exactly what the sidebar builds.
 */
export function appNavigation(plugins: InstalledExtension[], contextKey: string, crds?: CrdRef[]): ResourceNode[] {
  return plugins.filter((p) => p.enabled && extensionEnabledFor(p, contextKey)).flatMap((p) => {
    const icon = extensionLogoIcon(p.manifest.name, p.icon);
    // A page reading a CRD this cluster doesn't serve would open on "Missing requirements" (#1024).
    const served = p.manifest.contributions.pages.filter((page) => readersServed(p, pageReaders(p, page), crds));
    const children: ResourceNode[] = [...served.flatMap((page, index, pages) => {
      const leaf = (item: typeof page) => ({
        id: `route:${extensionClusterRoute(contextKey, p.manifest.id, item.id)}`,
        label: item.title, icon: extensionPageIcon(item.title),
      });
      if (!page.group) return [leaf(page)];
      if (pages.findIndex((item) => item.group === page.group) !== index) return [];
      return [{ id: `extension:${p.manifest.id}:${page.group}`, label: page.group, icon: extensionPageIcon(page.group),
        children: pages.filter((item) => item.group === page.group).map(leaf) }];
    }), ...(p.manifest.sidecar?.operations ?? []).filter((operation) => !operation.view?.hidden).map((operation) => ({
      id: `route:${extensionOperationRoute(contextKey, p.manifest.id, p.revision, operation.name)}`,
      label: operation.title, icon: extensionPageIcon(operation.title),
    }))];
    if (!children.length) return [];
    // One destination: the app is the row, not a heading over one row.
    const leaves = children.flatMap((child) => child.children ?? [child]);
    if (leaves.length === 1) return [{ id: leaves[0].id, label: p.manifest.name, icon }];
    return [{ id: `extension:${p.manifest.id}`, label: p.manifest.name, icon, children }];
  });
}
