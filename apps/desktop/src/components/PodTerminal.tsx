import React, { useMemo } from "react";
import { TerminalPane } from "./TerminalPane";
import { podExecDriver } from "@srelens/core";

/**
 * Interactive in-pod shell (and node shells, via a `command` override). A thin
 * adapter over the shared {@link TerminalPane}, which owns the xterm instance,
 * PTY resize, scrollback search, and reconnect.
 */
export function PodTerminal({
  context,
  namespace,
  pod,
  container,
  command,
}: {
  context: string;
  namespace: string;
  pod: string;
  /** Exec into this specific container (for multi-container pods). */
  container?: string;
  /** Override the exec command (e.g. the node shell's `nsenter …`). */
  command?: string[];
}) {
  const driver = useMemo(
    () =>
      podExecDriver({
        context,
        namespace,
        pod,
        container,
        command,
        kind: command ? "node" : "pod",
      }),
    // command is a stable per-session array; joined into the deps key.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [context, namespace, pod, container, command?.join(" ")],
  );
  return <TerminalPane driver={driver} />;
}
