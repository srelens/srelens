import type { ClusterContext } from "@srelens/core";
import { getContextLabel, useMark } from "./marks";

/** Presentation only: callers retain the kubeconfig name for operations. */
export function useContextLabel(name: string, stableId?: string): string {
  useMark(stableId ?? "", name);
  return stableId ? getContextLabel(stableId, name) : name;
}

export function ContextLabel({ context }: { context: ClusterContext }) {
  return <>{useContextLabel(context.name, context.stableId)}</>;
}
