import { useCallback, useEffect, useRef, useState } from "react";
import type { Resource, ResourceStatus } from "./useResource";

/**
 * {@link useResource} with a memory of the last good answer.
 *
 * `useResource` refetches on every mount. That is right for a screen you open
 * once; it is wrong for a tab you come back to, where it means a spinner over
 * data the app already has and a whole cluster's worth of requests to arrive
 * at the same numbers. This is the same hook with three things added, and each
 * one is here because leaving it out broke something:
 *
 * 1. **A cache with a TTL.** Within {@link CACHE_TTL_MS} a remount paints the
 *    cached answer and issues NO request at all; past it, the cached answer is
 *    painted anyway and a refresh runs behind it. That second half is the
 *    stale-while-revalidate `resourceList` already does for list rows, and it
 *    is what makes returning to a tab feel instant rather than merely fast.
 * 2. **In-flight de-duplication.** Two readers of the same key that mount in
 *    the same frame — the overview's rail and its capacity strip both wanting
 *    the pod facts — share one request instead of racing two.
 * 3. **Two independent generation counters.** They guard different things and
 *    neither substitutes for the other:
 *    - a per-hook one, the rule {@link useResource} already has: a response
 *      for a context the reader has LEFT is dropped rather than painted over
 *      the cluster they are looking at now.
 *    - a module-wide CLEAR generation: deleting a pending promise cannot
 *      cancel its `.then`, so a request that was already in flight when the
 *      cache was cleared must not write its answer into the fresh one.
 *      Classic's overview learned this the hard way and its comment says so.
 *
 * **Nothing here is persisted** (R-6): a cache that survived a restart would
 * paint a cluster's old numbers before its real ones, and this screen's whole
 * argument is that a figure on it means what it says.
 *
 * ## Stale data is visibly stale
 *
 * Rows on screen plus a failed refresh is not an error state — throwing away
 * the last good answer loses information nobody asked to lose — and it is not
 * a healthy one either. It is {@link CachedResource.stale}: the data stays,
 * the reason comes with it, and the screen is expected to SAY so. A screen
 * that quietly kept showing figures that stopped refreshing would be lying by
 * omission, which is the same failure as a `0` in place of "no reading".
 */

/**
 * How long an answer counts as current.
 *
 * Classic's overview cache used the same thirty seconds and the figure has
 * held up: long enough that flicking between tabs never refetches, short
 * enough that nobody reads a minute-old node count as live. A reader who
 * wants certainty has `reload`, which ignores this entirely.
 */
export const CACHE_TTL_MS = 30_000;

/**
 * The most keys the cache holds — `resourceList`'s own limit, and the same
 * reasoning: a workspace of ten clusters times a handful of loaders each, with
 * room to spare, and a hard stop rather than a map that grows for the life of
 * the process.
 */
const CACHE_LIMIT = 40;

interface Entry<T> {
  data: T;
  /** When this answer was fetched, for the TTL and for the screen to show. */
  at: number;
}

let cache = new Map<string, Entry<unknown>>();
let inflight = new Map<string, Promise<Entry<unknown>>>();

/**
 * Bumped by every clear. A request captures it before it starts and writes to
 * the cache only if no clear happened in between.
 */
let clearGeneration = 0;

/** Forget every cached answer. Exported for tests and for a workspace reset. */
export function clearResourceCache(key?: string) {
  clearGeneration++;
  if (key !== undefined) {
    cache.delete(key);
    inflight.delete(key);
    return;
  }
  cache = new Map();
  inflight = new Map();
}

/** Test-only: how many answers the cache is holding. */
export function __cacheSizeForTests(): number {
  return cache.size;
}

function cacheSet(key: string, entry: Entry<unknown>) {
  if (!cache.has(key) && cache.size >= CACHE_LIMIT) {
    const oldest = cache.keys().next().value;
    if (oldest !== undefined) cache.delete(oldest);
  }
  // Re-inserted so the key is youngest in insertion order, which is the order
  // eviction reads (Map preserves it).
  cache.delete(key);
  cache.set(key, entry);
}

const defaultEmpty = (v: unknown) => v == null || v === "" || (Array.isArray(v) && v.length === 0);

/**
 * One load per key at a time, cached, with the clear generation honoured.
 *
 * `force` skips the TTL — it does NOT skip a request already in flight, which
 * is exactly what a reader pressing retry wants: the answer they are waiting
 * for is already on its way.
 */
function request<T>(key: string, load: () => Promise<T>, force: boolean): Promise<Entry<T>> {
  if (!force) {
    const hit = cache.get(key);
    if (hit && Date.now() - hit.at < CACHE_TTL_MS) return Promise.resolve(hit as Entry<T>);
  }
  const pending = inflight.get(key);
  if (pending) return pending as Promise<Entry<T>>;

  const generation = clearGeneration;
  const running = load()
    .then((data) => {
      const entry: Entry<T> = { data, at: Date.now() };
      if (clearGeneration === generation) cacheSet(key, entry as Entry<unknown>);
      return entry;
    })
    .finally(() => {
      inflight.delete(key);
    });
  inflight.set(key, running as Promise<Entry<unknown>>);
  return running;
}

export interface CachedResource<T> extends Resource<T> {
  /**
   * The data on screen is the last good answer AND the refresh behind it
   * failed. The rows are real and they are no longer being updated; the screen
   * must say the second half rather than presenting the first as current.
   */
  stale: boolean;
  /** When the data on screen was fetched — absent when there is none. */
  updatedAt?: number;
}

/**
 * `forKey` is the key this payload was fetched for, carried in the state so
 * the render-time gate below can compare it against the key being asked
 * about. Required rather than optional on purpose: every write has to stamp
 * it, and the type is what makes forgetting one a compile error.
 */
type State<T> = {
  status: ResourceStatus;
  data?: T;
  error?: string;
  stale: boolean;
  updatedAt?: number;
  forKey: string;
};

/**
 * A cached, stale-while-revalidate resource. The `key` is the cache identity
 * and the effect's only dependency, so **it must name everything the loader
 * closes over** — the context above all. `load` is read through a ref, so it
 * may be a fresh closure on every render without retriggering anything.
 *
 * What it returns is GATED on the key the held state was fetched for matching
 * the one passed in THIS render — `useObject`'s gate, in `useObject`'s shape,
 * and for the same reason. The effect below resets the state on a key change,
 * but an effect runs after commit and after paint: on the very render the
 * caller switches cluster, the previous cluster's data is still in this
 * hook's state, and a real browser paints one committed frame pairing the NEW
 * cluster's heading with the OLD cluster's figures — `status: "ready"`,
 * `stale: false`, and the previous cluster's `updatedAt` printed beside them.
 * A settled-state test cannot see it (RTL flushes effects synchronously),
 * which is exactly how it survived review.
 *
 * It is reachable on a MOUNTED hook, not only at mount: `Overview` renders
 * its inner component with no `key`, and no screen carries `key={name}` (see
 * `lib/clusterMoved.tsx`), so switching cluster changes the key under a hook
 * that stays put.
 *
 * A plain comparison computed fresh every render, not a second effect: it
 * holds on the very first commit after the key changes, and it cannot be
 * undone by a future refactor reordering effects.
 */
export function useCachedResource<T>(
  key: string,
  load: () => Promise<T>,
  isEmpty: (v: T) => boolean = defaultEmpty,
): CachedResource<T> {
  const loadRef = useRef(load);
  loadRef.current = load;
  const emptyRef = useRef(isEmpty);
  emptyRef.current = isEmpty;

  const settled = useCallback((entry: Entry<T>, forKey: string): State<T> => {
    const status: ResourceStatus = emptyRef.current(entry.data) ? "empty" : "ready";
    return { status, data: entry.data, stale: false, updatedAt: entry.at, forKey };
  }, []);

  const [state, setState] = useState<State<T>>(() => {
    const hit = cache.get(key) as Entry<T> | undefined;
    return hit ? settled(hit, key) : { status: "loading", stale: false, forKey: key };
  });

  const gen = useRef(0);
  const [tick, setTick] = useState(0);
  const forced = useRef(false);
  const reload = useCallback(() => {
    forced.current = true;
    setTick((t) => t + 1);
  }, []);

  useEffect(() => {
    const mine = ++gen.current;
    const hit = cache.get(key) as Entry<T> | undefined;
    const force = forced.current;
    forced.current = false;

    // The cached answer goes up FIRST, whether or not a refresh follows it.
    // A spinner in front of data the app already has is the thing this hook
    // exists to remove.
    //
    // Whether a refresh follows is `request`'s to decide and NOT repeated
    // here: a second TTL check in this effect would be a rule with two homes
    // that no test can tell apart — disabling either one on its own left the
    // suite green, which is how the duplicate was found.
    setState(hit ? settled(hit, key) : { status: "loading", stale: false, forKey: key });

    request(key, () => loadRef.current(), force).then(
      (entry) => {
        if (gen.current !== mine) return;
        setState(settled(entry, key));
      },
      (e: unknown) => {
        if (gen.current !== mine) return;
        const error = e instanceof Error ? e.message : String(e);
        setState((prev) =>
          prev.data === undefined
            ? { status: "error", error, stale: false, forKey: key }
            : { ...prev, error, stale: true },
        );
      },
    );

    return () => {
      if (gen.current === mine) gen.current++;
    };
  }, [key, tick, settled]);

  // The gate itself. `reload` is handed back either way: it is stable, and a
  // caller must be able to retry the key it is asking about right now.
  const { forKey, ...current } = state;
  return forKey === key ? { ...current, reload } : { status: "loading", stale: false, reload };
}
