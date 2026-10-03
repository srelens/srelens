/**
 * Where an Ingress rule sends a reader, as something they can open or copy.
 *
 * **These addresses are built from cluster data, so they are BUILT, never
 * passed through.** {@link openExternal} is only for an address srelens put
 * together itself, and that is what this is: srelens picks the scheme, the
 * host has to be the DNS name the API server's own validation requires of
 * `spec.rules[].host`, the path is set through `URL` so it is encoded, and the
 * result is parsed back and compared before anyone can open it. A host that
 * is not a DNS name (userinfo, a port, a path, upper case) is shown as text
 * and never becomes an address. (#774)
 */

/** A rule's address: a URL to open and copy, or a host there is no URL for
 *  (a wildcard, or a value that is not a DNS name), to copy as it is. */
export type IngressRuleAddress = { kind: "url"; url: string } | { kind: "host"; host: string };

/**
 * A DNS-1123 subdomain: what `spec.rules[].host` must be, apart from the
 * wildcard form, before the API server admits the Ingress at all.
 */
const DNS_SUBDOMAIN = /^[a-z0-9]([-a-z0-9]*[a-z0-9])?(\.[a-z0-9]([-a-z0-9]*[a-z0-9])?)*$/;

/**
 * Characters that make a path a pattern rather than a place: regex groups,
 * classes, quantifiers, anchors and escapes, and the `*` of a glob. Not `.`,
 * which literal paths are full of.
 */
const PATTERN = /[()[\]{}*+?|^$\\]/;

/**
 * The address of one rule: `host` + `path`, over https when the Ingress lists
 * the host under `spec.tls[].hosts`.
 *
 * `null` when the rule has no host, since there is nothing to name. A wildcard
 * host comes back as itself. A path that is a pattern (by its own characters,
 * or because the controller treats every path as one) links to the host's
 * root, as #774 specifies: a pattern names no single address. The root need
 * not be routed by this rule; the Path column beside the link shows the
 * pattern as written, so the reader can tell the two apart.
 */
export function ingressRuleAddress(
  host: string,
  path: string,
  ingress: { tlsHosts: readonly string[]; regexPaths: boolean },
): IngressRuleAddress | null {
  if (!host) return null;
  if (host.length > 253 || !DNS_SUBDOMAIN.test(host)) return { kind: "host", host };
  const scheme = ingress.tlsHosts.some((tls) => tlsCovers(tls, host)) ? "https" : "http";
  const url = new URL(`${scheme}://${host}`);
  if (!ingress.regexPaths && path.startsWith("/") && !PATTERN.test(path)) url.pathname = path;
  // Belt and braces: what comes back out of `URL` must be the host that went
  // in, with nothing added around it.
  if (url.hostname !== host || url.port || url.username || url.password) return { kind: "host", host };
  return { kind: "url", url: url.href };
}

/** Whether a `spec.tls[].hosts` entry covers `host`: an exact match, or a
 *  `*.` wildcard standing for exactly one label. */
function tlsCovers(tls: string, host: string): boolean {
  if (tls === host) return true;
  if (!tls.startsWith("*.")) return false;
  const dot = host.indexOf(".");
  return dot > 0 && host.slice(dot + 1) === tls.slice(2);
}

/**
 * Whether ingress-nginx matches this Ingress's paths as regular expressions.
 *
 * `use-regex` says so outright. A `rewrite-target` does too: ingress-nginx
 * applies the regex location modifier to every path on the host once one is
 * set, which is what its capture groups (`/$2`) depend on.
 */
export function ingressUsesRegexPaths(annotations: Record<string, unknown>): boolean {
  return (
    annotations["nginx.ingress.kubernetes.io/use-regex"] === "true" ||
    typeof annotations["nginx.ingress.kubernetes.io/rewrite-target"] === "string"
  );
}
