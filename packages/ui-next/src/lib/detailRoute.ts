/**
 * `/k/<kind>/<namespace>/<name>` — the resource detail route.
 *
 * `openTab` dedupes by route string, so the route IS a resource's identity: a
 * Pod named `web` and a ConfigMap named `web` must not collapse onto one tab.
 * Four segments always, with `CLUSTER_SCOPED_SEGMENT` standing in for a
 * cluster-scoped kind's namespace, so the arity never varies and parsing can
 * count segments rather than pattern-match one.
 *
 * Every segment is `encodeURIComponent`-ed on the way in and decoded on the
 * way out: a CRD's kind and a resource's name can both contain a `/` in the
 * wild, which would otherwise change how many segments the route splits into.
 */

/** Stands in for a cluster-scoped kind's namespace, which does not exist. */
export const CLUSTER_SCOPED_SEGMENT = "-";

export function detailRoute(kind: string, namespace: string | null, name: string): string {
  const ns = namespace === null ? CLUSTER_SCOPED_SEGMENT : encodeURIComponent(namespace);
  return `/k/${encodeURIComponent(kind)}/${ns}/${encodeURIComponent(name)}`;
}

export interface DetailRouteParts {
  kind: string;
  namespace: string | null;
  name: string;
}

/**
 * The inverse of `detailRoute`, or `null` for anything it cannot make a
 * subject of — a route of the wrong arity, and a route whose segments will not
 * decode. Parsed by counting segments after splitting
 * on `/`, not by pattern-matching — a decoded name can contain anything,
 * including characters that would otherwise look like part of the route's
 * shape.
 *
 * A `/k/<slug>` LIST route (three segments once split) and a `/k/<kind>/<ns>/<name>`
 * DETAIL route (five) share the `/k/` prefix; this refuses anything that
 * isn't exactly five, so `screenFor` can tell the two apart.
 */
export function parseDetailRoute(route: string): DetailRouteParts | null {
  const segments = route.split("/");
  if (segments.length !== 5) return null;
  const [empty, k, rawKind, rawNamespace, rawName] = segments;
  if (empty !== "" || k !== "k") return null;
  if (!rawKind || !rawNamespace || !rawName) return null;
  try {
    return {
      kind: decodeURIComponent(rawKind),
      namespace: rawNamespace === CLUSTER_SCOPED_SEGMENT ? null : decodeURIComponent(rawNamespace),
      name: decodeURIComponent(rawName),
    };
  } catch {
    // A malformed escape (`%zz`, a truncated multi-byte sequence) makes
    // `decodeURIComponent` THROW. `parseLogsRoute` in `screens/Logs.tsx`
    // explains why that matters at length, and every word of it applies here:
    // this parser also runs during render over persisted routes, so a throw
    // is the whole window failing to boot rather than one bad tab. `null` is
    // this function's existing answer for a route it cannot make a subject of.
    return null;
  }
}
