import {
  CAPABILITY_CATALOG,
  NETWORK_HTTP,
  type ExtensionPreviousVersion,
  type ExtensionSource,
  type InstalledExtension,
} from "@srelens/core";
import { POD_FACTS } from "./ExtensionBindings";

/*
 * The wording an installed app's Details use in more than one tab: the Overview and the
 * Inspector (#575) describe the same grants, source and dates, and two copies of a
 * sentence drift.
 */

const facts = new Map(CAPABILITY_CATALOG.map((capability) => [capability.id, capability]));

/** What a granted host capability can do, from the backend registry's own annotations. */
export function describeGrant(id: string): string {
  // The broker's own capability (#568): never offered to MCP or the catalog, since
  // called directly it would fetch any URL. That is not "not provided".
  if (id === NETWORK_HTTP) return "Read-only · GET requests to this app's hosts only, sent by the host";
  // Likewise the pod capabilities (#567), which run only as the app's streams.
  if (POD_FACTS[id]) return `${POD_FACTS[id]} · only pods this app's bindings may reach`;
  const fact = facts.get(id);
  if (!fact) return "Not provided by this host";
  return [
    fact.readOnly ? "Read-only" : "Changes resources",
    fact.requiresConfirm && "Asks for confirmation",
    fact.sensitive && "Sensitive",
    fact.destructive && "Destructive",
  ]
    .filter(Boolean)
    .join(" · ");
}

const from: Record<ExtensionSource, string> = { catalog: "from the Catalog", local: "local manifest" };

/**
 * Says only what the host verified. The installed version's proof is rechecked on every
 * load, and a failure quarantines the app; a kept version's is checked when it is restored.
 */
export function origin(version: InstalledExtension | ExtensionPreviousVersion) {
  const installed = "history" in version;
  const signer = !version.signatureProof
    ? "Unsigned"
    : !installed
      ? "Signed; verified when restored"
      : version.quarantined
        ? "Signature not verified"
        : "Signed by srelens";
  return `${signer} · ${from[version.source]}`;
}

export const installedOn = (seconds: number) => new Date(seconds * 1000).toLocaleString();

/** A size as a reader measures it. */
export function bytes(size: number) {
  return size < 1024 ? `${size} B` : size < 1024 * 1024 ? `${(size / 1024).toFixed(1)} KiB` : `${(size / (1024 * 1024)).toFixed(1)} MiB`;
}

/**
 * Why an installed app is switched off, as the installed list says it after "Disabled:",
 * or null when it is on. A failed signature check outranks a policy block, since removing
 * or reinstalling the app is the only way back from it.
 */
export function inactiveReason(plugin: InstalledExtension): string | null {
  if (plugin.quarantined) return `${plugin.quarantined}. Remove it or reinstall it from the Catalog.`;
  if (plugin.policyBlocked) return `${plugin.policyBlocked}.`;
  return plugin.enabled ? null : "it is not enabled.";
}
