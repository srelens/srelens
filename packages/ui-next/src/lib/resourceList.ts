import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { watchNamespaces, type WatchHandle, type WatchStatus } from "@srelens/core";
import { rowKey, type KindDescriptor, type ListRow, type RowKey } from "./kinds/types";

export type ResourceListStatus = "loading" | "ready" | "empty" | "error";

/** One selected namespace whose listing failed, in a view of several (#688). */
export interface NamespaceFailure {
  namespace: string;
  error: string;
}

export interface ResourceList<Row> {
  rows: Row[];
  status: ResourceListStatus;
  error?: string;
  /** True when the polled list stopped at a backend row cap (#609). */
  truncated?: boolean;
  /**
   * In a view of several namespaces, each one whose listing failed — the
   * others' rows are still in `rows`. Empty for an all-namespaces or
   * one-namespace view, where `error` already says everything. `error` is
   * set as well, to the first failure's message, so a screen that does not
   * read this still warns rather than going quiet.
   */
  namespaceFailures: NamespaceFailure[];
  watch: WatchStatus;
  reload(): void;
}

const POLL_MS = 5000;
const ENRICH_MS = 10000;
const CACHE_LIMIT = 40;

// Memory-only, view-keyed row cache. Capped at CACHE_LIMIT entries, evicting
// the oldest on insert. Never persisted (R-6) — a cache that survived a
// restart would show a cluster's old workloads before its real ones.
let rowCache = new Map<string, unknown[]>();

function cacheGet(key: string): unknown[] | undefined {
  return rowCache.get(key);
}

function cacheSet(key: string, rows: unknown[]) {
  if (!rowCache.has(key) && rowCache.size >= CACHE_LIMIT) {
    const oldest = rowCache.keys().next().value;
    if (oldest !== undefined) rowCache.delete(oldest);
  }
  // Re-insert to keep the key fresh in insertion order (Map preserves it).
  rowCache.delete(key);
  rowCache.set(key, rows);
}

/** Test-only: clear the module-level cache between test cases. */
export function resetListCache() {
  rowCache = new Map();
}

/**
 * The namespace scopes a selection is listed over: `""` (cluster scope) for
 * none, otherwise each selected namespace on its own — never the cluster scope
 * narrowed afterwards, which a namespace-scoped credential is refused (#688).
 */
function scopesFor(namespaces: string[]): string[] {
  return namespaces.length === 0 ? [""] : [...new Set(namespaces)];
}

/**
 * The WHOLE selection is in the key: keyed on `""` as it was, a view of two
 * namespaces and "all namespaces" shared one cache entry.
 */
function viewKey(context: string, namespaces: string[], kind: string) {
  return `${context}|${scopesFor(namespaces).join(",")}|${kind}`;
}

function failuresOf(errors: Map<string, string>, scopes: string[]): NamespaceFailure[] {
  if (scopes.length < 2) return [];
  return scopes.flatMap((namespace) => {
    const error = errors.get(namespace);
    return error === undefined ? [] : [{ namespace, error }];
  });
}

function deriveStatus(rows: unknown[], error: string | undefined, loading: boolean): ResourceListStatus {
  if (loading) return "loading";
  if (error) return rows.length > 0 ? "ready" : "error";
  return rows.length === 0 ? "empty" : "ready";
}

/**
 * Merges `metrics` into `rows` by row identity, at render — never in state. A
 * row with no entry in `metrics` is returned untouched (same reference), so a
 * kind with no `enrich` (the vast majority) pays nothing for this.
 *
 * `rowKey`, not `row.name`: on "all namespaces" two namespaces each running an
 * `api-0` are two rows, and matching on the name alone put one pod's CPU and
 * memory on the other's row — which the table displays and sorts on.
 */
function mergeMetrics<Row extends ListRow>(rows: Row[], metrics: Map<RowKey, Partial<Row>> | undefined): Row[] {
  if (!metrics || metrics.size === 0) return rows;
  return rows.map((row) => {
    const extra = metrics.get(rowKey(row));
    return extra ? { ...row, ...extra } : row;
  });
}

/**
 * `forKey` is the view this payload was watched or polled for, carried in the
 * state so the render-time gate below can compare it against the view being
 * asked about. Required rather than optional on purpose: every write has to
 * stamp it, and the type is what makes forgetting one a compile error.
 */
interface ListState {
  rows: unknown[];
  error?: string;
  /** Set only for poll sources that report a backend row cap (#609). */
  truncated?: boolean;
  /** Per-namespace failures, by namespace; see {@link ResourceList.namespaceFailures}. */
  errors: Map<string, string>;
  loading: boolean;
  watch: WatchStatus;
  forKey: string;
}

const NO_ERRORS = new Map<string, string>();

function firstError(errors: Map<string, string>): string | undefined {
  return errors.values().next().value;
}

/**
 * The data engine for a resource-list screen: watch vs poll, a view-keyed row
 * cache, and cancellation, with no knowledge of columns or layout. Follows
 * the generation-counter pattern from useResource — a result that arrives
 * after the view changed or the component unmounted is dropped by comparing
 * a captured generation against the current one.
 *
 * What it returns is GATED on the view the held state was fetched for
 * matching the one passed in THIS render — `useObject`'s gate, in
 * `useObject`'s shape, and for the same reason. The effect below resets the
 * rows on a view change, but an effect runs after commit and after paint: on
 * the very render the caller switches cluster or namespace, the previous
 * view's rows are still in this hook's state, and a real browser paints one
 * committed frame pairing the NEW view's heading with the OLD view's table —
 * `status: "ready"`, under which every row action would run against the
 * cluster the reader has left. A settled-state test cannot see it (RTL
 * flushes effects synchronously), which is exactly how it survived review.
 *
 * The gate covers `metrics` too, which is held apart from the rows and merged
 * at render: without it, the change render fed {@link mergeMetrics} one
 * view's readings map, and a pod named the same in both views took the other
 * one's CPU — displayed, and sorted on.
 *
 * It is reachable on a MOUNTED hook, not only at mount: `Resources` renders
 * its inner component with no `key`, and no screen carries `key={name}` (see
 * `lib/clusterMoved.tsx`), so switching cluster changes the view under a hook
 * that stays put.
 *
 * A plain comparison computed fresh every render, not a second effect: it
 * holds on the very first commit after the view changes, and it cannot be
 * undone by a future refactor reordering effects.
 */
export function useResourceList<Row extends ListRow>(
  context: string,
  kind: string,
  descriptor: KindDescriptor<Row> | undefined,
  namespaces: string[],
  files: string[],
): ResourceList<Row> {
  const scopes = scopesFor(namespaces);
  const scopesKey = scopes.join(",");
  const key = viewKey(context, namespaces, kind);
  const gen = useRef(0);
  const [tick, setTick] = useState(0);
  const reload = useCallback(() => setTick((t) => t + 1), []);

  const [state, setState] = useState<ListState>(() => {
    const cached = cacheGet(key);
    return { rows: cached ?? [], error: undefined, errors: NO_ERRORS, loading: cached === undefined, watch: "live", forKey: key };
  });

  // Held apart from `state`: enrichment (pod/node metrics) runs on its own
  // cadence and must never gate or fail the list itself. Merged into the
  // returned rows at render time, in `mergeMetrics` below.
  const [metrics, setMetrics] = useState<Map<RowKey, Partial<Row>> | undefined>(undefined);

  useEffect(() => {
    const mine = ++gen.current;
    const cached = cacheGet(key);
    setState({ rows: cached ?? [], error: undefined, errors: NO_ERRORS, loading: cached === undefined, watch: "live", forKey: key });
    setMetrics(undefined);

    if (!descriptor) {
      return;
    }

    let enrichInterval: ReturnType<typeof setInterval> | undefined;
    if (descriptor.enrich) {
      const enrich = descriptor.enrich;
      const runEnrich = () => {
        // One reading per scope, merged. Each is best-effort on its own: a
        // cluster with no metrics-server, or a namespace whose metrics are
        // refused, must still list its rows — swallowed here, not surfaced
        // as `error`, and costing only that scope its readings.
        Promise.allSettled(scopes.map((ns) => enrich(context, ns))).then((results) => {
          if (gen.current !== mine) return;
          const merged = new Map<RowKey, Partial<Row>>();
          for (const r of results) {
            if (r.status === "fulfilled") for (const [k, v] of r.value) merged.set(k, v);
            else console.error(r.reason);
          }
          setMetrics(merged);
        });
      };
      runEnrich();
      enrichInterval = setInterval(runEnrich, descriptor.enrichMs ?? ENRICH_MS);
    }

    if (descriptor.source === "watch") {
      let handle: WatchHandle | undefined;
      let stopped = false;

      watchNamespaces(
        context,
        scopes[0] === "" ? [] : scopes,
        kind,
        (rows) => {
          if (gen.current !== mine) return;
          cacheSet(key, rows);
          setState((s) => ({ ...s, rows, loading: false }));
        },
        (status) => {
          if (gen.current !== mine) return;
          setState((s) => ({ ...s, watch: status }));
        },
        (error, ns) => {
          if (gen.current !== mine) return;
          setState((s) => {
            const errors = new Map(s.errors).set(ns, error);
            return { ...s, errors, error: firstError(errors), loading: false };
          });
        },
        files,
      ).then(
        (h) => {
          if (stopped || gen.current !== mine) {
            // A handle that resolves after cleanup is stopped immediately
            // rather than leaked.
            h.stop();
            return;
          }
          handle = h;
        },
        (e: unknown) => {
          // A failed watch start (e.g. the backend's invokeCommand rejects)
          // must surface as `error`, not leave the hook on `loading`
          // forever — errors are returned, never thrown.
          if (gen.current !== mine) return;
          setState((s) => ({ ...s, error: e instanceof Error ? e.message : String(e), loading: false }));
        },
      );

      return () => {
        if (gen.current === mine) gen.current++;
        stopped = true;
        handle?.stop();
        if (enrichInterval) clearInterval(enrichInterval);
      };
    }

    // source: "poll" — one load per scope, merged. A scope that failed keeps
    // the whole view's previous rows only when EVERY scope failed; otherwise
    // the ones that answered are the list and the rest are named failures.
    const load = descriptor.load;
    const runPoll = () => {
      if (!load) return;
      Promise.allSettled(scopes.map((ns) => load(context, ns))).then((results) => {
        if (gen.current !== mine) return;
        const errors = new Map<string, string>();
        const rows: unknown[] = [];
        let truncated = false;
        results.forEach((r, i) => {
          const ns = scopes[i];
          if (r.status === "rejected") {
            errors.set(ns, r.reason instanceof Error ? r.reason.message : String(r.reason));
          } else if (r.value.error) {
            errors.set(ns, r.value.error);
          } else {
            rows.push(...(r.value.rows ?? []));
            truncated ||= r.value.truncated === true;
          }
        });
        if (errors.size === scopes.length) {
          setState((s) => ({ ...s, errors, error: firstError(errors), truncated: undefined, loading: false }));
          return;
        }
        cacheSet(key, rows);
        setState((s) => ({
          ...s,
          rows,
          errors,
          error: firstError(errors),
          truncated: truncated || undefined,
          loading: false,
        }));
      });
    };
    runPoll();
    const interval = setInterval(runPoll, POLL_MS);

    return () => {
      if (gen.current === mine) gen.current++;
      clearInterval(interval);
      if (enrichInterval) clearInterval(enrichInterval);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [context, scopesKey, kind, descriptor, tick, files.join(",")]);

  // Enrichment changes on its own cadence. Keep the merged array stable between
  // list/metrics updates so consumers can key host reads to a real snapshot,
  // rather than each consumer render creating another snapshot and read.
  const rows = useMemo(() => mergeMetrics(state.rows as Row[], metrics), [state.rows, metrics]);

  // The gate itself. `reload` is handed back either way: it is stable, and a
  // caller must be able to retry the view it is asking about right now. The
  // remaining `setState`s in the effect are updater forms that spread the
  // state they are given, so they carry `forKey` through under the generation
  // guard above; the required field is what stops a future full write from
  // dropping it.
  if (state.forKey !== key) {
    return { rows: [], status: "loading", error: undefined, truncated: undefined, namespaceFailures: [], watch: "live", reload };
  }

  return {
    rows,
    status: deriveStatus(state.rows, state.error, state.loading),
    error: state.error,
    truncated: state.truncated,
    namespaceFailures: failuresOf(state.errors, scopes),
    watch: state.watch,
    reload,
  };
}
