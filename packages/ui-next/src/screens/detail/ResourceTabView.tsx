import { ExtensionResourceSlot } from "../../extensions/Extensions";
import { ExtensionPanelSlot } from "../../extensions/ExtensionPanelSlot";
import { ExtensionProviderSlot } from "../../extensions/ExtensionProviderSlot";
import { ExtensionRelatedSlot } from "../../extensions/ExtensionRelatedSlot";
import { ExtensionPodSlot } from "../../extensions/ExtensionPodTools";
import type { ReactNode } from "react";
import {
  ageFromTimestamp,
  asRecord,
  podContainerStatuses,
  podMetrics,
  restartTotal,
  str,
  type K8sObject,
  type ResourceStatusLine,
} from "@srelens/core";
import { LoadingState, StatusPill, statusTone } from "@srelens/ui-kit";
import { FailureState } from "../../lib/errorCopy";
import { CUSTOM_RESOURCE_ACTIONS } from "../../lib/kinds/custom";
import { formatCpu, formatMemory } from "../../lib/kinds/columns";
import { DetailActions } from "./DetailActions";
import { ResourceTabFrame, TabFacts, TabMetadata, type Tile } from "./ResourceFrame";
import { SectionMemory } from "./Section";
import {
  describeTarget,
  detailTabs,
  useDetailPaneState,
  useDetailSubject,
  useLoad,
  PANE_DETAILS,
  PANE_EVENTS,
  PANE_METRICS,
  PANE_YAML,
} from "./detailData";

export interface ResourceTabViewProps {
  context: string;
  /** The Kubernetes kind, as the route carries it — not the list's slug. */
  kind: string;
  /** The API group — a custom kind only; see `detailRoute`. */
  group?: string;
  namespace: string | null;
  name: string;
}

/**
 * The pod usage the CPU and Memory tiles show.
 *
 * Only a Pod has this, and that is a fact about where the data comes from
 * rather than a taste: `k8s.podMetrics` reports per-pod usage in a namespace,
 * so there is nothing to look this subject up by unless the subject IS a pod.
 * A workload's own usage would be the sum of its pods' and is a different
 * question, asked of a different call.
 *
 * Best-effort, exactly as `RelatedPodsSection` treats the same call: a cluster
 * with no metrics-server must cost the reader two tiles, not the page. It
 * rides `useLoad`, so it is target-gated like every other fetch in this
 * screen — a tile can never show the previous subject's usage — and it is
 * fetched once per subject rather than per pane switch.
 */
function usageTiles(cpuMillicores?: number, memoryMiB?: number): Tile[] {
  if (cpuMillicores === undefined || memoryMiB === undefined) return [];
  return [
    // The captions are the mock's, and they name what the figure IS — a
    // reading taken now, and the resident set the kernel counts. Neither is a
    // verdict, so neither takes a tone: a tone here would be this file
    // deciding what "a lot of memory" means, which is not a thing srelens
    // knows about someone else's workload.
    { key: "cpu", label: "CPU", value: formatCpu(cpuMillicores), delta: "current" },
    { key: "memory", label: "Memory", value: formatMemory(memoryMiB), delta: "working set" },
  ];
}

/**
 * The design's strip: Ready, Restarts, CPU, Memory, Age — and only the ones
 * this subject actually has. THIS SCREEN'S ALONE; the peek states the same
 * verdict as one line of words under the subject's name.
 *
 * Same discipline as the tab strip above it: a ConfigMap has no health, no
 * containers and no usage, so it gets one tile rather than four em dashes
 * dressed up as figures.
 *
 * THE READY TILE TAKES NO OPINION OF ITS OWN. Its figure, its caption and its
 * tone are all `resourceStatusLine`'s — core's single verdict on a subject,
 * the very one the peek's header word and the list row's pill are drawn from.
 * The mock's caption reads "all available"; srelens's word for that state is
 * whatever core says, which for a healthy pod is "Running" and for a Job is
 * "Complete". Writing the mock's phrase here would have meant a table pairing
 * a form of words with a colour, and six of those have been found and removed
 * on this branch.
 *
 * The figure is core's whole ready PHRASE ("1/1 ready", "3/3 complete") rather
 * than a ratio cut out of it. The noun belongs to the phrase because it is not
 * "ready" for every kind, and slicing it off here would be this file parsing a
 * string core deliberately assembled.
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
    // kubelet has not reported yet. The header has no line of its own for it.
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
      // NOT toned — how many restarts are too many is the reader's judgement
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
 * The resource FULL TAB: one subject filling a tab of its own, headed by its
 * name and the trail that locates it, its actions on the same line, a strip of
 * figures beneath, then its panes.
 *
 * ONE OF TWO SCREENS, and it knows nothing about the other. Spec rule R-5 said
 * the peek and the full tab were the same pane and the user's own mock of this
 * tab retired it: the peek heads a column with a name, a kind and a status
 * line and puts its actions in a footer; this heads a page with a breadcrumb
 * and puts them in the header, folds the containers table into Overview, and
 * reads its facts across three columns instead of down one.
 *
 * WHAT IT DOES NOT DUPLICATE, which is the point: the object is read once, the
 * panes load lazily under one rule, the per-kind blocks are the shared ones,
 * the facts are one derivation, and the actions are the row menu's — all of it
 * through `detailData` and {@link DetailActions}. The two screens differ in
 * how a fact reads and cannot differ in what it says.
 *
 * Deferred, and deliberately not scaffolded: the right-hand agent rail, the
 * Relations and Drill tabs, and the ask bar across the bottom of the window.
 * Each needs something that does not exist yet, and a tab named after an empty
 * pane is worse than an absent one.
 */
export function ResourceTabView({ context, kind, group, namespace, name }: ResourceTabViewProps) {
  const subject = useDetailSubject({ context, kind, namespace, name, group });
  const { object, status, error, descriptor, statusLine, hasMetrics } = subject;

  // No Containers tab — the design puts that table inline, which a page has the
  // room for and a 352px column does not.
  const tabs = detailTabs(hasMetrics ? [{ id: PANE_METRICS, label: "Metrics" }] : []);
  const { active, selectTab, yamlPane, eventsPane } = useDetailPaneState({
    context,
    kind,
    namespace,
    name,
    tabs,
    group,
  });

  // Hooks before any early return, so the fetch below is never conditional on
  // the object having arrived. Only a Pod is looked up — see `usageTiles`.
  const target = [context, kind, namespace, name] as const;
  // Never for a custom kind called Pod: the usage would be the built-in pod's (#919).
  const wantsUsage = !subject.custom && kind === "Pod" && namespace !== null;
  const usage = useLoad<{ cpu: number; memory: number } | null>(wantsUsage, target, async () => {
    const result = await podMetrics(context, namespace ?? "");
    const mine = result.metrics?.find((m) => m.name === name);
    return { data: mine ? { cpu: mine.cpuMillicores, memory: mine.memoryMiB } : null };
  });

  // Every state draws the frame: the name and the trail are the route's, not the
  // object's, so a read that is slow or failed still says what it was reading.
  const frame = (pane: ReactNode, tiles: Tile[] = [], actions?: ReactNode) => (
    <ResourceTabFrame
      name={name}
      cluster={context}
      namespace={namespace}
      kind={kind}
      tabs={tabs}
      active={active}
      onTabChange={selectTab}
      tiles={tiles}
      actions={actions}
    >
      {pane}
    </ResourceTabFrame>
  );

  if (status === "loading") {
    return frame(<LoadingState label={`Loading ${describeTarget(kind, namespace, name)}`} />);
  }
  if (status === "error" || !object) {
    return frame(
      <FailureState
        title={`Could not load ${describeTarget(kind, namespace, name)}`}
        error={error}
        onRetry={subject.reload}
      />,
    );
  }

  const tiles = metricTiles(statusLine, object, usageTiles(usage.data?.cpu, usage.data?.memory));
  // The figures above the grid have said these.
  const said = new Set(tiles.map((tile) => tile.key));
  const facts = subject.facts.filter((fact) => !fact.tile || !said.has(fact.tile));

  // The tab's Overview: the fact grid, then the kind's own blocks, then the
  // containers table the design folds in here, then Labels and Annotations
  // side by side. Every block is a sibling of every other — `.section +
  // .section` draws the hairlines — except the metadata pair, which is a row
  // on purpose (see below).
  let pane: ReactNode =
    active === PANE_DETAILS ? (
      <>
        <TabFacts facts={facts} />
        {subject.body}
        {subject.containersTable}
        <TabMetadata labels={subject.labels} annotations={subject.annotations} />
      </>
    ) : null;
  if (active === PANE_METRICS) pane = subject.metricsPane;
  else if (active === PANE_YAML) pane = yamlPane;
  else if (active === PANE_EVENTS) pane = eventsPane;

  return frame(
    <>
      {/* The same fold memory the peek reads, wrapped by this screen around
          its own pane: the store is keyed by kind, so a block a reader opens
          in one screen is open in the other without either screen knowing
          the other exists. A provider renders no element, so the run of
          sections beneath is unbroken. */}
      <SectionMemory kind={kind}>{pane}</SectionMemory>
      {active === PANE_DETAILS && <ExtensionPanelSlot context={context} resource={object}/>}
      {active === PANE_DETAILS && <ExtensionProviderSlot context={context} resource={object}/>}
      {active === PANE_DETAILS && <ExtensionRelatedSlot context={context} resource={object}/>}
      {active === PANE_DETAILS && <ExtensionPodSlot context={context} resource={object}/>}
      {active === PANE_DETAILS && <ExtensionResourceSlot context={context} kind={object.kind ?? kind} group={object.apiVersion?.includes("/") ? object.apiVersion.split("/")[0] : ""} namespace={namespace} name={name} />}
    </>,
    tiles,
    <DetailActions
      host="tab"
      context={context}
      kind={kind}
      group={group}
      namespace={namespace}
      name={name}
      // A kind outside `K8S_KIND` has no descriptor at all — the
      // custom-resource case — so it inherits `customDescriptor`'s own
      // action set, Delete withheld and all.
      actions={descriptor?.actions ?? CUSTOM_RESOURCE_ACTIONS}
      flagged={statusLine?.flagged ?? false}
      suspended={object.spec?.suspend === true}
      unschedulable={object.spec?.unschedulable === true}
      // Those two are read off this pane's object, so a write made from the
      // bar has to bring a new one: without it a node cordoned from here
      // went on being offered Cordon.
      onChanged={subject.refresh}
    />,
  );
}
