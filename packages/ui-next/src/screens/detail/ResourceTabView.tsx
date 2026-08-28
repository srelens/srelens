import { useId, type ReactNode } from "react";
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
import {
  Breadcrumb,
  KV,
  LoadingState,
  Stat,
  statusTone,
  Tabs,
  type StatProps,
  type TabItem,
} from "@srelens/ui-kit";
import { FailureState } from "../../lib/errorCopy";
import { CUSTOM_RESOURCE_ACTIONS } from "../../lib/kinds/custom";
import { formatCpu, formatMemory } from "../../lib/kinds/columns";
import { DetailActions } from "./DetailActions";
import { SectionMemory, Section } from "./Section";
import {
  describeTarget,
  useDetailPaneState,
  useDetailSubject,
  useLoad,
  PANE_DETAILS,
  PANE_EVENTS,
  PANE_METRICS,
  PANE_YAML,
} from "./detailData";
import type { DetailFact } from "./facts";

export interface ResourceTabViewProps {
  context: string;
  /** The Kubernetes kind, as the route carries it — not the list's slug. */
  kind: string;
  namespace: string | null;
  name: string;
}

/** One tile of the strip. Absent facts are omitted, never drawn as em dashes. */
type Tile = StatProps & { key: string };

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
 * THE FULL TAB'S OWN FACT LAYOUT: three columns of label-above-value, each
 * pair ruled off beneath, filling the width of a page.
 *
 * The mock's grid, built here, by the screen that draws it. The peek reads the
 * very same facts down one column of label-beside-value rows and builds that
 * itself. What the two share is the LIST (`detailFacts`), so they cannot
 * disagree about what a subject says; what neither shares is a line of the
 * other's markup, so a change to this grid changes nothing in the peek.
 *
 * It replaces `FactGrid` — a kit wrapper that took the peek's rendered rows
 * and restyled them into these columns. That wrapper had to describe a layout
 * in terms of children it did not build (`.factgrid .section > :not(.kv)`,
 * then a second rule for tables, and a reviewer's note that a third kind of
 * child was coming), and it made the tab's layout a property of the peek's
 * DOM. `KV`'s own `stacked` form is what a row looks like here; the grid
 * around them is this screen's. (#331)
 *
 * Untitled, like the peek's list and for the same reason: the header above has
 * already named the subject, and an untitled block cannot fold, so Overview
 * can never open showing nothing.
 */
function TabFacts({ facts }: { facts: DetailFact[] }) {
  if (facts.length === 0) return null;
  return (
    <Section>
      <div data-slot="fact-grid" className="grid grid-cols-3 gap-x-6">
        {facts.map((fact) => (
          <KV key={fact.label} stacked k={fact.label} v={fact.value} mono={fact.mono} />
        ))}
      </div>
    </Section>
  );
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
 * and puts them in the header, calls the first pane Overview rather than
 * Details, folds the containers table into it, and reads its facts across
 * three columns instead of down one.
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
export function ResourceTabView({ context, kind, namespace, name }: ResourceTabViewProps) {
  const subject = useDetailSubject({ context, kind, namespace, name });
  const { object, status, error, descriptor, statusLine, hasMetrics } = subject;

  // THIS SCREEN'S STRIP: `Overview YAML Events Metrics`. Overview rather than
  // the peek's Details, and no Containers tab at all — the design puts that
  // table inline, which a page has the room for and a 352px column does not.
  const tabs: TabItem[] = [
    { id: PANE_DETAILS, label: "Overview" },
    { id: PANE_YAML, label: "YAML" },
    { id: PANE_EVENTS, label: "Events" },
    ...(hasMetrics ? [{ id: PANE_METRICS, label: "Metrics" }] : []),
  ];
  const { active, selectTab, yamlPane, eventsPane } = useDetailPaneState({
    context,
    kind,
    namespace,
    name,
    tabs,
  });

  // Hooks before any early return, so the fetch below is never conditional on
  // the object having arrived. Only a Pod is looked up — see `usageTiles`.
  const target = [context, kind, namespace, name] as const;
  const wantsUsage = kind === "Pod" && namespace !== null;
  const usage = useLoad<{ cpu: number; memory: number } | null>(wantsUsage, target, async () => {
    const result = await podMetrics(context, namespace ?? "");
    const mine = result.metrics?.find((m) => m.name === name);
    return { data: mine ? { cpu: mine.cpuMillicores, memory: mine.memoryMiB } : null };
  });

  const headingId = useId();
  const activeLabel = tabs.find((t) => t.id === active)?.label;

  if (status === "loading") {
    return <LoadingState label={`Loading ${describeTarget(kind, namespace, name)}`} />;
  }
  if (status === "error" || !object) {
    return <FailureState title={`Could not load ${describeTarget(kind, namespace, name)}`} error={error} />;
  }

  const tiles = metricTiles(statusLine, object, usageTiles(usage.data?.cpu, usage.data?.memory));

  // The tab's Overview: the fact grid, then the kind's own blocks, then the
  // containers table the design folds in here, then Labels and Annotations
  // side by side. Every block is a sibling of every other — `.section +
  // .section` draws the hairlines — except the metadata pair, which is a row
  // on purpose (see below).
  let pane: ReactNode =
    active === PANE_DETAILS ? (
      <>
        <TabFacts facts={subject.facts} />
        {subject.body}
        {subject.containersTable}
        {/* Two columns, and each wrapped, so neither section is the other's
            adjacent sibling — `.section + .section` would otherwise rule down
            the middle of the row instead of across it. */}
        <div data-slot="metadata-pair" className="rule-t grid grid-cols-2">
          <div>{subject.labels}</div>
          <div className="rule-l">{subject.annotations}</div>
        </div>
      </>
    ) : null;
  if (active === PANE_METRICS) pane = subject.metricsPane;
  else if (active === PANE_YAML) pane = yamlPane;
  else if (active === PANE_EVENTS) pane = eventsPane;

  return (
    <section
      aria-labelledby={headingId}
      data-slot="resource-tab"
      className="flex min-h-0 min-w-0 flex-1 flex-col"
    >
      <header className="rule-b flex items-center gap-3 px-3 py-2">
        <div className="flex min-w-0 items-baseline gap-2.5">
          {/* An `h1`: this is a page, and the tab strip above it is chrome.
              The peek's subject is an `h2` because a peek sits inside a
              screen that already has one. */}
          <h1 id={headingId} className="truncate text-[1.25rem] font-semibold">
            {name}
          </h1>
          {/* Cluster, namespace, kind — the design's small-caps trail. A
              cluster-scoped subject has no namespace to place, so the trail is
              two steps rather than a step reading "—". */}
          <Breadcrumb
            className="shrink-0"
            parts={namespace ? [context, namespace, kind] : [context, kind]}
          />
        </div>
        <span className="flex-1" />
        <div data-slot="tab-actions" className="flex shrink-0 items-center gap-1.5">
          <DetailActions
            host="tab"
            context={context}
            kind={kind}
            namespace={namespace}
            name={name}
            // A kind outside `K8S_KIND` has no descriptor at all — the
            // custom-resource case — so it inherits `customDescriptor`'s own
            // action set, Delete withheld and all.
            actions={descriptor?.actions ?? CUSTOM_RESOURCE_ACTIONS}
            flagged={statusLine?.flagged ?? false}
            suspended={object.spec?.suspend === true}
          />
        </div>
      </header>

      <div className="rule-b px-3">
        <Tabs
          tabs={tabs}
          active={active}
          onChange={selectTab}
          label="Resource views"
          variant="underline"
        />
      </div>

      {/* Between the strip and the panel rather than inside it: these are the
          subject's vital signs, as worth having beside its manifest as beside
          its facts. `.stat + .stat` draws the vertical rules the design puts
          between the tiles; the row sizes them, which is what `Stat`'s own
          comment asks a caller to do. */}
      {tiles.length > 0 && (
        <div data-slot="metric-strip" className="rule-b flex">
          {tiles.map(({ key, ...tile }) => (
            <Stat key={key} {...tile} className="flex-1" />
          ))}
        </div>
      )}

      <div
        className="pane-body"
        role="tabpanel"
        // Named rather than pointed at: `Tabs` puts no ids on its buttons, so
        // `aria-labelledby` would have nothing to reference.
        aria-label={activeLabel}
        // A scrolling region has to be reachable, or its content is unreadable
        // to anyone driving the page from the keyboard.
        tabIndex={0}
      >
        {/* The same fold memory the peek reads, wrapped by this screen around
            its own pane: the store is keyed by kind, so a block a reader opens
            in one screen is open in the other without either screen knowing
            the other exists. A provider renders no element, so the run of
            sections beneath is unbroken. */}
        <SectionMemory kind={kind}>{pane}</SectionMemory>
      </div>
    </section>
  );
}
