import { invokeCommand, subscribe } from "../transport/transport";

export interface WatchHandle {
  stop: () => void;
}

/** Connection health of a watch. */
export type WatchStatus = "live" | "reconnecting";

/** Resource kinds that support live watching. */
export const WATCHABLE_KINDS = [
  "pods",
  "deployments",
  "statefulsets",
  "daemonsets",
  "jobs",
  "cronjobs",
  "configmaps",
  "secrets",
  "resourcequotas",
  "limitranges",
  "services",
  "ingresses",
  "endpointslices",
  "networkpolicies",
  "persistentvolumeclaims",
  "persistentvolumes",
  "storageclasses",
  "serviceaccounts",
  "roles",
  "clusterroles",
  "rolebindings",
  "clusterrolebindings",
  "events",
] as const;

// Monotonic id so each watch gets a unique channel (avoids cross-view mixups).
let watchSeq = 0;

/**
 * Start a live watch for a watchable resource kind. The Rust backend streams
 * full summary snapshots over a Tauri event channel; `onRows` is called with
 * each snapshot. Call `stop()` to unsubscribe and cancel the backend watch.
 *
 * The listener is registered BEFORE the backend watch starts, so the initial
 * snapshot (emitted as soon as the watch's list completes) can't race ahead of
 * the subscription and get lost — which previously left the view stuck loading.
 */
export async function watchResource(
  context: string,
  namespace: string,
  kind: string,
  onRows: (rows: Array<{ name: string }>) => void,
  onStatus?: (status: WatchStatus) => void,
  onError?: (error: string) => void,
  kubeconfigFiles: string[] = [],
): Promise<WatchHandle> {
  // Tauri event names allow only [alphanumeric, -, /, :, _], but a context name
  // can contain other characters (e.g. the "@" in "admin@cluster"), which makes
  // `listen` throw. Sanitize to the allowed set; the `watchSeq` suffix keeps
  // every channel unique regardless of any collisions the replacement introduces.
  const channel = `watch:${kind}:${context}:${namespace}:${++watchSeq}-${Math.random().toString(36).slice(2, 10)}`.replace(/[^a-zA-Z0-9/:_-]/g, "_");
  const dispose = await subscribe(channel, (payload) => {
    // The backend emits a snapshot (array), a `{status}` object, or, for a
    // permanent (403/401) failure that won't self-heal, an `{error}` object.
    if (Array.isArray(payload)) {
      onRows(payload as Array<{ name: string }>);
    } else if (payload && typeof payload === "object" && "status" in payload) {
      onStatus?.((payload as { status: WatchStatus }).status);
    } else if (payload && typeof payload === "object" && "error" in payload) {
      onError?.(String((payload as { error: unknown }).error));
    }
  });
  try {
    await invokeCommand("start_resource_watch", {
      context,
      namespace,
      kind,
      channel,
      kubeconfigPaths: kubeconfigFiles,
    });
  } catch (e) {
    dispose();
    throw e;
  }
  return {
    stop: () => {
      dispose();
      void invokeCommand("stop_watch", { channel });
    },
  };
}

/** The namespace scopes to watch for a selection: `[""]` (cluster scope) for none. */
function scopesFor(selection: string[]): string[] {
  return selection.length === 0 ? [""] : [...new Set(selection)];
}

function mergeKey(row: { name: string; namespace?: string }): string {
  return `${row.name}\0${row.namespace ?? ""}`;
}

/**
 * {@link watchResource} over a namespace selection, as one watch.
 *
 * An empty selection is "all namespaces": one cluster-scope watch. Anything
 * else is one namespaced watch PER selected namespace, never a cluster-scope
 * watch narrowed afterwards. A credential scoped to a few namespaces is
 * refused a cluster-scope list outright (#688), and on a large cluster the
 * cluster-scope watch streamed every namespace to draw two of them.
 *
 * Each namespace's latest snapshot is held apart and the union is emitted
 * ordered by name then namespace, the backend's own snapshot order. Nothing is
 * emitted until every namespace has either answered or failed: a list missing
 * a namespace that simply has not answered yet would be painted as loaded.
 *
 * A failure is reported with the namespace it came from (`""` for the cluster
 * scope), and the message is passed through untouched so `describeError` can
 * still classify it. The namespaces that answered keep their rows — one
 * refused namespace is a fact about that namespace, not about the list.
 *
 * Status is `reconnecting` while any one watch is.
 */
export async function watchNamespaces(
  context: string,
  selection: string[],
  kind: string,
  onRows: (rows: Array<{ name: string; namespace?: string }>) => void,
  onStatus?: (status: WatchStatus) => void,
  onError?: (error: string, namespace: string) => void,
  kubeconfigFiles: string[] = [],
): Promise<WatchHandle> {
  const scopes = scopesFor(selection);
  if (scopes.length === 1) {
    const [only] = scopes;
    return watchResource(context, only, kind, onRows, onStatus, (e) => onError?.(e, only), kubeconfigFiles);
  }

  const snapshots = new Map<string, Array<{ name: string; namespace?: string }>>();
  const failed = new Set<string>();
  const reconnecting = new Set<string>();

  const emitIfSettled = () => {
    if (snapshots.size === 0) return;
    if (scopes.some((ns) => !snapshots.has(ns) && !failed.has(ns))) return;
    const merged = [...snapshots.values()].flat();
    merged.sort((a, b) => {
      const ka = mergeKey(a);
      const kb = mergeKey(b);
      return ka < kb ? -1 : ka > kb ? 1 : 0;
    });
    onRows(merged);
  };

  const started = await Promise.allSettled(
    scopes.map((ns) =>
      watchResource(
        context,
        ns,
        kind,
        (rows) => {
          snapshots.set(ns, rows);
          emitIfSettled();
        },
        (status) => {
          const before = reconnecting.size > 0;
          if (status === "live") reconnecting.delete(ns);
          else reconnecting.add(ns);
          const after = reconnecting.size > 0;
          if (before !== after) onStatus?.(after ? "reconnecting" : "live");
        },
        (error) => {
          failed.add(ns);
          onError?.(error, ns);
          emitIfSettled();
        },
        kubeconfigFiles,
      ),
    ),
  );

  const handles = started.flatMap((r) => (r.status === "fulfilled" ? [r.value] : []));
  const rejected = started.find((r): r is PromiseRejectedResult => r.status === "rejected");
  if (rejected) {
    for (const h of handles) h.stop();
    throw rejected.reason;
  }
  return {
    stop: () => {
      for (const h of handles) h.stop();
    },
  };
}
