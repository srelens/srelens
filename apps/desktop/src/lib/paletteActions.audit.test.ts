import { describe, it, expect } from "vitest";
import catalog from "./capability-catalog.json";
import { paletteActionCapabilityIds } from "./paletteActions";

// Capabilities intentionally NOT palette actions, each with a reason. Keep this
// list small and justified — the whole point is that new mutating capabilities
// fail CI until they are either palette-registered or excluded here.
const EXCLUDED: Record<string, string> = {
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
