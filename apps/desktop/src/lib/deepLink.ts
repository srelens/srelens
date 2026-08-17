// `srelens://` deep-link parsing (#36).
//
// Two routes, both taking a kube context as their first segment:
//   srelens://cluster/<context>
//   srelens://resource/<context>/<namespace>/<kind>/<name>
//
// Parsing is kept pure and separate from navigation so the URL grammar can be
// tested exhaustively — a deep link is attacker-reachable in the sense that
// any web page can ask the OS to open one, so it is validated rather than
// trusted.

/** A parsed, validated deep-link destination. */
export type DeepLinkTarget =
  | { route: "cluster"; context: string }
  | {
      route: "resource";
      context: string;
      /** Null for cluster-scoped kinds, or when the link omits a namespace. */
      namespace: string | null;
      /** Canonical Kubernetes kind, e.g. "Pod" — resolved by the caller. */
      kind: string;
      name: string;
    };

/** A single path segment is rejected outright if it carries control
 *  characters; they cannot appear in a legitimate Kubernetes name or context
 *  and would otherwise reach logs and the UI verbatim. */
function isCleanSegment(value: string): boolean {
  return value.length > 0 && ![...value].some((c) => c.codePointAt(0)! < 0x20 || c.codePointAt(0)! === 0x7f);
}

/**
 * Split an `srelens://` URL into decoded path segments, or null when it is not
 * an srelens link at all.
 *
 * Tolerates the extra slashes different platforms produce (`srelens://x` and
 * `srelens:///x` both occur, since a link with no authority component is
 * normalized differently by each OS handler). Each segment is
 * percent-decoded AFTER splitting, so an encoded `/` inside a context name —
 * OpenShift contexts look like `default/api-example-com:6443/user` — stays a
 * single segment instead of splitting the route apart.
 */
function segmentsOf(url: string): string[] | null {
  const match = /^srelens:\/*(.*)$/i.exec(url.trim());
  if (!match) return null;
  const [path] = match[1].split(/[?#]/, 1);
  const raw = path.split("/").filter((segment) => segment.length > 0);
  try {
    return raw.map(decodeURIComponent);
  } catch {
    // Malformed percent-encoding — refuse rather than guess.
    return null;
  }
}

/**
 * Parse a deep link, or return null when it is not a link we serve. Unknown
 * routes and wrong segment counts are refused rather than partially honoured,
 * so a malformed link is inert instead of navigating somewhere unintended.
 */
export function parseDeepLink(url: string): DeepLinkTarget | null {
  const segments = segmentsOf(url);
  if (!segments || segments.length === 0) return null;
  if (!segments.every(isCleanSegment)) return null;

  const [route, ...rest] = segments;

  if (route === "cluster") {
    if (rest.length !== 1) return null;
    return { route: "cluster", context: rest[0] };
  }

  if (route === "resource") {
    if (rest.length !== 4) return null;
    const [context, namespace, kind, name] = rest;
    return {
      route: "resource",
      context,
      // "-" is the conventional placeholder for a cluster-scoped resource,
      // since an empty path segment would collapse when the URL is split.
      namespace: namespace === "-" ? null : namespace,
      kind,
      name,
    };
  }

  return null;
}

/**
 * Collapse links that would open the SAME view, keeping the last of each.
 *
 * A batch is routed against one render's `tabs`, so two links to the same
 * cluster+kind would each find no existing tab and append their own —
 * duplicate tabs and duplicate resource watches from one double-click. The
 * last link wins because it is the one whose focus should end up in front.
 */
export function dedupeDeepLinkTargets(targets: DeepLinkTarget[]): DeepLinkTarget[] {
  const byView = new Map<string, DeepLinkTarget>();
  for (const target of targets) {
    const key =
      target.route === "cluster"
        ? `cluster:${target.context}`
        : // Keyed by the VIEW, not the resource: two pods in one cluster share
          // a single Pods tab.
          `resource:${target.context}:${target.kind}`;
    // Delete before re-inserting: Map.set keeps a replaced key at its ORIGINAL
    // position, which would emit an earlier link after a later one. Routing
    // order is what decides which tab ends up active, so the batch has to come
    // out in last-occurrence order for "the last link wins" to hold.
    byView.delete(key);
    byView.set(key, target);
  }
  return [...byView.values()];
}
