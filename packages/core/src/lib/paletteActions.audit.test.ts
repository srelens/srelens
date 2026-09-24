import { describe, it, expect } from "vitest";
import catalog from "./capability-catalog.json";
import flux from "../../../../examples/extensions/flux.json";
import argocd from "../../../../examples/extensions/argocd.json";
import type { ExtensionManifest } from "./extensions";
import { appPaletteCommands, paletteActionCapabilityIds } from "./paletteActions";

// Capabilities intentionally NOT palette actions, each with a reason. Keep this
// list small and justified — the whole point is that new mutating capabilities
// fail CI until they are either palette-registered or excluded here.
const EXCLUDED: Record<string, string> = {
  "k8s.annotate": "a host action primitive: reached only through an app's declared action, on a reviewed resource",
  "k8s.setFields": "a host action primitive: reached only through an app's declared action, on a reviewed resource",
  "k8s.setStatusCondition": "a host action primitive: reached only through an app's declared action, on a reviewed resource",
  "k8s.requestRolloutRestart": "reviewed app action adapter; the palette uses k8s.rolloutRestart without a reviewed UID/resourceVersion",
  "k8s.requestCordonNode": "reviewed app action adapter; the palette uses k8s.cordonNode without a reviewed UID/resourceVersion",
  "k8s.mergePatch": "a host action primitive: reached only through an app's declared action, on a reviewed resource",
  "k8s.nodeServiceRestart": "node service remediation is surfaced in Node view / incident actions and requires an SSH target",
  "extensions.action": "not a static entry: reached only through an app's contributed action command (see below), which opens the host confirmation in the app resource inspector",
  "extensions.configure": "extension lifecycle and permissions are managed in Settings → Apps",
  "k8s.applyManifest": "reached via the manifest editor, not a one-click palette action",
  "k8s.updateConfigData": "ConfigMap/Secret data edits happen in the ResourceOverview drawer, which needs a key/value patch beyond a bare resource ref",
  "k8s.deleteContext": "kubeconfig management lives in Settings",
  "k8s.helmInstall": "surfaced via HelmOpDialog from HelmReleasesView, not a resource-targeted palette action",
  "k8s.helmUpgrade": "surfaced via HelmOpDialog from HelmReleasesView, not a resource-targeted palette action",
  "k8s.helmRollback": "surfaced via HelmOpDialog from HelmReleasesView, not a resource-targeted palette action",
  "k8s.helmUninstall": "surfaced via HelmOpDialog from HelmReleasesView, not a resource-targeted palette action",
  "k8s.helmRepoAdd": "surfaced via the repo form in HelmReleasesView, not a resource-targeted palette action",
  "k8s.helmRepoUpdate": "surfaced via the repo form in HelmReleasesView, not a resource-targeted palette action",
  "toolbox.installKubectl": "toolbox installs live in the Toolbox view",
  "toolbox.installHelm": "toolbox installs live in the Toolbox view",
  "toolbox.installKrew": "toolbox installs live in the Toolbox view",
  "toolbox.installPlugin": "toolbox installs live in the Toolbox view",
  "toolbox.upgradePlugin": "toolbox installs live in the Toolbox view",
  "toolbox.removePlugin": "toolbox installs live in the Toolbox view",
  "settings.set": "settings writes are performed by controls in Settings and other stateful views",
};

describe("command palette action coverage", () => {
  it("registers every non-read-only capability (or explicitly excludes it)", () => {
    const registered = paletteActionCapabilityIds();
    const missing = (catalog as Array<{ id: string; readOnly: boolean }>)
      .filter((c) => !c.readOnly)
      .map((c) => c.id)
      .filter((id) => !registered.has(id) && !(id in EXCLUDED));
    expect(missing).toEqual([]);
  });
  it("has no stale EXCLUDED entries", () => {
    const ids = new Set((catalog as Array<{ id: string }>).map((c) => c.id));
    expect(Object.keys(EXCLUDED).filter((id) => !ids.has(id))).toEqual([]);
  });
});

// Contributed commands (#544) are not static entries: they come from installed
// manifests. What must hold for every one of them is checked here, against the
// shipped examples, so a command can never be a door around the host.
describe("app-contributed palette commands", () => {
  const manifests = [flux, argocd] as unknown as ExtensionManifest[];
  const byId = new Map((catalog as Array<{ id: string; readOnly: boolean; requiresConfirm: boolean }>).map((c) => [c.id, c]));

  it("prefixes every command with the app's name", () => {
    for (const manifest of manifests) {
      const commands = appPaletteCommands({ id: manifest.id, name: manifest.name, manifest }, null);
      expect(commands.length).toBeGreaterThan(0);
      for (const command of commands) expect(command.label.startsWith(`${manifest.name}: `)).toBe(true);
    }
  });

  it("opens only pages the app declares", () => {
    for (const manifest of manifests) {
      const pages = new Set(manifest.contributions.pages.map((p) => p.id));
      for (const command of appPaletteCommands({ id: manifest.id, name: manifest.name, manifest }, null)) {
        expect(command.target.kind).toBe("page");
        if (command.target.kind === "page") expect(pages.has(command.target.page)).toBe(true);
      }
    }
  });

  it("runs every declared action command only through extensions.action, which the host confirms", () => {
    const action = byId.get("extensions.action");
    expect(action).toMatchObject({ readOnly: false, requiresConfirm: true });
    let seen = 0;
    for (const manifest of manifests) {
      for (const page of manifest.contributions.pages) {
        const commands = appPaletteCommands({ id: manifest.id, name: manifest.name, manifest }, { capability: page.capability });
        for (const command of commands) {
          if (command.target.kind !== "action") continue;
          seen++;
          expect(command.capabilityId).toBe("extensions.action");
          const declared = manifest.actions?.find((a) => a.name === (command.target as { action: string }).action);
          expect(declared?.resource).toBe(page.capability);
        }
      }
      // Every action command the manifest contributes is reachable from some page.
      const declared = (manifest.contributions.commands ?? []).filter((c) => "action" in c.target);
      const reachable = new Set(manifest.contributions.pages.flatMap((page) =>
        appPaletteCommands({ id: manifest.id, name: manifest.name, manifest }, { capability: page.capability })
          .filter((c) => c.target.kind === "action").map((c) => c.id)));
      expect([...reachable].sort()).toEqual(declared.map((c) => `${manifest.id}/${c.id}`).sort());
    }
    expect(seen).toBeGreaterThan(0);
  });
});
