import { extensionClusterRoute, extensionEnabledFor, type InstalledExtension } from "@srelens/core";
import type { ResourceNode } from "@srelens/ui-kit";
import { Icons } from "../lib/icons";
import { extensionLogoIcon, extensionPageIcon } from "./ExtensionLogo";

/**
 * The sidebar's "Apps" group for one cluster: every enabled app that is on for
 * it and contributes a page, each app's pages beneath it, pages that share a
 * `group` nested under it in their first page's place. `null` when no app
 * contributes a page here, so the sidebar shows no empty group.
 *
 * `contextKey` is both what each page's route carries and what an app's cluster
 * scope names: the one identity no two contexts share (#623, #695). Pure, so
 * the budget test (#581) times exactly what the sidebar builds.
 */
export function appNavigation(plugins: InstalledExtension[], contextKey: string): ResourceNode | null {
  const apps = plugins.filter((p) => p.enabled && extensionEnabledFor(p, contextKey) && p.manifest.contributions.pages.length);
  if (!apps.length) return null;
  return {
    id: "extensions", label: "Apps", icon: Icons.apps,
    children: apps.map((p) => ({
      id: `extension:${p.manifest.id}`, label: p.manifest.name, icon: extensionLogoIcon(p.manifest.id, p.manifest.name),
      children: p.manifest.contributions.pages.flatMap((page, index, pages) => {
        const leaf = (item: typeof page) => ({
          id: `route:${extensionClusterRoute(contextKey, p.manifest.id, item.id)}`,
          label: item.title, icon: extensionPageIcon(item.title),
        });
        if (!page.group) return [leaf(page)];
        if (pages.findIndex((item) => item.group === page.group) !== index) return [];
        return [{ id: `extension:${p.manifest.id}:${page.group}`, label: page.group, icon: extensionPageIcon(page.group),
          children: pages.filter((item) => item.group === page.group).map(leaf) }];
      }),
    })),
  };
}
