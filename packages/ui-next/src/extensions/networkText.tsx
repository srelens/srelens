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
