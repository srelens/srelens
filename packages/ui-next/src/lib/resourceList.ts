import { useCallback, useEffect, useRef, useState } from "react";
import { watchResource, type WatchHandle, type WatchStatus } from "@srelens/core";
import { rowKey, type KindDescriptor, type ListRow, type RowKey } from "./kinds/types";

export type ResourceListStatus = "loading" | "ready" | "empty" | "error";

export interface ResourceList<Row> {
  rows: Row[];
  status: ResourceListStatus;
  error?: string;
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

function viewKey(context: string, namespace: string, kind: string) {
  return `${context}|${namespace}|${kind}`;
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
 * The data engine for a resource-list screen: watch vs poll, a view-keyed row
 * cache, and cancellation, with no knowledge of columns or layout. Follows
 * the generation-counter pattern from useResource — a result that arrives
 * after the view changed or the component unmounted is dropped by comparing
 * a captured generation against the current one.
 */
export function useResourceList<Row extends ListRow>(
  context: string,
  kind: string,
  descriptor: KindDescriptor<Row> | undefined,
  namespace: string,
  files: string[],
): ResourceList<Row> {
  const key = viewKey(context, namespace, kind);
  const gen = useRef(0);
  const [tick, setTick] = useState(0);
  const reload = useCallback(() => setTick((t) => t + 1), []);

  const [state, setState] = useState<{ rows: unknown[]; error?: string; loading: boolean; watch: WatchStatus }>(() => {
    const cached = cacheGet(key);
    return { rows: cached ?? [], error: undefined, loading: cached === undefined, watch: "live" };
  });

  // Held apart from `state`: enrichment (pod/node metrics) runs on its own
  // cadence and must never gate or fail the list itself. Merged into the
  // returned rows at render time, in `mergeMetrics` below.
  const [metrics, setMetrics] = useState<Map<RowKey, Partial<Row>> | undefined>(undefined);

  useEffect(() => {
    const mine = ++gen.current;
    const cached = cacheGet(key);
    setState({ rows: cached ?? [], error: undefined, loading: cached === undefined, watch: "live" });
    setMetrics(undefined);

    if (!descriptor) {
      return;
    }

    let enrichInterval: ReturnType<typeof setInterval> | undefined;
    if (descriptor.enrich) {
      const enrich = descriptor.enrich;
      const runEnrich = () => {
        enrich(context, namespace).then(
          (result) => {
            if (gen.current !== mine) return;
            setMetrics(result);
          },
          (e: unknown) => {
            // Best-effort: a cluster with no metrics-server must still list
            // its rows. Swallowed here, not surfaced as `error`.
            console.error(e);
          },
        );
      };
      runEnrich();
      enrichInterval = setInterval(runEnrich, descriptor.enrichMs ?? ENRICH_MS);
    }

    if (descriptor.source === "watch") {
      let handle: WatchHandle | undefined;
      let stopped = false;

      watchResource(
        context,
        namespace,
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
        (error) => {
          if (gen.current !== mine) return;
          setState((s) => ({ ...s, error, loading: false }));
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

    // source: "poll"
    const load = descriptor.load;
    const runPoll = () => {
      if (!load) return;
      load(context, namespace).then(
        (result) => {
          if (gen.current !== mine) return;
          if (result.error) {
            setState((s) => ({ ...s, error: result.error, loading: false }));
            return;
          }
          const rows = result.rows ?? [];
          cacheSet(key, rows);
          setState((s) => ({ ...s, rows, error: undefined, loading: false }));
        },
        (e: unknown) => {
          if (gen.current !== mine) return;
          setState((s) => ({ ...s, error: e instanceof Error ? e.message : String(e), loading: false }));
        },
      );
    };
    runPoll();
    const interval = setInterval(runPoll, POLL_MS);

    return () => {
      if (gen.current === mine) gen.current++;
      clearInterval(interval);
      if (enrichInterval) clearInterval(enrichInterval);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [context, namespace, kind, descriptor, tick, files.join(",")]);

  return {
    rows: mergeMetrics(state.rows as Row[], metrics),
    status: deriveStatus(state.rows, state.error, state.loading),
    error: state.error,
    watch: state.watch,
    reload,
  };
}
