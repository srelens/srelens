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
  const channel = `watch:${kind}:${context}:${namespace}:${++watchSeq}`.replace(/[^a-zA-Z0-9/:_-]/g, "_");
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
