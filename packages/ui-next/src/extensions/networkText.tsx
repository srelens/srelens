import { NETWORK_HTTP, networkHosts } from "@srelens/core";
import { plainText } from "./displayText";

/** A `${settings.<id>}` reference's setting id, when `value` is one. */
export const settingReference = (value: unknown) =>
  typeof value === "string" ? /^\$\{settings\.([A-Za-z0-9-]{1,64})\}$/.exec(value)?.[1] : undefined;

/** A declared setting's title as plain text; its id when the manifest declares no title. */
export function settingTitle(manifest: unknown, id: string) {
  const settings = (manifest as { settings?: unknown } | null)?.settings;
  const setting = (Array.isArray(settings) ? settings : []).find(
    (entry) => (entry as { id?: unknown } | null)?.id === id,
  ) as { title?: unknown } | undefined;
  return plainText(typeof setting?.title === "string" ? setting.title : id);
}

/**
 * One `network.http` host (#568) as a person reads it: the setting a URL is saved in,
 * a wildcard's reach, or the host as written.
 */
export function hostText(manifest: unknown, host: string) {
  const id = settingReference(host);
  if (id) return `The host of the URL saved in ${settingTitle(manifest, id)}`;
  return host.startsWith("*.") ? `${plainText(host)} (one subdomain label)` : plainText(host);
}

/**
 * The same, drawn: a literal host as machine text, a setting's host as the sentence
 * that says where it comes from.
 */
export function HostText({ manifest, host }: { manifest: unknown; host: string }) {
  const id = settingReference(host);
  if (id) return <>{hostText(manifest, host)}</>;
  return (
    <>
      <code>{plainText(host)}</code>
      {host.startsWith("*.") && " (one subdomain label)"}
    </>
  );
}

/** `value` as JSON with every object's keys sorted, so equal values compare equal. */
const canonical = (value: unknown): string =>
  Array.isArray(value)
    ? `[${value.map(canonical).join(",")}]`
    : value && typeof value === "object"
      ? `{${Object.keys(value)
          .sort()
          .map((key) => `${JSON.stringify(key)}:${canonical((value as Record<string, unknown>)[key])}`)
          .join(",")}}`
      : JSON.stringify(value) ?? "null";

/**
 * What a declared setting can put in a request: its declaration without the words it is
 * drawn with, as the host's access review compares it. `undefined` when undeclared.
 */
function declaration(manifest: unknown, id: string): unknown {
  const settings = (manifest as { settings?: unknown } | null)?.settings;
  const setting = (Array.isArray(settings) ? settings : []).find(
    (entry) => (entry as { id?: unknown } | null)?.id === id,
  ) as Record<string, unknown> | undefined;
  if (!setting) return undefined;
  const { title: _title, description: _description, options, ...rest } = setting;
  return Array.isArray(options)
    ? { ...rest, options: options.map((option) => (option as { value?: unknown } | null)?.value) }
    : rest;
}

/** Every setting a value refers to, anywhere inside it, with its declaration. */
function referencedSettings(manifest: unknown, value: unknown): Record<string, unknown> {
  const found: Record<string, unknown> = {};
  const walk = (node: unknown) => {
    const id = settingReference(node);
    if (id) found[id] = declaration(manifest, id);
    else if (Array.isArray(node)) node.forEach(walk);
    else if (node && typeof node === "object") Object.values(node).forEach(walk);
  };
  walk(value);
  return found;
}

/**
 * Where a manifest's `network.http` may reach (#568), as comparable text: each host
 * entry, and for a `${settings.<id>}` one the setting's declaration. Two versions can list
 * the same entry while its default points somewhere else, and with no saved value the
 * default is where requests go.
 */
export function networkReach(manifest: unknown): string {
  return canonical(
    networkHosts(manifest)
      .map((host) => ({ host, settings: referencedSettings(manifest, host) }))
      .sort((a, b) => a.host.localeCompare(b.host)),
  );
}

/**
 * A manifest's `network.http` requests (#568) as comparable text: each binding's name,
 * arguments, and the declaration of every setting they interpolate. Two versions with the
 * same grants and hosts can still send different requests — another path, a secret in
 * another header, or a URL setting with another default — and that is what this tells.
 */
export function networkRequests(manifest: unknown): string {
  const capabilities = (manifest as { capabilities?: unknown } | null)?.capabilities;
  const bindings = (Array.isArray(capabilities) ? capabilities : [])
    .map((entry) => entry as { name?: unknown; target?: unknown; arguments?: unknown })
    .filter((binding) => binding.target === NETWORK_HTTP)
    .map((binding) => ({
      name: binding.name,
      arguments: binding.arguments,
      settings: referencedSettings(manifest, binding.arguments),
    }))
    .sort((a, b) => String(a.name).localeCompare(String(b.name)));
  return canonical(bindings);
}

/** A host for a rollback's review: `hostText`, with a setting host's default beside it. */
export function reachText(manifest: unknown, host: string) {
  const id = settingReference(host);
  const fallback = id ? (declaration(manifest, id) as { default?: unknown } | undefined)?.default : undefined;
  return typeof fallback === "string"
    ? `${hostText(manifest, host)} (default ${plainText(fallback)})`
    : hostText(manifest, host);
}
