import { useSyncExternalStore } from "react";
import { ConfirmDialog } from "@srelens/ui-kit";
import {
  answerClusterTerminal,
  onClusterTerminalPending,
  pendingClusterTerminal,
} from "../lib/clusterTerminal";
import { useProtectedConfirm } from "../lib/protectedConfirm";

/**
 * The question a protected cluster's local shell asks before it opens on
 * Windows (#101).
 *
 * Elsewhere the host makes that shell read-only. A Windows desktop has no
 * guards to do it with, so the shell would be an ordinary one on a cluster a
 * workspace protects: the reader types the cluster's name first, as for any
 * other change to it.
 *
 * `openClusterTerminal` holds the request, because the status bar and ⌘J reach
 * it from outside any component. This draws it, mounted once in the window's
 * band. New session's Local shell asks inside its own dialog instead.
 */
export function ClusterTerminalConfirm() {
  const pending = useSyncExternalStore(onClusterTerminalPending, pendingClusterTerminal);
  // Keyed, so a shell held for another cluster starts with an empty field.
  return pending ? <Ask key={pending.cluster.stableId} context={pending.cluster.name} /> : null;
}

function Ask({ context }: { context: string }) {
  const typed = useProtectedConfirm(context);
  // The rail's name for the cluster; the context's own if protection was
  // lifted while the question was up.
  const name = typed.protection?.name ?? context;
  return (
    <ConfirmDialog
      title={`Open a terminal on ${name}?`}
      confirmLabel="Open terminal"
      confirmDisabled={!typed.ready}
      onCancel={() => void answerClusterTerminal(false)}
      onConfirm={() => void answerClusterTerminal(true)}
      message={
        <>
          <p style={{ marginTop: 0 }}>
            {`On Windows this terminal can't be made read-only. kubectl in it can change ${name}.`}
          </p>
          {typed.field}
        </>
      }
    />
  );
}
