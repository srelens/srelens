import { useEffect } from "react";
import { getContexts, useContexts } from "./clusters";
import { isClusterPaused, useTabs } from "./tabsStore";

/** Capability calls use context names; pause preferences use stable ids. */
export function isContextPaused(name: string): boolean {
  return getContexts().some(context => context.name === name && isClusterPaused(context.stableId));
}

/** Hide immediately, then forget a dialog whose captured target was paused. */
export function useDismissOnPause(name: string | undefined, dismiss: () => void): boolean {
  const contexts = useContexts();
  const { workspace } = useTabs();
  const paused = contexts.some(context => context.name === name && workspace.pausedClusters?.includes(context.stableId));
  useEffect(() => {
    if (paused) dismiss();
  }, [paused, dismiss]);
  return paused;
}
