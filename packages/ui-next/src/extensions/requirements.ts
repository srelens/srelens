import type { CrdRef, ExtensionPage, InstalledExtension } from "@srelens/core";

/** A CRD one of an app's custom-resource readers reads. */
export interface CrdRequirement { group: string; versions: string[]; plural: string; kind: string; namespaced: boolean }

/** What a page reads: its own reader, and those of the pages its dashboard draws. */
export function pageReaders(plugin: InstalledExtension, page: ExtensionPage): Set<string> {
  return new Set([page.capability, ...(page.dashboard?.pages ?? []).flatMap(id =>
    plugin.manifest.contributions.pages.filter(p => p.id === id).map(p => p.capability))]);
}

/**
 * The CRDs `readers` read, once each. A reader accepts its `versions` in preference
 * order, or the one `arguments.version` it fixes (#547); the host reads the first one
 * the cluster serves, and nothing else.
 */
export function crdRequirements(plugin: InstalledExtension, readers: Set<string>): CrdRequirement[] {
  return plugin.manifest.capabilities.filter(b => readers.has(b.name) && b.target === "k8s.listCustomResource")
    .map(b => ({ group: String(b.arguments.group), versions: b.versions?.length ? b.versions : [String(b.arguments.version)], plural: String(b.arguments.plural), kind: String(b.arguments.kind), namespaced: b.arguments.namespaced === true }))
    .filter((r, i, rows) => rows.findIndex(other => other.group === r.group && other.versions.join() === r.versions.join() && other.plural === r.plural && other.namespaced === r.namespaced) === i);
}

/** How `crds` serves one requirement, and the listed version the host would read there. */
export function requirementStatus(r: CrdRequirement, crds: CrdRef[]) {
  const crd = crds.find(c => c.group === r.group && c.plural === r.plural && c.kind === r.kind);
  const served = crd ? (crd.versions ?? [crd.version]) : [];
  const resolved = r.versions.find(version => served.includes(version));
  const status = !crd ? "Missing CRD" : !resolved ? "Required version unavailable" : crd.namespaced !== r.namespaced ? "Scope mismatch" : "Available";
  return { resolved, status };
}

/**
 * False only when `crds` is known and lacks something `readers` need (#1024). Unknown
 * — discovery failed, which ordinary RBAC often makes it — is not missing.
 */
export function readersServed(plugin: InstalledExtension, readers: Set<string>, crds?: CrdRef[]): boolean {
  return !crds || crdRequirements(plugin, readers).every(r => requirementStatus(r, crds).status === "Available");
}
