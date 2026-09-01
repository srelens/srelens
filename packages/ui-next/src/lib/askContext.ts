import { parseDetailRoute } from "./detailRoute";
import { parseLogsRoute } from "../screens/Logs";

/**
 * What a question is ABOUT — the cluster, and the resource on screen when the
 * route names one.
 *
 * **Why this is not just a cluster name.** The first version of the agent's
 * context preface carried the cluster alone, and it was not enough. Press
 * "Summarise this stream" on a pod's logs and the agent received
 * `Current context: cluster prod-eu.` with the question "Summarise the last 500
 * log lines and group errors by cause" — no pod, no namespace. Having no
 * target, it went looking for one: `listServices`, `listEvents` and `podLogs`
 * across `kube-system`, then two unrelated `m01-test-01-*` namespaces, then a
 * production one. Ten-plus tool calls, none of them the stream the reader was
 * looking at, and log lines read from namespaces nobody asked about.
 *
 * Every MCP tool call takes an explicit namespace and name. An agent given
 * only a cluster has to guess them, and "guess" here means "search the
 * cluster".
 */
export interface AskContext {
  /** Always known — the active cluster's context name. */
  cluster: string;
  namespace?: string;
  /** `Pod`, `Deployment`, … as core names them. */
  kind?: string;
  name?: string;
  /**
   * The surface the reader is looking at, when it narrows what they mean.
   * `logs` is the one that matters today: "summarise this stream" is a
   * question about a log stream, and saying so is the difference between the
   * agent reading that pod's logs and hunting for a pod to read.
   */
  surface?: "logs";
}

/**
 * The context for a question asked from this route.
 *
 * Derived from the route rather than from component state, because the route
 * IS a resource's identity here — `logsRoute` and `detailRoute` both bake
 * kind, namespace and name into it, which is why tabs dedupe by route string.
 * A route that names no resource yields the cluster alone, which is honest:
 * there is nothing more to say about `/overview`.
 */
export function askContextFor(route: string, cluster: string): AskContext {
  const logs = parseLogsRoute(route);
  if (logs) {
    return { cluster, namespace: logs.namespace, kind: logs.kind, name: logs.name, surface: "logs" };
  }
  const detail = parseDetailRoute(route);
  if (detail) {
    // `namespace` is `null` for a cluster-scoped kind — a Node, a
    // PersistentVolume. Left absent rather than sent as the literal "null",
    // which would be a namespace no cluster has.
    return {
      cluster,
      namespace: detail.namespace ?? undefined,
      kind: detail.kind,
      name: detail.name,
    };
  }
  return { cluster };
}
