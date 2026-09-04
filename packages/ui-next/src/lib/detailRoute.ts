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
 * `/edit/<kind>/<namespace>/<name>` — the resource EDITOR route, the detail
 * route's shape under a different prefix.
 *
 * It is here, beside `detailRoute`, and not written a third time, because the
 * argument for the shape is the same argument word for word. `Edit` used to
 * mint `/edit/<name>`: the name alone, while every other route the row menu
 * mints carries all three segments. `openTab` dedupes by route STRING, so the
 * route IS a resource's identity — and with only a name in it, `Edit` on
 * `default/api` and on `staging/api` collapsed onto ONE tab titled "Edit api",
 * and the second click focused the first resource's editor. A Pod `api` and a
 * Deployment `api` collapsed the same way. That is the dead end #346 and #349
 * closed for `logs`, `shell` and `forward`; `Edit` was still minting one.
 *
 * No editor screen is registered for it yet, so the tab renders the
 * Placeholder — which is what `/incidents`, `/agent` and `/topology` do, and is
 * this design's own treatment of a screen the migration has not reached.
 * `handoffFor` in `apps/desktop/src/design.ts` carries `{context, kind}` and no
 * name, so its "Open in classic" lands on the cluster overview exactly as it
 * does for a DETAIL route today. Reported rather than changed: that file is
 * outside this task's reach.
 */
export function editRoute(
  kind: string,
  namespace: string | null,
  name: string,
  on: {
    /** The cluster the resource is on — part of the editor's identity, see {@link EditRouteParts}. */
    cluster: string;
    /** The API group, for a custom kind only — see {@link EditRouteParts.group}. */
    group?: string;
  },
): string {
  const ns = namespace === null ? CLUSTER_SCOPED_SEGMENT : encodeURIComponent(namespace);
  const qualified = on.group ? `${on.group}/${kind}` : kind;
  return `/edit/${encodeURIComponent(on.cluster)}/${encodeURIComponent(qualified)}/${ns}/${encodeURIComponent(name)}`;
}

/**
 * What an edit route names, over and above a detail route.
 *
 * **The cluster is in the route** because the editor PINS the cluster it was
 * opened on — every read and write goes there, whatever the rail does later —
 * and `openTab` dedupes by route string. With the cluster left out, Edit on
 * staging's `default/web` while prod's editor for `default/web` was open
 * focused prod's draft, and staging's resource could not be opened at all
 * without closing that tab first. The detail route has no cluster in it
 * because the detail pane follows the rail; the editor deliberately does not.
 *
 * **The group is in the route for a custom kind** because a kind NAME alone
 * does not identify one: a CRD may legally reuse a built-in kind's name in its
 * own group — `Deployment` in `acme.io` — and an editor that inferred
 * "built-in" from the name would read, and offer to delete, the built-in
 * object of the same name instead. It rides in the kind segment as
 * `<group>/<kind>`, kubectl's own qualified spelling; the segment is
 * percent-encoded, so the `/` never changes the route's arity.
 */
export interface EditRouteParts extends DetailRouteParts {
  /** Absent only on the five-segment shape an earlier build persisted, which
   *  had no cluster in it; the editor then pins whatever the rail is on. */
  cluster?: string;
  /** Set only for a custom kind. A built-in kind's group is known to the backend. */
  group?: string;
}

/**
 * The one parser both shapes share: five segments after splitting on `/`, the
 * first empty, the second the prefix's own word, and every one of the last
 * three decoded or the whole route refused.
 *
 * COUNTING segments rather than pattern-matching, for the reason `detailRoute`
 * gives above: a decoded kind or name can contain a `/`, so only the count
 * tells the shape. That is also what refuses the LEGACY `/edit/<name>` — three
 * segments — so a tab a previous session persisted is named by `describe`'s
 * fallback branch rather than mistaken for a subject with `<name>` as its kind.
 */
function parseResourceRoute(route: string, prefix: string): DetailRouteParts | null {
  const segments = route.split("/");
  if (segments.length !== 5) return null;
  const [empty, head, rawKind, rawNamespace, rawName] = segments;
  if (empty !== "" || head !== prefix) return null;
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
    // these parsers also run during render over persisted routes, so a throw
    // is the whole window failing to boot rather than one bad tab. `null` is
    // the existing answer for a route that cannot be made a subject of.
    return null;
  }
}

/**
 * The inverse of {@link editRoute}, or `null` for anything it cannot make a
 * subject of — including the legacy one-segment `/edit/<name>`.
 *
 * Six segments is the shape `editRoute` mints; five is the shape before the
 * cluster joined it, still parsed so a tab an earlier build persisted opens
 * an editor rather than the Placeholder.
 */
export function parseEditRoute(route: string): EditRouteParts | null {
  const segments = route.split("/");
  if (segments.length === 5) {
    const parts = parseResourceRoute(route, "edit");
    return parts ? splitGroup(parts) : null;
  }
  if (segments.length !== 6) return null;
  const [empty, head, rawCluster, ...rest] = segments;
  if (empty !== "" || head !== "edit" || !rawCluster) return null;
  const parts = parseResourceRoute(`/edit/${rest.join("/")}`, "edit");
  if (!parts) return null;
  let cluster: string;
  try {
    cluster = decodeURIComponent(rawCluster);
  } catch {
    return null;
  }
  const qualified = splitGroup(parts);
  return qualified ? { ...qualified, cluster } : null;
}

/** `<group>/<kind>` in the kind segment becomes `group` and `kind`; a bare kind is left alone. */
function splitGroup(parts: DetailRouteParts): EditRouteParts | null {
  const at = parts.kind.lastIndexOf("/");
  if (at < 0) return parts;
  const group = parts.kind.slice(0, at);
  const kind = parts.kind.slice(at + 1);
  if (!group || !kind) return null;
  return { ...parts, group, kind };
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
  return parseResourceRoute(route, "k");
}
