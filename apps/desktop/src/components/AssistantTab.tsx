import React from "react";
import { AssistantConversation } from "./AssistantConversation";

/**
 * Full-tab, workspace-level host for the assistant — opened globally rather
 * than scoped to a specific resource. When a cluster is active it's attached
 * as context (cluster + namespace, no resource kind/name); with no active
 * cluster the conversation carries no context at all.
 */
export function AssistantTab({ cluster, namespace }: { cluster: string | null; namespace?: string }) {
  return (
    <AssistantConversation
      className="h-full"
      context={cluster ? { context: cluster, namespace } : undefined}
    />
  );
}
