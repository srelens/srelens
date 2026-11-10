import { useEffect, useState } from "react";
import {
  ageFromTimestamp,
  asRecord,
  podBounds,
  podContainerStatuses,
  podMetrics,
  podUsage,
  restartTotal,
  str,
  type K8sObject,
  type PodResourceUsage,
  type ResourceStatusLine,
} from "@srelens/core";
import { Sparkline, StatusPill, statusTone } from "@srelens/ui-kit";
import { formatCpu, formatMemory, ownHistory, spanWords, usageTone } from "../../lib/kinds/columns";
import { rowKey } from "../../lib/kinds/types";
import { recordUsage, usageHistory, USAGE_POLL_MS, type UsageSample } from "../../lib/usageHistory";
import type { DetailSubject } from "./detailData";
import type { DetailFact } from "./facts";
import type { Tile } from "./ResourceFrame";

/**
 * A pod's last ten minutes of CPU and memory, kept moving while a detail of it
 * is open (#1074).
 *
 * The past is the Pods list's own (`lib/usageHistory`), so a peek opened from
 * the list starts with what the list has already read rather than with one
 * point. From there the detail reads for itself, on the list's cadence: a full
 * tab is often open with no list behind it, and a figure fetched once and
 * labelled "current" stops being current ten seconds later.
 *
 * Best-effort, as the list treats the same call: a cluster with no
 * metrics-server costs the reader two tiles, not the page. A read that fails,
 * or no longer mentions the pod, takes the tiles away until one succeeds, as
 * a failed poll takes a list row's figures away: the last good reading is not
 * "current" once the next one has failed. The past stays in the store, so the
 * graph comes back whole. Keyed by the subject, so a tile never shows the
 * previous subject's usage while the next one's first reading is on its way.
 */
function usePodUsage(enabled: boolean, context: string, namespace: string, name: string): UsageSample[] {
  const key = rowKey({ name, namespace });
  const subject = `${context} ${key}`;
  const [read, setRead] = useState<{ subject: string; samples: UsageSample[] } | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let live = true;
    const failed = () => {
      if (live) setRead({ subject, samples: [] });
    };
    const poll = () =>
      podMetrics(context, namespace).then((out) => {
        const mine = out.metrics?.find((m) => m.name === name);
        if (!mine) return failed();
        if (!live) return;
        const past = recordUsage(context, [{ key, cpu: mine.cpuMillicores, memory: mine.memoryMiB }]);
        setRead({ subject, samples: past.get(key) ?? [] });
      }, failed);
    void poll();
    const timer = setInterval(poll, USAGE_POLL_MS);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [enabled, context, namespace, name, key, subject]);

  if (!enabled) return [];
  return read?.subject === subject ? read.samples : usageHistory(context, key);
}

/**
 * The CPU and Memory tiles: the latest reading, a graph of the ones before it,
 * and how it stands against what the pod was given.
 *
 * Measured exactly as the Pods list measures it, through core's `podUsage`
 * over the pod's own bounds, so the two cannot mean different things by a
 * percentage: against the limit where every container sets one, else against
 * the request, else against nothing, when the caption says what the figure is
 * instead. The tone is the list's too, and colours the caption and the graph.
 */
function usageTiles(object: K8sObject, samples: UsageSample[]): Tile[] {
  const created = str(asRecord(object.metadata).creationTimestamp);
  const own = ownHistory({ created: created || undefined, usageHistory: samples });
  const latest = own[own.length - 1];
  if (!latest) return [];
  const usage = podUsage(podBounds(object), { cpuMillicores: latest.cpu, memoryMiB: latest.memory });
  const elapsed = latest.at - own[0].at;
  const tile = (
    key: string,
    label: string,
    value: string,
    of: (sample: UsageSample) => number,
    bound: PodResourceUsage | null,
    plainly: string,
  ): Tile => ({
    key,
    label,
    value,
    delta: bound ? `${Math.round(bound.percent)}% of ${bound.of}` : plainly,
    tone: usageTone(bound),
    children: (
      <Sparkline
        points={own.map(of)}
        at={own.map((s) => s.at)}
        height={24}
        ceiling={bound?.bound}
        tone={usageTone(bound)}
        // What the graph covers, which is less than ten minutes until the
        // history has been kept that long.
        ariaLabel={elapsed > 0 ? `${label} over the last ${spanWords(elapsed)}` : label}
      />
    ),
  });
  return [
    tile("cpu", "CPU", formatCpu(latest.cpu), (s) => s.cpu, usage.cpu, "current"),
    tile("memory", "Memory", formatMemory(latest.memory), (s) => s.memory, usage.memory, "working set"),
  ];
}

/**
 * The design's strip: Ready, Restarts, CPU, Memory, Age, and only the ones
 * this subject actually has. Drawn by both screens: across the full tab, and
 * at the top of the peek's Overview (#1074).
 *
 * A ConfigMap has no health, no containers and no usage, so it gets one tile
 * rather than four em dashes dressed up as figures.
 *
 * THE READY TILE TAKES NO OPINION OF ITS OWN. Its figure, its caption and its
 * tone are all `resourceStatusLine`'s: core's single verdict on a subject,
 * the very one the peek's header word and the list row's pill are drawn from.
 * The figure is core's whole ready PHRASE ("1/1 ready", "3/3 complete") rather
 * than a ratio cut out of it, because the noun is not "ready" for every kind.
 */
function metricTiles(statusLine: ResourceStatusLine | null, object: K8sObject, usage: Tile[]): Tile[] {
  const tiles: Tile[] = [];
  if (statusLine?.readyText) {
    tiles.push({
      key: "ready",
      label: "Ready",
      value: statusLine.readyText,
      delta: statusLine.status,
      tone: statusTone(statusLine.health),
    });
  } else if (statusLine) {
    // A verdict with no ready figure to hang it on: a Node, a CronJob, a Pod the
    // kubelet has not reported yet.
    tiles.push({
      key: "status",
      label: "Status",
      value: <StatusPill status={statusLine.status} kind={statusLine.health} tinted />,
    });
  }
  const statuses = podContainerStatuses(object.status);
  if (statuses.length > 0) {
    const restarts = restartTotal(statuses);
    tiles.push({
      key: "restarts",
      label: "Restarts",
      // "none" for zero, and nothing at all otherwise: the count is already
      // on the tile, and a caption repeating it in words says nothing. It is
      // NOT toned: how many restarts are too many is the reader's judgement
      // about their own workload, and core has no verdict to lend here.
      value: str(restarts),
      ...(restarts === 0 ? { delta: "none" } : {}),
    });
  }
  tiles.push(...usage);
  const created = str(asRecord(object.metadata).creationTimestamp);
  if (created) tiles.push({ key: "age", label: "Age", value: ageFromTimestamp(created) });
  return tiles;
}

/**
 * A subject's tiles, for either screen. A hook, and called before a screen's
 * early returns, so the usage reading is never conditional on the object
 * having arrived; there are no tiles until it has.
 */
export function useMetricTiles(
  subject: DetailSubject,
  { context, kind, namespace, name }: { context: string; kind: string; namespace: string | null; name: string },
): Tile[] {
  // Only a Pod: `podMetrics` reports per-pod usage, and a workload's would be
  // the sum of its pods', a different question. Never for a custom kind
  // called Pod, whose usage would be the built-in pod's (#919).
  const wantsUsage = !subject.custom && kind === "Pod" && namespace !== null;
  const samples = usePodUsage(wantsUsage, context, namespace ?? "", name);
  const { object, statusLine } = subject;
  return object ? metricTiles(statusLine, object, usageTiles(object, samples)) : [];
}

/** The facts a screen still has to say once its tiles have said theirs. */
export function factsBelow(facts: DetailFact[], tiles: Tile[]): DetailFact[] {
  const said = new Set(tiles.map((tile) => tile.key));
  return facts.filter((fact) => !fact.tile || !said.has(fact.tile));
}
