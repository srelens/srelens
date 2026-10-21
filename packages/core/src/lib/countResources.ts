import { invokeCapability, type Invoker } from "../transport/transport";

/** One kind to count, addressed the way `k8s.listResource` takes it. */
export interface CountKindRef {
  kind: string;
  group?: string;
  version?: string;
  plural?: string;
  /**
   * With `group`, `version` and `plural`, addresses a custom resource without
   * the backend asking the cluster's discovery. Also tells this wrapper which
   * kinds the namespace selection applies to.
   */
  namespaced?: boolean;
}

export interface KindCountResult {
  kind: string;
  group: string;
  /** `null` when this kind could not be counted — never `0` for a refusal. */
  count: number | null;
  error?: string;
}

export interface CountResourcesOutcome {
  /** In the order the kinds were asked for. */
  counts?: KindCountResult[];
  /** The whole count failed — the cluster answered nothing, or the transport did. */
  error?: string;
}

/** The backend's bounds (`crates/kube/src/count.rs`), kept here so a caller never trips them. */
export const COUNT_MAX_NAMESPACES = 128;
export const COUNT_MAX_REQUESTS = 1024;

/**
 * How many objects of each kind there are, via `k8s.countResources`: one
 * small request per kind and namespace rather than a list (#402).
 *
 * Kept inside the backend's bounds rather than letting a big ask fail whole:
 * kinds are split into calls of at most {@link COUNT_MAX_REQUESTS} requests,
 * run one after another so a large CRD group does not land on the cluster at
 * once; and a selection of more than {@link COUNT_MAX_NAMESPACES} namespaces
 * counts only the cluster-scoped kinds, the namespaced ones coming back with
 * no number. One failed call fails the outcome — a partial set of counts
 * would be drawn as current when part of it is not.
 */
export async function countResources(
  context: string,
  kinds: CountKindRef[],
  namespaces: string[],
  invoke: Invoker = invokeCapability,
): Promise<CountResourcesOutcome> {
  const tooMany = namespaces.length > COUNT_MAX_NAMESPACES;
  const asked = tooMany ? kinds.filter((k) => k.namespaced === false) : kinds;
  const scope = tooMany ? [] : namespaces;
  const perCall = Math.max(1, Math.floor(COUNT_MAX_REQUESTS / Math.max(1, scope.length)));

  const answered: KindCountResult[] = [];
  try {
    for (let i = 0; i < asked.length; i += perCall) {
      const out = await invoke<{ counts: KindCountResult[] }>("k8s.countResources", {
        context,
        kinds: asked.slice(i, i + perCall),
        namespaces: scope,
      });
      answered.push(...out.counts);
    }
  } catch (e) {
    return { error: e instanceof Error ? e.message : String(e) };
  }

  if (!tooMany) return { counts: answered };
  const byKind = answered.values();
  return {
    counts: kinds.map((k) =>
      k.namespaced === false
        ? (byKind.next().value as KindCountResult)
        : { kind: k.kind, group: k.group ?? "", count: null, error: `more than ${COUNT_MAX_NAMESPACES} namespaces are selected to count` },
    ),
  };
}
