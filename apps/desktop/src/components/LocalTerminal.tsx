import React, { useMemo } from "react";
import { TerminalPane } from "./TerminalPane";
import { localTerminalDriver } from "@srelens/core";

/**
 * A local shell (the user's login shell) scoped to a kube context so `kubectl`
 * targets it by default — distinct from the in-pod exec terminal. A thin
 * adapter over the shared {@link TerminalPane}; a shell exit is intentional, so
 * the pane offers a manual "Restart shell" rather than auto-reconnecting.
 */
export function LocalTerminal({
  context,
  kubeconfigFiles,
}: {
  context: string;
  /** Extra kubeconfig files the app has been told about (merged for the shell). */
  kubeconfigFiles: string[];
}) {
  const driver = useMemo(
    () => localTerminalDriver({ context, extraKubeconfigs: kubeconfigFiles }),
    [context, kubeconfigFiles],
  );
  return <TerminalPane driver={driver} banner={`kubectl scoped to \x1b[1m${context}\x1b[0m`} />;
}
