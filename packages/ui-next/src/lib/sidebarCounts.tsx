import { useEffect, useMemo, useRef, useSyncExternalStore, type ReactNode } from "react";
import {
  countResources,
  filtersKind,
  getForwards,
  isForwardEnded,
  K8S_KIND,
  selectsNothing,
  subscribeForwards,
  type ClusterContext,
  type CountKindRef,
  type CrdRef,
  type KindCountResult,
} from "@srelens/core";
import { CACHE_TTL_MS, useCachedResource } from "./cachedResource";
import { useCollectionScope } from "./collectionScope";
import { descriptorFor } from "./kinds/descriptors";
import { groupNumber } from "./numbers";
import { CLUSTER_KINDS, NAV_GROUPS, type NodeCounts } from "./tree";
import { useNamespaceCollections, useNamespaces } from "./workspace";

export interface SidebarCountArgs {
  /** The cluster the sidebar is showing, or `null` for none. */
  ctx: ClusterContext | null;
  paused: boolean;
  /** The sidebar's open folds, by node id. */
  expanded: readonly string[];
  /** The cluster's discovered CRDs. */
  crds: CrdRef[];
}

interface Asked {
  id: string;
  ref: CountKindRef;
}

const clock = (at: number) => new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

/**
 * The number at the end of each sidebar row (#402), by node id, for
 * `kindNodes` / `crdNodes` to put on their nodes.
 *
 * **What a number means** is the list header's rule: how many of that kind
 * are in the namespaces the active tab has selected (`useNamespaces` read
 * outside any tab is the active tab's selection), or in the cluster for a
 * cluster-scoped kind. Switching to a tab with another selection moves them.
 *
 * **What is counted** is only what is in sight: the cluster band's kinds and
 * Port forwards, which are rows of their own and always showing, the kinds
 * under an open fold, and a CRD only when both "Custom resources" and its API
 * group are open. A paused cluster asks nothing at all.
 *
 * **What is never drawn is `0` for something not known.** A kind the backend
 * could not count gets no entry. A selection of nothing is `0` without asking
 * — but only once it is an answer: while the tab's collection is still
 * waiting for a listing, resolving or failed, `useNamespaces` hands back the
 * same select-nothing marker, and a zero then would be a guess. A kind that a
 * collection in force narrows by name is left out too: the count cannot apply
 * a name expression, and a number that ignored it would disagree with the
 * list's own header. A built-in row does not know its group, and a collection
 * naming `acme.io/Deployment` does not narrow the built-in Deployment, so a
 * built-in is counted and then left out only if the group the backend counted
 * it in is the one the collections name.
 *
 * **Freshness.** Counted on demand, then again every {@link CACHE_TTL_MS}
 * while the page is visible, and on coming back to it if the last count is
 * older than that. A refresh that fails keeps the last numbers, marked with a
 * dotted underline, with when they were counted — `useCachedResource`'s `stale`.
 *
 * ponytail: an open kind's live rows.length could replace its 30 s-old count; one source until that matters
 */
export function useSidebarCounts({ ctx, paused, expanded, crds }: SidebarCountArgs): NodeCounts {
  const clusterId = ctx?.stableId;
  const selection = useNamespaces(clusterId);
  const names = useNamespaceCollections(clusterId);
  const scope = useCollectionScope(clusterId, names);
  const forwards = useSyncExternalStore(subscribeForwards, getForwards, getForwards);
  const folds = expanded.join("\u0000");
  const live = ctx !== null && !paused;

  const { zero, ask, namespaces, narrows } = useMemo(() => {
    const open = new Set(folds.split("\u0000"));
    const inForce = scope && scope.status !== "gone" ? scope.collections : [];
    const nothing = selectsNothing(selection);
    const settled = !scope || scope.status === "ready" || scope.status === "gone";
    // Every collection in force names this kind in this group. A built-in is
    // named by its bare kind too, as the backend's list reads it
    // (`collections::names_kind`), whatever group it was counted in.
    const narrows = (kind: string, group: string, builtIn: boolean) =>
      inForce.length > 0 &&
      inForce.every((c) => filtersKind(c, { kind, group }) || (builtIn && filtersKind(c, { kind, group: "" })));
    const zero: string[] = [];
    const ask: Asked[] = [];
    const add = (id: string, ref: CountKindRef) => {
      // A built-in asks without its group: checked once the answer names it.
      if (ref.group !== undefined && narrows(ref.kind, ref.group, false)) return;
      if (ref.namespaced && nothing) {
        if (settled) zero.push(id);
        return;
      }
      ask.push({ id, ref });
    };
    if (live) {
      const kinds = [...CLUSTER_KINDS, ...NAV_GROUPS.flatMap((group) => (open.has(group.id) ? group.kinds : []))];
      for (const slug of kinds) add(`kind:${slug}`, { kind: K8S_KIND[slug], namespaced: descriptorFor(slug)?.scope !== "cluster" });
      if (open.has("crds")) {
        for (const crd of crds) {
          if (!open.has(`crdgroup:${crd.group || "core"}`)) continue;
          add(`crd:${crd.name}`, { kind: crd.kind, group: crd.group, version: crd.version, plural: crd.plural, namespaced: crd.namespaced });
        }
      }
    }
    // Sorted, so the answer's order and the cache key are the same question
    // however discovery happened to order the CRDs this time.
    ask.sort((a, b) => a.id.localeCompare(b.id));
    return { zero, ask, namespaces: nothing ? [] : [...selection].sort(), narrows };
  }, [live, folds, crds, selection, scope]);

  const key = `sidebarCounts|${ctx?.name ?? ""}|${namespaces.join(",")}|${ask.map((a) => a.id).join(",")}`;
  const counted = useCachedResource<KindCountResult[]>(
    key,
    async () => {
      if (!ctx || ask.length === 0) return [];
      const out = await countResources(ctx.name, ask.map((a) => a.ref), namespaces);
      if (out.error !== undefined || !out.counts) throw new Error(out.error ?? "no counts came back");
      return out.counts;
    },
    () => false,
  );

  const { reload } = counted;
  const updatedAt = useRef<number | undefined>(undefined);
  updatedAt.current = counted.updatedAt;
  const asking = ask.length > 0;
  useEffect(() => {
    if (!asking) return;
    const tick = setInterval(() => {
      if (document.visibilityState === "visible") reload();
    }, CACHE_TTL_MS);
    const back = () => {
      if (document.visibilityState === "visible" && Date.now() - (updatedAt.current ?? 0) >= CACHE_TTL_MS) reload();
    };
    document.addEventListener("visibilitychange", back);
    return () => {
      clearInterval(tick);
      document.removeEventListener("visibilitychange", back);
    };
  }, [asking, key, reload]);

  const { data, stale } = counted;
  const at = counted.updatedAt;
  return useMemo(() => {
    const map = new Map<string, ReactNode>();
    for (const id of zero) map.set(id, groupNumber(0));
    const why = stale && at !== undefined ? `Counted at ${clock(at)}; could not refresh` : undefined;
    data?.forEach((c, i) => {
      const id = ask[i]?.id;
      if (id === undefined || c.count === null || narrows(c.kind, c.group, ask[i].ref.group === undefined)) return;
      map.set(
        id,
        why ? (
          // Marked rather than faded, which took the number below 3:1 (#1001).
          // The mark and the title reach no screen reader; the words do.
          <span className="underline decoration-dotted underline-offset-2" title={why}>
            {groupNumber(c.count)}
            <span className="sr-only">, not refreshed</span>
          </span>
        ) : (
          groupNumber(c.count)
        ),
      );
    });
    if (live && ctx) {
      map.set("route:/forwards", groupNumber(forwards.filter((f) => f.context === ctx.name && !isForwardEnded(f)).length));
    }
    return map;
  }, [zero, ask, narrows, data, stale, at, live, ctx, forwards]);
}
