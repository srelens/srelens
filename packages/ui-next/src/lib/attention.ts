import { useEffect, useMemo, useRef, useState } from "react";
import {
  listDaemonSets,
  listDeployments,
  listEvents,
  listStatefulSets,
  podOverview,
  podStatus,
  type ClusterContext,
  type StatusVerdict,
} from "@srelens/core";
import { daemonSetVerdict, deploymentVerdict, statefulSetVerdict } from "./kinds/columns";
import { involvedObject } from "./kinds/events";

/**
 * Home's "Needs attention": what is wrong right now across the workspace's
 * connected clusters, read with the calls the cluster overview already makes.
 *
 * **No new reads.** The overview's `Not ready` list is built from
 * `k8s.podOverview` and the three scaling-kind lists, judged by core's
 * `podStatus` and `scaledStatus`; this is the same five calls per cluster
 * (`listEvents` the fifth) and the same verdicts, so an item here and the
 * overview a click away cannot disagree about the same object. The
 * `/incidents` route still has no screen behind it, so there was no incident
 * feed to reuse.
 *
 * **A failed read stays a failure.** Each cluster's answer carries the reasons
 * any of its reads refused, beside whatever did answer, and the screen says so
 * — "nothing needs attention" is a claim about everything that was checked,
 * and a cluster nobody could look at was not checked.
 */

/**
 * How often the strip re-reads. Twice the result cache's thirty seconds and
 * twelve times the resource lists' fallback poll, so Home is never the
 * busiest reader in the window: five list calls per cluster is not free, and
 * a reader who wants it now has the cluster's own overview.
 */
export const ATTENTION_REFRESH_MS = 60_000;

/** How many clusters are read at once — each read is five calls in parallel. */
export const ATTENTION_CONCURRENCY = 3;

/** How far back a warning event still counts as happening now. */
const WARNING_WINDOW_MS = 60 * 60 * 1000;

export interface AttentionItem {
  /** `ClusterContext.stableId` — what the screen finds the context by. */
  clusterId: string;
  /** The context name, which every core call and the agent speak. */
  cluster: string;
  /** The Kubernetes kind, as `detailRoute` spells it. */
  kind: string;
  namespace: string;
  name: string;
  /** What is wrong, in its own words: kubectl's status, `1/3 ready`, an event's reason. */
  problem: string;
  cause: "crash" | "image" | "replicas" | "warning";
}

export interface ClusterAttention {
  items: AttentionItem[];
  /** Why part of this cluster could not be checked, raw, for `summarise`. Empty when every read answered. */
  failures: string[];
  /** The pod read stopped before the whole list — see `PodOverview.truncated`. */
  truncated: boolean;
  /**
   * The event list stopped at the backend's cap. Recent warnings can sit past
   * it on a busy cluster, so "no warnings in the last hour" is not a claim
   * this read can make.
   */
  eventsTruncated: boolean;
}

/** kubectl's words for a pod that cannot pull its image: the first try, the back-off, a bad name. */
const IMAGE_WORDS = /(^|:)(ErrImagePull|ImagePullBackOff|InvalidImageName)$/;

/**
 * A pod is crash-looping or failing to pull when kubectl's word says so —
 * the word `podStatus` reads, so a pod called out here is one the pod list
 * draws red. A Pending pod waiting for a node is not either, and is left to
 * the overview's `Not ready` list.
 */
function podCause(verdict: StatusVerdict): AttentionItem["cause"] | null {
  const word = String(verdict.status);
  if (IMAGE_WORDS.test(word)) return "image";
  if (word.includes("CrashLoopBackOff")) return "crash";
  return null;
}

/**
 * One cluster's attention items, and why any of it could not be read.
 *
 * Never rejects: every core call here returns its error rather than throwing,
 * and anything that throws anyway is caught into `failures` — one bad answer
 * must not take the other clusters' strip down with it.
 */
export async function readClusterAttention(
  context: Pick<ClusterContext, "stableId" | "name">,
  now = Date.now(),
): Promise<ClusterAttention> {
  const name = context.name;
  try {
    const [pods, deployments, statefulSets, daemonSets, events] = await Promise.all([
      podOverview(name),
      // The empty namespace is every namespace, as on the overview.
      listDeployments(name, ""),
      listStatefulSets(name, ""),
      listDaemonSets(name, ""),
      listEvents(name, null),
    ]);
    const failures = [pods.error, deployments.error, statefulSets.error, daemonSets.error, events.error]
      .filter((reason): reason is string => reason !== undefined && reason !== "");
    const items: AttentionItem[] = [];
    const seen = new Set<string>();
    const add = (item: Omit<AttentionItem, "clusterId" | "cluster">) => {
      // One row per object: a crash-looping pod also raises BackOff events,
      // and the reader needs to hear about the pod once.
      const key = `${item.kind}\u0000${item.namespace}\u0000${item.name}`;
      if (seen.has(key)) return;
      seen.add(key);
      items.push({ clusterId: context.stableId, cluster: name, ...item });
    };

    for (const pod of pods.pods?.unsettled ?? []) {
      const verdict = podStatus(pod);
      const cause = podCause(verdict);
      if (cause) add({ kind: "Pod", namespace: pod.namespace, name: pod.name, problem: String(verdict.status), cause });
    }
    const workloads = [
      ...(deployments.deployments ?? []).map((row) => ({ kind: "Deployment", row, verdict: deploymentVerdict(row), ready: row.ready })),
      ...(statefulSets.statefulsets ?? []).map((row) => ({ kind: "StatefulSet", row, verdict: statefulSetVerdict(row), ready: row.ready })),
      ...(daemonSets.daemonsets ?? []).map((row) => ({ kind: "DaemonSet", row, verdict: daemonSetVerdict(row), ready: `${row.ready}/${row.desired}` })),
    ];
    for (const { kind, row, verdict, ready } of workloads) {
      if (verdict.flagged) add({ kind, namespace: row.namespace, name: row.name, problem: `${ready} ready`, cause: "replicas" });
    }
    for (const event of events.events ?? []) {
      if (event.type !== "Warning") continue;
      const at = Date.parse(event.created ?? "");
      if (!Number.isFinite(at) || now - at > WARNING_WINDOW_MS) continue;
      const object = involvedObject(event);
      if (!object.kind || !object.name) continue;
      add({ kind: object.kind, namespace: event.namespace, name: object.name, problem: event.reason, cause: "warning" });
    }
    return { items, failures, truncated: pods.pods?.truncated ?? false, eventsTruncated: events.truncated ?? false };
  } catch (cause) {
    return { items: [], failures: [String(cause)], truncated: false, eventsTruncated: false };
  }
}

/** Broken pods before short workloads before warnings: the worst thing first. */
const RANK: Record<AttentionItem["cause"], number> = { crash: 0, image: 0, replicas: 1, warning: 2 };

/**
 * Every answered target's items in one list, worst first and otherwise in
 * target order — the order the strip draws and the suggestions are taken in.
 */
export function rankedAttention(targets: readonly ClusterContext[], scans: Readonly<Record<string, ClusterAttention>>): AttentionItem[] {
  return targets.flatMap((t) => scans[t.stableId]?.items ?? []).sort((a, b) => RANK[a.cause] - RANK[b.cause]);
}

/** `namespace/name`, or the bare name for a cluster-scoped object such as a Node. */
export function attentionPath(item: Pick<AttentionItem, "namespace" | "name">): string {
  return item.namespace ? `${item.namespace}/${item.name}` : item.name;
}

/**
 * The question an item's Ask puts in the assistant's prompt. Names the object
 * by namespace and name and the cluster by its context name, which is what the
 * agent's tools take.
 */
export function attentionQuestion(item: AttentionItem): string {
  const path = attentionPath(item);
  switch (item.cause) {
    case "crash":
      return `Why is ${path} crash-looping on ${item.cluster}?`;
    case "image":
      return `Why can't ${path} pull its image on ${item.cluster}?`;
    case "replicas":
      return `Why does ${item.kind} ${path} have unavailable replicas on ${item.cluster}?`;
    case "warning":
      return `What is behind the ${item.problem} warning on ${item.kind} ${path} on ${item.cluster}?`;
  }
}

/** Run `work` over `items`, at most `limit` at a time. */
async function eachLimited<T>(items: readonly T[], limit: number, work: (item: T) => Promise<void>): Promise<void> {
  let next = 0;
  const lane = async () => {
    while (next < items.length) await work(items[next++]);
  };
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, lane));
}

/**
 * Each target's latest answer, keyed by `stableId`, re-read every
 * {@link ATTENTION_REFRESH_MS}.
 *
 * A cluster that has not answered yet has no key — absent is "not known",
 * never "nothing wrong". Nothing is read while `paused` (the workspace is
 * sealed, or Home is not the tab on screen) or while the window is hidden, and
 * a window shown again after a missed refresh reads straight away. A cluster
 * that leaves `targets` — paused, disconnected — leaves the answer too.
 */
export function useAttention(targets: readonly ClusterContext[], paused: boolean): Readonly<Record<string, ClusterAttention>> {
  const [scans, setScans] = useState<Record<string, ClusterAttention>>({});
  // The effect's identity: which clusters, by id and by the name the calls use.
  const key = targets.map((t) => `${t.stableId}\u0000${t.name}`).join("\n");
  // The batch last started, across effect runs: a new set of clusters waits for
  // the reads already out, so two batches never run side by side.
  const batch = useRef<Promise<void>>(Promise.resolve());

  useEffect(() => {
    if (paused || targets.length === 0) return;
    let alive = true;
    let running = false;
    let last = -Infinity;
    const run = async () => {
      if (running || document.visibilityState === "hidden") return;
      running = true;
      const previous = batch.current;
      const mine = (async () => {
        await previous;
        let cut = false;
        last = Date.now();
        await eachLimited(targets, ATTENTION_CONCURRENCY, async (context) => {
          // Before EACH read, not once per batch: a pause or a hidden window
          // stops the reads not yet started; the ones out are let land.
          if (!alive || document.visibilityState === "hidden") {
            cut = true;
            return;
          }
          const scan = await readClusterAttention(context);
          if (alive) setScans((held) => ({ ...held, [context.stableId]: scan }));
        });
        // A batch cut short is due again as soon as the window is shown.
        if (cut) last = -Infinity;
      })();
      batch.current = mine;
      await mine;
      running = false;
      // Cut short, and the window shown again while reads were still out: its
      // catch-up found this batch running and stood down, so the skipped
      // clusters are read now rather than at the next tick.
      if (alive && last === -Infinity && document.visibilityState === "visible") void run();
    };
    const onVisible = () => {
      if (document.visibilityState === "visible" && Date.now() - last >= ATTENTION_REFRESH_MS) void run();
    };
    void run();
    const timer = setInterval(() => void run(), ATTENTION_REFRESH_MS);
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      alive = false;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
    // `key` stands for `targets`: a new array of the same clusters is not a reason to re-read.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paused, key]);

  return useMemo(() => {
    const held: Record<string, ClusterAttention> = {};
    for (const t of targets) if (scans[t.stableId]) held[t.stableId] = scans[t.stableId];
    return held;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scans, key]);
}
