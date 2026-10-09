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

import { describeError } from "./errors";
import { isClusterScopedKind, isNavigableResourceKind } from "./resourceNavigation";

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
  // Two steps, not one pattern: `\/*` and `(.*)` both matched a slash, so a URL
  // of nothing but slashes gave the engine a quadratic number of ways to split
  // them — 2.5s at 40KB (js/polynomial-redos, #48). A deep link arrives from
  // the OS, so its length is not ours to trust.
  const trimmed = url.trim();
  if (!/^srelens:/i.test(trimmed)) return null;
  const afterScheme = trimmed.slice("srelens:".length).replace(/^\/+/, "");
  const [path] = afterScheme.split(/[?#]/, 1);
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

/** The title on a refused link's notice, in both designs. */
export const DEEP_LINK_REFUSED = "Couldn't open that link";

/** The title on a held link's notice, in both designs. */
const DEEP_LINK_HELD = "That link will be checked once the contexts load";

/**
 * A link that can be opened; the sentence that says why it cannot; or `held`,
 * for a link whose context a failed listing did not return and so cannot yet
 * be judged.
 */
export type DeepLinkCheck =
  | { ok: true; target: DeepLinkTarget }
  | { ok: false; reason: string; held?: undefined }
  | { ok: false; held: true };

/**
 * Decide whether a link can be opened against the contexts this machine lists,
 * and say why not when it cannot.
 *
 * Both designs judge a link by this one rule set, so a link that one refuses
 * the other cannot open, and the reader sees the same sentence in either. It
 * takes context NAMES because a link names its context the way a kubeconfig
 * does, and is matched exactly: context names are case-sensitive.
 *
 * Call it only once the contexts have been listed. A link judged against an
 * empty list during a cold start would be refused as naming a context that
 * does not exist.
 *
 * **A listing that failed has not said a context is missing** (#855). It may
 * have refused outright, or come back partial with a reason: either way the
 * context a link names may be perfectly present behind an unreadable file or a
 * dropped connection. So with `listingFailed`, a link naming a context the list
 * lacks is `held` for the caller to judge again once the contexts reload,
 * rather than refused with a claim about the cluster that nobody has made.
 * What no listing can change is still refused at once: a link that does not
 * parse, a kind with no detail view, a namespaced kind given `-`.
 */
export function checkDeepLink(
  url: string,
  contextNames: readonly string[],
  { listingFailed = false }: { listingFailed?: boolean } = {},
): DeepLinkCheck {
  const target = parseDeepLink(url);
  if (!target) return { ok: false, reason: "It isn't a link srelens understands." };
  const listed = contextNames.includes(target.context);
  if (!listed && !listingFailed) {
    return { ok: false, reason: `No kube context named "${target.context}".` };
  }
  if (target.route === "resource") {
    // `K8S_KIND` alone is too permissive: Events have a list view but no
    // detail, so such a link would quietly land on the list instead of the
    // object it named.
    if (!isNavigableResourceKind(target.kind)) {
      return { ok: false, reason: `srelens can't open a ${target.kind} directly.` };
    }
    // "-" means cluster-scoped. Allowing it for a namespaced kind would search
    // every namespace and open whichever matching name came back first: a link
    // that silently opens the wrong object.
    if (!isClusterScopedKind(target.kind) && target.namespace === null) {
      return { ok: false, reason: `${target.kind} is namespaced, so the link needs a namespace.` };
    }
  }
  if (!listed) return { ok: false, held: true };
  return { ok: true, target };
}

/**
 * The one notice for links held behind a failed listing: what the listing
 * failed with, through `describeError` like every other failure, and that the
 * links will be checked once the contexts load. Checked, not opened: a listing
 * that answers may still lack the context, and the link is refused then.
 */
export function deepLinkHeldNotice(listingError: string): { title: string; detail: string } {
  return {
    title: DEEP_LINK_HELD,
    detail: `The kube contexts could not be listed. ${describeError(listingError).detail}`,
  };
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
