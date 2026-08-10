import React from "react";
import { AssistantConversation } from "./AssistantConversation";

/**
 * Full-tab, workspace-level host for the assistant — opened globally rather
 * than scoped to a specific resource. `cluster` is always `null` on this tab
 * (it's not scoped to whichever cluster happens to be active elsewhere in the
 * workspace), so it never attaches a resource `context`; `availableContexts`
 * (all configured kube contexts) drives the multi-cluster select instead —
 * see `AssistantConversation`.
 */
export function AssistantTab({
  cluster,
  namespace,
  availableContexts,
}: {
  cluster: string | null;
  namespace?: string;
  availableContexts: string[];
}) {
  return (
    <AssistantConversation
      className="h-full"
      context={cluster ? { context: cluster, namespace } : undefined}
      availableContexts={availableContexts}
    />
  );
}
