import { K8S_KIND } from "@srelens/core";
import { parseDetailRoute } from "./detailRoute";
import { parseLogsRoute } from "../screens/Logs";
import { isBuiltInKind } from "./routes";

/** The agent screen's own route — the one tab that is not a subject. */
const AGENT_ROUTE = "/agent";

/**
 * The Kubernetes kind a built-in list route shows — `/k/pods` is `Pod`.
 *
 * `undefined` for anything that is not one of core's kinds: the control room,
 * `/helm`, a route nobody minted. Naming a kind srelens cannot resolve would
 * be worse than naming none.
 */
function listKind(route: string): string | undefined {
  const slug = route.startsWith("/k/") ? route.slice(3).split("/")[0] : "";
  // `isBuiltInKind` is a type guard, so the lookup below is indexed by a
  // `ResourceKind` rather than by any string that arrived from a route.
  return slug !== "" && isBuiltInKind(slug) ? K8S_KIND[slug] : undefined;
}

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
  /**
   * The namespaces the tab the question was asked from is narrowed to — the
   * standing selection behind the list screens' picker (`workspace.ts`'s
   * `useNamespaces`), NOT a property of any one resource.
   *
   * It is the reader's stated scope, and the agent has to be told or it
   * ignores it. On a StatefulSets list narrowed to one namespace, "which
   * MongoDB replica set spiked?" ran `podMetrics` across TEN namespaces —
   * every `*-dataservices` in the cluster — because nothing said the reader
   * had already answered that question with the picker.
   *
   * Carried for the tab the question comes FROM, and never for a conversation
   * started on the agent tab, which is about the cluster and has no list
   * behind it. Empty means the reader chose "all namespaces", which is a real
   * answer and gets no sentence: a preface claiming a scope the reader did not
   * set would be worse than none.
   */
  namespaces?: string[];
}

/**
 * The context for a question asked from this route.
 *
 * Derived from the route rather than from component state, because the route
 * IS a resource's identity here — `logsRoute` and `detailRoute` both bake
 * kind, namespace and name into it, which is why tabs dedupe by route string.
 * A route that names no resource still says what it can — which list is open,
 * and the namespaces that list is narrowed to. A route with none of that
 * yields the cluster alone, which is honest: there is nothing more to say
 * about `/overview`. The agent tab is the one route that says only the
 * cluster by rule rather than for want of anything to add.
 */
export function askContextFor(route: string, cluster: string, namespaces: string[] = []): AskContext {
  // The agent tab is not a subject. It is the full view of whichever
  // conversation is selected, so a NEW conversation started there is about the
  // cluster and nothing else — no list to be narrowed, no kind on screen.
  // Anything more would be scope borrowed from a tab the reader is not on.
  if (route === AGENT_ROUTE) return { cluster };

  // A route that names ONE resource is more specific than any standing
  // selection, so the picker is not carried alongside it — it would only
  // widen what is already exact.
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
  // A list: WHICH list, and the namespaces this tab is narrowed to. Both are
  // things the reader can see and the agent cannot — "pass kind type like
  // which tab is opened".
  //
  // The kind travels without a name, which is deliberate: `Pod` alone says
  // the reader is looking at pods, and the run key needs BOTH a kind and a
  // name before it treats a subject as its own conversation, so a list stays
  // keyed by its route.
  const kind = listKind(route);
  const about: AskContext = { cluster };
  if (kind !== undefined) about.kind = kind;
  if (namespaces.length > 0) about.namespaces = [...namespaces];
  return about;
}

/**
 * The identity of the conversation a question belongs to.
 *
 * A run is keyed by its SUBJECT, not by the route and not by the tab. Two
 * consequences worth stating, because both were deliberate:
 *
 * - A pod's logs and that same pod's detail produce the SAME key. Same
 *   subject, different lens — forking there would split a conversation the
 *   reader experiences as one.
 * - The namespace picker is NOT in the key. Narrowing is a filter adjusted
 *   while thinking, so re-narrowing must not fork the chat. It travels in the
 *   preface as scope instead (see {@link AskContext.namespaces}).
 * - A list's kind is not in the key either, for the same reason it is not a
 *   subject: `${cluster}|${route}` already separates one list from another.
 *
 * The cluster is always in the key: the same pod name in two clusters is two
 * subjects, which is the whole lesson of this migration's cluster-identity
 * findings.
 */
export function runKeyFor(about: AskContext, route: string): string {
  const cluster = about.cluster.trim();
  if (about.kind && about.name) {
    return `${cluster}|${about.kind}|${about.namespace ?? ""}|${about.name}`;
  }
  return `${cluster}|${route}`;
}

/**
 * A short human label for a run, for the rail that lists them.
 *
 * Says the subject, not the route: `ai-editor` and `statefulsets` are what a
 * reader recognises, and the cluster is already the rail's own context.
 */
export function runLabelFor(about: AskContext, route: string): string {
  if (about.kind && about.name) return `${about.kind}/${about.name}`;
  const tail = route.split("/").filter(Boolean).pop() ?? "cluster";
  return tail;
}
