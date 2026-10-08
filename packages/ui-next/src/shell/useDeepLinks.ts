import { useEffect, useRef, useState } from "react";
import { checkDeepLink, invokeCommand, isTauri, subscribe, targetNamespace } from "@srelens/core";
import { useContexts } from "../lib/clusters";
import { detailRoute } from "../lib/detailRoute";
import { openCluster, openOnCluster } from "../lib/openCluster";
import { useWorkspaceSealed } from "./LockGate";

/**
 * `srelens://` links, for the new design (#370). Classic's consumer in
 * `apps/desktop/src/App.tsx` is the model, and the backend half is
 * `apps/desktop/src-tauri/src/deep_link.rs`.
 *
 * The backend queues every link and only NUDGES (`deep-link-pending`), so a
 * link that cold-starts the app is not lost while the WebView boots. This
 * subscribes to the nudge FIRST and drains `take_pending_deep_links` once the
 * listener has landed: draining first leaves a gap in which a link is queued,
 * nudged to nobody, and left until some later link fires another nudge.
 *
 * What is drained is held here until the window has booted, because a link
 * judged against an empty context list would be refused as naming a context
 * that does not exist. It is held while the workspace is sealed, too: behind
 * the lock cover nothing in the workspace moves, and a link that opened a tab
 * and probed a cluster there would be a way to act on a locked window from
 * outside it.
 *
 * Each link is judged by core's `checkDeepLink`, the same rules and sentences
 * as classic. A refusal goes to `onRefused`, because this design has no toast
 * host. A link that passes opens on the cluster it NAMES, which becomes the one
 * in focus — see `openOnCluster` for why opening the route alone would show
 * the rail's cluster instead.
 *
 * Not deduped the way classic dedupes: `openTab` already dedupes by route, and
 * classic's key (cluster and kind) would drop the first of two pod links in one
 * cluster, which here are two different tabs.
 */
export function useDeepLinks({
  windowLabel,
  ready,
  onRefused,
}: {
  /** Only `main` drains: every window hears the nudge, and one drain takes the queue. */
  windowLabel: string;
  /** True once boot has listed the contexts and restored the workspace. */
  ready: boolean;
  onRefused: (reason: string) => void;
}): void {
  const [pending, setPending] = useState<string[]>([]);
  const contexts = useContexts();
  const sealed = useWorkspaceSealed();
  const refuse = useRef(onRefused);
  useEffect(() => {
    refuse.current = onRefused;
  });

  useEffect(() => {
    if (!isTauri() || windowLabel !== "main") return;
    let disposed = false;
    let release: (() => void) | undefined;
    const drain = () => {
      void invokeCommand<string[]>("take_pending_deep_links")
        .then((urls) => {
          if (urls.length > 0) setPending((queued) => [...queued, ...urls]);
        })
        .catch(() => {});
    };
    void subscribe("deep-link-pending", drain)
      .then((off) => {
        if (disposed) {
          off();
          return;
        }
        release = off;
        drain();
      })
      .catch(() => {});
    return () => {
      disposed = true;
      release?.();
    };
  }, [windowLabel]);

  useEffect(() => {
    if (!ready || sealed || pending.length === 0) return;
    // The whole queue, in order, so the last link is the one left in front.
    const queued = pending;
    setPending([]);
    const names = contexts.map((c) => c.name);
    for (const url of queued) {
      const check = checkDeepLink(url, names);
      if (!check.ok) {
        refuse.current(check.reason);
        continue;
      }
      const { target } = check;
      // Found by name, as `checkDeepLink` matched it.
      const context = contexts.find((c) => c.name === target.context);
      if (!context) continue;
      if (target.route === "cluster") {
        openCluster(context);
      } else {
        const namespace = targetNamespace(target.kind, target.namespace);
        openOnCluster(context, detailRoute(target.kind, namespace, target.name));
      }
    }
  }, [ready, sealed, pending, contexts]);
}
