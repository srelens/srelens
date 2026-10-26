import type { ClusterContext } from "@srelens/core";
import { useConsole } from "../../console";
import { useAssistant } from "../../lib/assistant";
import { openOnCluster } from "../../lib/openCluster";
import { openTab } from "../../lib/tabsStore";

/**
 * Put a question in the assistant's prompt and open `/agent`.
 *
 * `/agent`'s composer is the console dock, and the dock's draft lives in the
 * console provider precisely so it survives the dock moving between mount
 * points — so a draft set here is what that screen's prompt shows when it
 * mounts. Filled in, not sent: the reader sees the question and presses Enter.
 *
 * A question about something on a cluster opens on that cluster, because the
 * assistant asks about the cluster in focus.
 *
 * `undefined` while the AI Assistant is off (#968). Home spans every cluster,
 * so a caller asking about one checks that cluster with `isAssistantAllowed`.
 */
export function useAskAssistant(): ((question: string, context?: ClusterContext) => void) | undefined {
  const { setDraft } = useConsole();
  const { enabled } = useAssistant();
  if (!enabled) return undefined;
  return (question, context) => {
    setDraft(question);
    if (context) openOnCluster(context, "/agent");
    else openTab("/agent");
  };
}
