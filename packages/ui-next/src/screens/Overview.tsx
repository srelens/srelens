import { useCallback, useMemo, useState, type ReactNode } from "react";
import {
  K8S_KIND,
  cordonNode,
  drainNode,
  formatStorageSize,
  notify,
  podStatus,
  toKubectl,
  type ClusterCapacity,
  type ClusterContext,
  type HealthKind,
  type MetricsServerFact,
  type NodeSummary,
  type NodeUsage,
  type PodSummary,
  type StatusVerdict,
} from "@srelens/core";
import {
  ActionBar,
  Alert,
  Button,
  ClipboardCopyStatus,
  ConfirmDialog,
  EmptyState,
  ErrorState,
  RawError,
  KVList,
  KubectlPreview,
  LoadingState,
  Meter,
  Screen,
  Section,
  SideRail,
  Stat,
  StatusPill,
  StatusRow,
  Table,
  useClipboardCopy,
  loadTone,
  statusTone,
  type ActionBarAction,
  type ClipboardCopyController,
  type Column,
  type Tone,
} from "@srelens/ui-kit";
import { useConsole } from "../console";
import { useClusterGate } from "../lib/clusterMoved";
import { useActiveContext, useContexts } from "../lib/clusters";
import { detailRoute } from "../lib/detailRoute";
import { FailureLine, FailureState, FailureWord, summarise } from "../lib/errorCopy";
import { Icons } from "../lib/icons";
import {
  daemonSetVerdict,
  deploymentVerdict,
  nodeVerdict,
  podFlagged,
  statefulSetVerdict,
} from "../lib/kinds/columns";
import { ROW_ACTION_LABEL } from "../lib/kinds/rowActions";
import { UnhealthyDot } from "../lib/kinds/rowAffordances";
import {
  useOverview,
  type ObjectCounts,
  type Overview as OverviewData,
  type OverviewFacts,
  type OverviewNodes,
  type OverviewPods,
  type OverviewWorkloads,
} from "../lib/overview";
import { useInfo } from "../lib/probe";
import { describe } from "../lib/routes";
import { openTab, useTabs } from "../lib/tabsStore";
import { LINK_WORD, useWorkspaceView } from "../lib/workspace";
import { Fleet } from "./overview/Fleet";
import { NoClusterScreen } from "./resourceShell";

/**
 * What a figure says when there is no reading behind it.
 *
 * `null` from `nodeUsage`/`clusterCapacity` means nobody measured — no
 * metrics-server, a node that has not been scraped, a list that was refused.
 * `0%` is a measurement, and it reads as an idle cluster; an empty meter reads
 * as one too. This screen is the last layer, and the place the distinction
 * would be lost, so absence gets words rather than a zero.
 *
 * The words say only that there is no reading, never why. WHY is the rail's to
 * state, once — a screen where five tiles and two columns each announce a
 * missing metrics-server has said it seven times and explained it nowhere.
 */
const NO_READING = "No reading";

/** A fact the cluster did not carry. Never a guess in its place. */
const UNKNOWN = "—";

/** The header's one action, verbatim from the design (§7). */
const SUMMARISE_LABEL = "Summarise";
const SUMMARISE = "Summarise the health of this cluster";

/** §7's rail width, and this screen's alone — see `SideRail`'s note on widths. */
const RAIL_WIDTH = 286;

/**
 * How much of a context name the toolbar's eyebrow carries.
 *
 * The design's is `PROD-EU / GKE` — two short words. Real kubeconfig contexts
 * are not: `m01-1786968575165/kubernetes-admin@cluster.local` rendered in full
 * swamps the bar and pushes the screen's own title along with it.
 *
 * A number rather than a CSS truncation because the ellipsis has to fall in the
 * MIDDLE — see {@link shorten}. `text-overflow` only cuts the tail.
 */
const EYEBROW_MAX = 30;

/**
 * A context name cut to fit, keeping both ends.
 *
 * Cutting the tail is the one form that must not be used here: kubeadm names
 * end `…@cluster.local` on every cluster in a fleet, so `m01-178696…` and
 * `m02-178701…` are still told apart while `…@cluster.local` on both is not.
 * Cutting the head loses the opposite half. So the middle goes, and both ends
 * that distinguish one cluster from another survive.
 *
 * Nothing is hidden by it: the rail's `Context` row carries the whole name, one
 * region away, which is why the header does not need to.
 */
function shorten(text: string, max = EYEBROW_MAX): string {
  if (text.length <= max) return text;
  const head = Math.ceil((max - 1) / 2);
  return `${text.slice(0, head)}…${text.slice(text.length - (max - 1 - head))}`;
}

/** This screen's own words for the two node actions. Nothing else renders them. */
const NODE_ACTION_LABEL = {
  cordon: "Cordon",
  uncordon: "Uncordon",
  drain: "Drain",
} as const;

/** One node, flattened so the table can key, sort and filter on its fields. */
type NodeRow = NodeSummary & { usage: NodeUsage };

/**
 * `/overview` — the cluster overview (§7).
 *
 * Its three left-hand sections are here — the capacity strip, the nodes table
 * and the `Not ready` list — beside the `At a glance` rail's four.
 *
 * Split in two the way `Events.tsx` and `Workloads.tsx` are: with no cluster
 * in focus there is no context name to load anything for, and a hook cannot be
 * skipped, so the guard returns before any of them runs.
 */
export function Overview({ route }: { route: string }) {
  const context = useActiveContext();
  const title = describe(route, context?.name).title;

  if (!context) {
    return <NoClusterScreen title={title} noun="nodes" />;
  }

  return <ClusterOverview title={title} context={context} />;
}

function ClusterOverview({ title, context }: { title: string; context: ClusterContext }) {
  // Core takes a context *name*; the workspace holds a `stableId`. The two are
  // never interchangeable — see `lib/clusters`.
  const name = context.name;
  const overview = useOverview(name);
  const { ask } = useConsole();
  const Sparkle = Icons.ask;
  // The server version, from the probe the shell already ran at launch — the
  // same reading the rail's `Version` row takes, not a second call. Absent
  // until it lands, and the head then reads the cluster's name alone.
  const version = useInfo(context.stableId)?.version ?? "";

  // `<cluster name> / <provider>`, and just the name until the facts answer.
  // A provider row that said "unknown" would look like an answer; an absent
  // one says nothing, which is what the cluster said.
  const provider = overview.facts.facts?.provider ?? "";

  return (
    <Screen
      title={title}
      eyebrow={provider ? `${shorten(name)} / ${provider}` : shorten(name)}
      // The rail is full height beside the main column, so the body fills and
      // the column below scrolls inside itself rather than the page scrolling
      // the rail away with it.
      fill
      actions={
        // Exactly one header action, per §7. A `Button` rather than the row's
        // `AskChip` for the reason `Events.tsx` gives: the chip is invisible
        // until its row is hovered, which is right for one of forty rows and
        // invisible on a toolbar. The visible word is the design's; the
        // question it will actually send is the accessible name.
        <Button
          type="button"
          size="sm"
          aria-label={`${SUMMARISE_LABEL}: ${SUMMARISE}`}
          title={`${SUMMARISE_LABEL}: ${SUMMARISE}`}
          onClick={() => ask(SUMMARISE)}
        >
          <Sparkle size={12} aria-hidden="true" />
          {SUMMARISE_LABEL}
        </Button>
      }
    >
      <SideRail
        head="At a glance"
        // §7 heads the left pane too, level with the rail's own head:
        // `<name> · <version>`. The separator goes with the version, so a
        // cluster nobody has probed yet reads as its name rather than as a
        // name trailing a dot.
        mainHead={version ? `${name} · ${version}` : name}
        width={RAIL_WIDTH}
        rail={<AtAGlance context={context} overview={overview} />}
      >
        {/* The column of bands, scrolling inside itself.

            NO PADDING AND NO GAP, and both are the design rather than an
            oversight. The bands are direct siblings so `.section + .section`
            rules between them; a gap would put daylight where the design has
            one hairline, and a pad would inset every band inside a second
            margin the rail beside it does not have. */}
        <div className="scroll flex min-h-0 flex-1 flex-col">
          <Stale overview={overview} />
          <Capacity overview={overview} />
          <Nodes context={name} nodes={overview.nodes} />
          <NotReady context={name} overview={overview} />
        </div>
      </SideRail>
    </Screen>
  );
}

/**
 * The one line that says the figures below it have stopped refreshing.
 *
 * Every loader on this screen keeps its last good answer in memory, so coming
 * back to the tab paints the cluster instantly instead of spending a whole
 * round of requests to arrive at the same numbers. The cost of that is a state
 * the screen did not have before: **real rows, no longer being updated.**
 *
 * That state is not an error — throwing away the last good answer loses
 * information nobody asked to lose, and an `ErrorState` where a cluster used
 * to be is worse than the cluster as it was a minute ago. It is not health
 * either. So it gets a sentence, and the sentence is the whole point: figures
 * that quietly stopped refreshing are the same lie as a `0` in place of "no
 * reading", and this screen already refuses that one.
 *
 * **Said once, at the top.** A stale nodes list and a stale namespace count
 * are one outage, and five bands each announcing it has said it five times and
 * explained it nowhere — the rule `metricsServerRow` follows for a missing
 * metrics-server, applied to the same problem.
 */
function Stale({ overview }: { overview: OverviewData }) {
  const stale = summarise(overview.staleReasons);
  if (overview.staleReasons.length === 0) return null;
  return (
    <div className="p-3 pb-0">
      {/* The sentence is the deliberate part and stays exactly as written; the
          reasons under it are classified, and deduplicated, because one
          expired token stops all seven loaders and is one outage. */}
      <Alert tone="warn" title="Showing the last reading — this is no longer refreshing">
        {stale.detail}
        <RawError text={stale.raw ?? ""} className="mt-1" />
      </Alert>
    </div>
  );
}

/* -------------------------------------------------------------- the rail */

/**
 * `At a glance` — the facts about the cluster that are not measurements.
 *
 * Four `Section`s and nothing around them. `SideRail` drops what it is handed
 * straight into one box and `.section + .section` is what rules between them,
 * so a wrapper per child would break that adjacency and the rail would read as
 * one undivided block. `AboutKind` renders its sections the same way and for
 * the same reason; the kit's suite pins it.
 */
function AtAGlance({ context, overview }: { context: ClusterContext; overview: OverviewData }) {
  const contexts = useContexts();
  const { workspace } = useTabs();

  // The workspace's own order, resolved through the context list — the same
  // derivation the cluster rail down the edge of the window makes. An id whose
  // context has gone is skipped rather than drawn as a placeholder; `Fleet`
  // then puts this cluster back at the head if the list has lost it.
  const byId = new Map(contexts.map((c) => [c.stableId, c]));
  const clusters = workspace.clusters
    .map((id) => byId.get(id))
    .filter((c): c is ClusterContext => c !== undefined);

  return (
    <>
      <ControlPlane context={context} facts={overview.facts} />
      <ObjectsByKind context={context.name} objects={overview.objects} />
      <OpenIncidents />
      <Section title="Fleet" smallCaps>
        <Fleet clusters={clusters} active={context} />
      </Section>
    </>
  );
}

/**
 * What the metrics server row says, or `null` for no row at all.
 *
 * **`v1beta1` is the API GROUP's version, and the mock's `v0.7.2` is the
 * component's.** They are different facts. The component version lives in the
 * image tag of a Deployment in `kube-system`, which is a read many readers of
 * this screen are not granted and which would fail differently from everything
 * around it; that trade was ruled not worth making. Discovery already carries
 * the group version, it costs nothing, and it is the version that says what
 * this app will actually be talking to.
 *
 * **The mock's `· reporting` is not rendered off this field.** An aggregated
 * APIService stays registered in discovery while the deployment behind it is
 * down, so `present` means "the group is registered", not "metrics-server is
 * answering". The tiles and the node columns are where a missing ANSWER shows
 * up, as "No reading".
 *
 * `unknown` gets no row. It is not `absent`: a cluster nobody could reach has
 * not told us metrics-server is missing, and drawing that as an absence would
 * be the rail inventing the one fact it is here to report.
 */
function metricsServerRow(fact: MetricsServerFact | undefined): ReactNode {
  if (!fact) return null;
  if (fact.state === "absent") {
    return (
      <>
        Not installed
        {/* Said once, here. The tiles and the node columns read "No reading"
            and deliberately do not each explain why — a screen where five
            tiles and two columns announce this has said it seven times and
            explained it nowhere. */}
        <span className="block text-faint">No CPU or memory readings without it.</span>
      </>
    );
  }
  if (fact.state === "present") return fact.version || "Installed";
  return null;
}

/**
 * `Control plane` — six facts about the cluster, of which as many are drawn as
 * the cluster actually reported.
 *
 * **A FACT WITH NOTHING BEHIND IT OMITS ITS ROW.** `provider` and `region`
 * arrive as empty strings when no node carried them, and on the live cluster
 * no node carries a region label at all — so this is the ordinary case rather
 * than an edge one. "Unknown" in the value would look like an answer, and an
 * em dash reads as one too; silence is what the cluster said.
 *
 * The version and the connection come from the probe the shell already ran at
 * launch, not from a call of this screen's own: `clusterInfo` runs for every
 * cluster in the rail on every launch, and asking again here would be a second
 * round trip for a fact already on the machine. Both are absent until it lands,
 * and absent is honest — nobody has asked yet, which is not the same as
 * "Disconnected".
 */
function ControlPlane({ context, facts }: { context: ClusterContext; facts: OverviewFacts }) {
  const info = useInfo(context.stableId);
  const { links } = useWorkspaceView();
  const link = links[context.stableId];

  const rows: Array<[key: string, value: ReactNode]> = [];
  if (info?.version) rows.push(["Version", info.version]);
  if (facts.facts?.provider) rows.push(["Provider", facts.facts.provider]);
  if (facts.facts?.region) rows.push(["Region", facts.facts.region]);
  // The one row that is always there: the context is how this screen was
  // opened, so there is no cluster for which it is unknown.
  rows.push(["Context", context.name]);
  // The status bar's own word for the same link, from the one table both read.
  if (link) rows.push(["Connection", LINK_WORD[link.state]]);
  const metrics = metricsServerRow(facts.facts?.metricsServer);
  if (metrics) rows.push(["Metrics server", metrics]);

  return (
    <Section title="Control plane" smallCaps>
      <KVList rows={rows} />
      {/* The facts call itself failing is not the same as a cluster that named
          none, and the rows above cannot tell the reader which happened. */}
      {facts.error && (
        <FailureWord
          error={facts.error}
          lead="Could not read the cluster's facts: "
          className="text-faint"
        />
      )}
    </Section>
  );
}

/**
 * `Objects by kind` — six counts, each a way into that kind's list.
 *
 * A button per row rather than a `KV`: §7 draws the counts as a navigation
 * list, and the accessible name is computed from the words already on screen
 * ("Deployment 25") rather than from a second string that can drift from
 * them. `ReasonRail` settled the same shape for the events rail.
 *
 * **The label is the KIND, singular — `K8S_KIND`, not `RESOURCE_LABELS`.**
 * The sidebar's plurals name a list you are about to open; a row here names
 * the kind the number counts, and the design writes `Deployment 25`. The two
 * tables already exist in core beside each other, so this is a choice of which
 * question is being answered rather than a table of this screen's own.
 *
 * **A kind that could not be counted shows no number and says so on its own
 * row.** `0` is a number a reader will believe, and a refused list and an
 * empty cluster are the same picture with opposite meanings. The em dash asks
 * the question, and the reason sits under the row that could not answer —
 * beside it, the way `Fleet` puts an unreachable cluster's reason on that
 * cluster's row, rather than as a paragraph under the section naming kinds a
 * second time.
 */
function ObjectsByKind({ context, objects }: { context: string; objects: ObjectCounts }) {
  return (
    <Section title="Objects by kind" smallCaps padded={false}>
      {objects.counts.map(({ slug, count, error }) => (
        <div key={slug}>
          <Button
            type="button"
            variant="ghost"
            // `.ns-row` is the design's flat row and follows `.btn` in the
            // stylesheet, so the row's padding, width and alignment win while
            // `ghost` keeps the border off — see `ReasonRail`.
            // `px-3` rather than `.ns-row`'s own 0.5rem: the band is unpadded,
            // so the row itself has to hold the 0.75rem inset that lines this
            // label up with the `Control plane` keys above it. A utility beats
            // a components-layer class by layer order, not by source order, so
            // this is decided rather than a coin flip.
            className="ns-row rounded px-3 font-normal"
            onClick={() => openTab(`/k/${slug}`, { clusterName: context })}
          >
            <span className="flex min-w-0 flex-1 truncate">{K8S_KIND[slug]}</span>
            <span className="path text-faint">{count === null ? UNKNOWN : count}</span>
          </Button>
          {error && (
            // The row is as wide as the rail, which is to say not wide enough
            // for a paragraph — the headline, and the original a click away.
            <FailureWord
              error={error}
              lead={`Could not count ${K8S_KIND[slug]}: `}
              className="path px-3 pb-1 text-faint"
            />
          )}
        </div>
      ))}
    </Section>
  );
}

/**
 * `Open incidents` — a named empty state, and the reason it is empty.
 *
 * The mock draws three incidents with titles, severity badges and sparklines.
 * **No Kubernetes API returns any of that.** There is no object whose title is
 * "5xx rate rising", no field that says SEV-2, and no series behind the
 * sparkline; incidents are a product feature the migration plan schedules on
 * its own, not something this screen could derive from the cluster if it tried
 * harder. Events come closest and are a different thing — they are per-object,
 * they expire in an hour, and a list of them is already a screen.
 *
 * So the section says what it is. A hole where the mock has content is read as
 * a bug and reported as one; a stated absence is read as a decision.
 */
function OpenIncidents() {
  return (
    <Section title="Open incidents" smallCaps padded={false}>
      {/* The rail-sized form. The page-sized one spends `py-10` on padding
          around three wrapped lines, and in a 286px rail that is enough to
          push `Fleet` below the fold — a section stating an absence taking
          the space from one with something to say. */}
      <EmptyState
        compact
        title="No incident feed yet"
        hint="Kubernetes reports no incident's title, severity or trend. srelens will grow its own."
      />
    </Section>
  );
}

/* ---------------------------------------------------------------- capacity */

/** A tile's figure, and the caption that carries the tone. */
interface Tile {
  value: string;
  caption?: string;
  tone?: Tone;
}

/**
 * The five figures across the top: Nodes, Pods, Namespaces, CPU, Memory.
 *
 * **The caption carries the tone, never the figure.** `Stat` spends its tone
 * on the delta alone for the same reason: a row of five coloured numbers shows
 * the reader nothing about which one to look at, and the judgement — `all
 * ready`, `8 not ready`, `312 / 460 cores` — is what the colour belongs to.
 *
 * Every section reads its own loader, so a refused namespace list empties one
 * tile and leaves the other four with their numbers.
 */
function Capacity({ overview }: { overview: OverviewData }) {
  const nodes = nodesTile(overview.nodes);
  const pods = podsTile(overview.pods);
  const namespaces = overview.namespaces.count;
  const cpu = cpuTile(overview.nodes.capacity);
  const memory = memoryTile(overview.nodes.capacity);

  return (
    <Section title="Capacity" smallCaps padded={false}>
      {/* The row sizes the tiles rather than each tile sizing itself: `Stat`
          cannot be given a width through `className` (two utilities that both
          set `flex` resolve by stylesheet order), so the grid does it.

          NO GAP. `.stat + .stat` draws a hairline between the tiles, which is
          the design's divider; a gap would put daylight either side of it and
          turn one strip into five cells. */}
      <div data-slot="capacity" className="grid grid-cols-5">
        <Stat label="Nodes" value={nodes.value} delta={nodes.caption} tone={nodes.tone} />
        <Stat label="Pods" value={pods.value} delta={pods.caption} tone={pods.tone} />
        {/* The one tile the design gives no caption: a namespace count has no
            judgement attached to it. */}
        <Stat label="Namespaces" value={namespaces === null ? NO_READING : String(namespaces)} />
        <Stat label="CPU" value={cpu.value} delta={cpu.caption} tone={cpu.tone} />
        <Stat label="Memory" value={memory.value} delta={memory.caption} tone={memory.tone} />
      </div>
    </Section>
  );
}

/**
 * How many nodes there are, and what is the matter with them.
 *
 * The partition and the tone both come from core's `nodeStatus` (through
 * `nodeVerdict`), never from a word this file pairs with a colour: a NotReady
 * node is `danger` there and a cordoned one is `warning`, and calling a
 * cordoned node "not ready" would be a second, wronger reading of a verdict
 * core already made. `statusTone` maps the health to the kit's token, and is
 * exported precisely so nobody keeps a private copy of that map.
 */
function nodesTile(nodes: OverviewNodes): Tile {
  // No rows AND no answer is no reading. Rows plus a failed refresh is the
  // last good reading, and `Stale` says at the top that it stopped
  // refreshing — blanking a real figure here would lose it and explain
  // nothing. An answer of no nodes at all is still an answer, and reads `0`.
  if (nodes.nodes.length === 0 && (nodes.status === "loading" || nodes.error)) {
    return { value: NO_READING };
  }

  const verdicts = nodes.nodes.map((row) => nodeVerdict(row.node));
  if (verdicts.length === 0) return { value: "0" };

  const value = String(verdicts.length);
  const notReady = verdicts.filter((v) => v.health === "danger").length;
  if (notReady > 0) return { value, caption: `${notReady} not ready`, tone: statusTone("danger") };

  const cordoned = verdicts.filter((v) => v.flagged).length;
  if (cordoned > 0) return { value, caption: `${cordoned} cordoned`, tone: statusTone("warning") };

  return { value, caption: "all ready", tone: statusTone("success") };
}

/**
 * How many pods there are, and how many need a second look.
 *
 * The figure is a COUNT the backend made server-side, not the length of a list
 * this screen fetched — `podOverview` counts 5 416 pods without shipping one of
 * them. A `total` of `null` means nobody counted, which is not an empty
 * cluster; `0` here is the cluster's own answer.
 *
 * `podFlagged` is core's `podStatus` — the same reading the pod list's dot and
 * the pod detail's header take, so a pod counted here as unhealthy is the one
 * the `Not ready` list will name. It reads `unsettled`, which holds every pod
 * that is not simply running, so nothing core would flag is missing from it.
 *
 * **Unless the backend capped it.** A capped list can only say how many it
 * FOUND, so the caption says "at least" — and "all ready" is withheld
 * entirely, because a clean bill of health is a claim about every pod and a
 * capped list has not seen every pod.
 */
function podsTile(pods: OverviewPods): Tile {
  if (pods.total === null) return { value: NO_READING };
  const value = String(pods.total);
  if (pods.total === 0) return { value };

  const flagged = (pods.unsettled ?? []).filter(podFlagged);
  if (flagged.length === 0) {
    return pods.truncated ? { value } : { value, caption: "all ready", tone: statusTone("success") };
  }
  const count = pods.truncated ? `at least ${flagged.length}` : String(flagged.length);
  return { value, caption: `${count} not ready`, tone: statusTone(worst(flagged)) };
}

/**
 * The worst health among the pods that are flagged — what tones their count.
 *
 * Read off the same `podStatus` that flagged them: a Pending pod is amber and
 * a crash-looping one is red, and a count that mixed them takes the redder of
 * the two rather than a colour this file chose.
 */
function worst(flagged: PodSummary[]): HealthKind {
  return flagged.some((pod) => podStatus(pod).health === "danger")
    ? "danger"
    : "warning";
}

function cpuTile(capacity: ClusterCapacity): Tile {
  const cpu = capacity.cpu;
  if (!cpu || cpu.allocatableMillicores === 0) return { value: NO_READING };
  const percent = (cpu.usedMillicores / cpu.allocatableMillicores) * 100;
  return {
    value: `${Math.round(percent)}%`,
    caption: `${cores(cpu.usedMillicores)} / ${cores(cpu.allocatableMillicores)} cores${partial(capacity)}`,
    tone: loadTone(percent),
  };
}

function memoryTile(capacity: ClusterCapacity): Tile {
  const memory = capacity.memory;
  if (!memory || memory.allocatableMiB === 0) return { value: NO_READING };
  const percent = (memory.usedMiB / memory.allocatableMiB) * 100;
  return {
    value: `${Math.round(percent)}%`,
    caption: `${mib(memory.usedMiB)} / ${mib(memory.allocatableMiB)}${partial(capacity)}`,
    tone: loadTone(percent),
  };
}

/**
 * What qualifies a total that is not the whole cluster's.
 *
 * `clusterCapacity` sums only the nodes that reported a metric — a node with
 * no reading is left out of both halves of the ratio rather than folded in as
 * an idle one — so whenever `nodesReporting` falls short of `nodesTotal` the
 * figure above describes part of the cluster. It carries the shortfall on the
 * return value for exactly this: a partial total shown bare reads as a whole
 * one, and nothing else on screen would contradict it.
 */
function partial(capacity: ClusterCapacity): string {
  if (capacity.nodesReporting >= capacity.nodesTotal) return "";
  return ` · ${capacity.nodesReporting} of ${capacity.nodesTotal} nodes reporting`;
}

/** Millicores as cores, to one decimal place and no trailing zero: `8.4`, `12`. */
function cores(millicores: number): string {
  return String(Math.round(millicores / 100) / 10);
}

/** MiB through core's own binary-size formatter, so `35.2Gi` reads as it does elsewhere. */
function mib(value: number): string {
  return formatStorageSize(`${value}Mi`);
}

/* ------------------------------------------------------------------- nodes */

/**
 * How many node rows this band draws before it stops.
 *
 * **A dashboard section is not an inventory.** The design's frame shows five
 * rows; a live cluster the user ran this against has 113, and every one of
 * them rendered — which pushed `Not ready`, the section that says what is
 * actually wrong, entirely off the screen. `/k/nodes` is the screen for the
 * whole list, and this one is a summary of it.
 *
 * Ten rather than five: five is the frame's own count on a 42-node cluster,
 * and a summary that shows fewer rows than the design drew is its own
 * regression on the ordinary case.
 */
const NODE_ROWS = 10;

/**
 * The order a summary of nodes has to be in: the ones worth looking at first.
 *
 * Alphabetical is what a list of 113 nodes arrives in, and the first ten of
 * those tell a reader nothing — the cordoned node and the one at 94% are as
 * likely to be at the end as anywhere. So the band sorts before it caps, and
 * what it sorts by is:
 *
 * 1. **core's verdict**, through the same `nodeVerdict` the State column
 *    draws — a NotReady node above a cordoned one above a healthy one. Not a
 *    predicate of this file's: `SEVERITY` is the `Not ready` list's own order
 *    over `HealthKind`, reused rather than restated.
 * 2. **the hotter of its two readings**, so among nodes core is content with,
 *    the one at 94% CPU comes before the one at 8%. A node with NO reading
 *    sorts as `-1` — below every measured node rather than among the idle
 *    ones — which is the rule the CPU column already sorts by.
 * 3. **the name**, so the order is stable between renders.
 *
 * The reader can still sort the table by any column; that reorders the rows
 * this chose, and the caption underneath says how many were chosen out of how
 * many there are.
 */
function worstFirst(a: NodeRow, b: NodeRow): number {
  const health = SEVERITY[nodeVerdict(a).health] - SEVERITY[nodeVerdict(b).health];
  if (health !== 0) return health;
  const load = hottest(b) - hottest(a);
  if (load !== 0) return load;
  return a.name.localeCompare(b.name);
}

/** The higher of a node's two readings, or `-1` when it reported neither. */
function hottest(row: NodeRow): number {
  return Math.max(row.usage.cpuPercent ?? -1, row.usage.memoryPercent ?? -1);
}

function Nodes({ context, nodes }: { context: string; nodes: OverviewNodes }) {
  const [pending, setPending] = useState<Pending | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const clipboard = useClipboardCopy();

  const all: NodeRow[] = nodes.nodes.map(({ node, usage }) => ({ ...node, usage }));
  // Sorted on a copy: `nodes.nodes` is the loader's array and shared with the
  // capacity tiles, which count off it.
  const rows = [...all].sort(worstFirst).slice(0, NODE_ROWS);

  /**
   * The divergence banner, its acknowledgement and the refusal behind it.
   *
   * `pending.context` is the cluster the action runs against; `context` is
   * what the reader has in FOCUS, which is the only thing a rail switch may
   * change. See `lib/clusterMoved` for why the gate re-arms here rather than
   * only stating the divergence: a drain confirm's whole input is one click,
   * so asking again costs nothing.
   */
  const gate = useClusterGate({
    pinned: pending?.context ?? null,
    live: context,
    verb: pending ? (pending.type === "drain" ? "drain" : pending.unschedulable ? "cordon" : "uncordon") : "act",
  });
  const reset = gate.reset;

  // Stable, so the column set below is built once per context rather than per
  // render — `Table` re-sorts whenever its `columns` identity changes. The
  // cluster each action was picked on rides IN the pending record (see
  // `Pending`), captured by the row's own action rather than read here.
  const open = useCallback(
    (next: Pending) => {
      setError("");
      reset();
      setPending(next);
    },
    [reset],
  );
  const columns = useMemo(
    () => nodeColumns(context, open, clipboard),
    [context, open, clipboard.feedback, clipboard.statusFor, clipboard.write],
  );

  function close() {
    setPending(null);
    setError("");
    gate.reset();
  }

  /**
   * The action itself, taken only from the confirm.
   *
   * Nothing in `actions` below calls core: every pick opens the dialog, and
   * only this runs. That is what makes "no node is cordoned or drained
   * without a confirm" true by construction rather than by each button
   * remembering — the same split `ResourceMenu`'s `pending` makes for the row
   * menu's destructive entries, and the reason `ConfirmDialog` and
   * `KubectlPreview` are the kit's rather than this screen's own.
   */
  async function confirm() {
    if (!pending) return;
    // Asked before anything else: it is the only question on screen whose
    // answer changes which machine the name below refers to. The write still
    // goes to `pending.context` either way — this re-arms the confirmation, it
    // does not retarget it.
    if (gate.refusal) {
      setError(gate.refusal);
      return;
    }

    setBusy(true);
    setError("");

    if (pending.type === "drain") {
      // `pending.context`, never the live prop. See {@link Pending}.
      const out = await drainNode(pending.context, pending.name);
      setBusy(false);
      // A refused write leaves the dialog up with the reason in it, rather
      // than closing as if the node had been drained.
      if (out.error) {
        setError(out.error);
        return;
      }
      notify.success(`Drained ${pending.name}`, `${out.evicted ?? 0} evicted, ${out.skipped ?? 0} skipped`);
    } else {
      const out = await cordonNode(pending.context, pending.name, pending.unschedulable);
      setBusy(false);
      if (out.error) {
        setError(out.error);
        return;
      }
      notify.success(`${pending.unschedulable ? "Cordoned" : "Uncordoned"} ${pending.name}`);
    }

    close();
    // The node's own `unschedulable` has changed; the table is what shows it.
    nodes.reload();
  }

  return (
    <Section title="Nodes" smallCaps padded={false}>
      {nodes.status === "loading" && all.length === 0 ? (
        <LoadingState label="Loading nodes" />
      ) : nodes.error && all.length === 0 ? (
        // The node list, and only it. A missing metrics-server is held apart
        // by the loader (`metricsError`) precisely so it cannot empty this
        // table; those rows keep their columns and read as no reading.
        //
        // And only when there is nothing to show. A refused REFRESH over rows
        // already on screen keeps the rows: the screen says once, at the top,
        // that they stopped refreshing, and an `ErrorState` where a cluster
        // used to be tells the reader strictly less.
        <FailureState
          title={`Could not list nodes on ${context}`}
          error={nodes.error}
          onRetry={nodes.reload}
        />
      ) : (
        <>
          <Table
            columns={columns}
            data={rows}
            getRowKey={(row) => row.name}
            emptyText="No nodes"
            emptyHint={`Nothing in ${context} reported a node.`}
          />
          {/* A cap the reader cannot see is a lie by omission: the band would
              look like the whole cluster. It says what it is showing, what it
              chose them by, and offers the screen that has the rest. */}
          {all.length > rows.length && (
            <Button
              type="button"
              variant="ghost"
              className="ns-row rounded px-3 font-normal"
              onClick={() => openTab("/k/nodes", { clusterName: context })}
            >
              <span className="flex min-w-0 flex-1 truncate text-faint">
                {`Showing ${rows.length} of ${all.length} nodes, worst first`}
              </span>
              <span className="path">All nodes</span>
            </Button>
          )}
        </>
      )}
      {pending && (
        <NodeConfirm
          pending={pending}
          // Captured, not current: the kubectl line under the message names
          // the cluster the write will actually reach.
          context={pending.context}
          moved={gate.alert}
          busy={busy}
          error={error}
          onConfirm={() => void confirm()}
          onCancel={close}
        />
      )}
      <ClipboardCopyStatus feedback={clipboard.feedback} />
    </Section>
  );
}

/**
 * The node's pool — the machine type it was created from.
 *
 * `NodeSummary.instanceType` is the backend's reading of
 * `node.kubernetes.io/instance-type` (falling back to the deprecated
 * `beta.kubernetes.io/instance-type`), and an empty string when the node
 * carries neither. Empty stays empty here and renders as the em dash every
 * other absent fact on this screen uses: the node said nothing, and a word in
 * its place would look like an answer.
 *
 * Deliberately NOT filled from something else that happens to be present. The
 * mock's pools (`c3-standard` out of `eu-w4-c3-standard-a1`) are a naming
 * convention, and `roles` is a different fact entirely; either guess would be
 * read as the machine type the cluster is billed for. kind's nodes are
 * containers rather than cloud machines and carry neither label, so an
 * em-dashed column there is the correct answer rather than a missing one.
 */
function pool(node: NodeRow): string {
  return node.instanceType;
}

/**
 * How much room a reading gets, whether or not there is one yet.
 *
 * `Meter`'s bar is `w-full`, which has no intrinsic width of its own, so a
 * table cell sized by its content collapsed the column onto the percentage
 * beside it and drew a 40px stub. At that size the tone is the one thing the
 * column exists to show and the one thing nobody can see — 88% red and 41%
 * green are two short marks. The design gives CPU and MEMORY a wide column
 * each; ten rems is as much as this table can spare before the row scrolls
 * sideways.
 *
 * **Declared on every cell, including the ones with no reading.** `Table`
 * measures the natural column widths on the first render that has rows and
 * PINS them (`table-layout: fixed` from then on), and the node list answers
 * well before the metrics do — so the first render is all "No reading", and a
 * width that arrived with the meters arrived after the columns had already
 * been fixed at the width of those two words. The meters then drew straight
 * across the columns beside them. Whatever the cell is going to hold, it asks
 * for the same room from the start.
 *
 * On the CONTENT rather than on the column because that is what a table's auto
 * layout reads: `Column.minWidth` is the floor a drag stops at, and it is not
 * consulted when the browser sizes the columns.
 */
const READING_WIDTH = "min-w-[10rem]";

/** The percentage a meter draws, or the words that say there is none. */
function reading(percent: number | null, ariaLabel: string) {
  return (
    <div className={READING_WIDTH}>
      {percent === null ? (
        NO_READING
      ) : (
        // Straight through, unrounded and uncapped. `Meter` clamps the bar it
        // draws while keeping `aria-valuetext` truthful and rounds only what
        // it shows, so rounding or clamping here would make a node at 140%
        // indistinguishable from one exactly at its limit — hiding the case
        // worth seeing.
        <Meter value={percent} ariaLabel={ariaLabel} />
      )}
    </div>
  );
}

/** `31/50`, or no reading — including for a node that reported no capacity. */
function podsRead(pods: NodeUsage["pods"]): string {
  // `{ used: 31, allocatable: 0 }` is a node that reported no allocatable pod
  // capacity. `31/0` is not a ratio, and it reads as a node overrun rather
  // than as a denominator nobody supplied.
  if (!pods || pods.allocatable === 0) return NO_READING;
  return `${pods.used}/${pods.allocatable}`;
}

function nodeColumns(
  context: string,
  open: (pending: Pending) => void,
  clipboard: ClipboardCopyController,
): Column<NodeRow>[] {
  return [
    {
      key: "name",
      header: "Name",
      sortable: true,
      render: (row) => (
        <span className="flex items-center gap-1.5">
          {nodeVerdict(row).flagged ? (
            <UnhealthyDot />
          ) : (
            // The dot's width, kept so a healthy node's name lines up under a
            // flagged one's rather than sliding left.
            <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0" />
          )}
          <span className="truncate">{row.name}</span>
        </span>
      ),
    },
    { key: "pool", header: "Pool", sortable: true, getValue: pool, render: (row) => pool(row) || UNKNOWN },
    {
      key: "state",
      header: "State",
      sortable: true,
      getValue: (row) => nodeVerdict(row).status,
      // The word AND the tone are core's `nodeStatus`, through the same
      // `nodeVerdict` the nodes list reads — including the cordoned case,
      // which it spells the way kubectl does. `tinted` is the design's
      // asymmetry: the word is coloured only when core called the state bad.
      render: (row) => {
        const verdict = nodeVerdict(row);
        return <StatusPill status={verdict.status} kind={verdict.health} tinted />;
      },
    },
    {
      key: "cpu",
      header: "CPU",
      sortable: true,
      // `-1` puts a node with no reading below every real one, rather than in
      // among the idle ones — the same rule the nodes list sorts metrics by.
      getSortValue: (row) => row.usage.cpuPercent ?? -1,
      render: (row) => reading(row.usage.cpuPercent, `${row.name} CPU`),
    },
    {
      key: "memory",
      header: "Memory",
      sortable: true,
      getSortValue: (row) => row.usage.memoryPercent ?? -1,
      render: (row) => reading(row.usage.memoryPercent, `${row.name} memory`),
    },
    {
      key: "pods",
      header: "Pods",
      sortable: true,
      align: "end",
      getSortValue: (row) => row.usage.pods?.used ?? -1,
      render: (row) => podsRead(row.usage.pods),
    },
    {
      key: "actions",
      header: "",
      sortable: false,
      filterable: false,
      render: (row) => (
        <ActionBar
          actions={nodeActions(context, row, open, clipboard)}
          label={`Actions for ${row.name}`}
          max={2}
        />
      ),
    },
  ];
}

/**
 * The row's actions: two on the bar and the rest behind the overflow, which is
 * the design's own shape for this table.
 *
 * A cordoned node is offered the other direction rather than the same action
 * again — the state is on the row, so the button can say what it will do.
 * `Node shell` is in the design's overflow and is not here: it needs the
 * ephemeral debug-pod flow, which this screen has no path to, and an action
 * that cannot work is worse than one that is absent.
 */
function nodeActions(
  context: string,
  row: NodeRow,
  open: (pending: Pending) => void,
  clipboard: ClipboardCopyController,
): ActionBarAction[] {
  const kubectlBase = { kind: "Node", name: row.name, namespace: null, context } as const;
  const copyKey = `${context}/Node/${row.name}`;
  const copyStatus = clipboard.statusFor(copyKey);
  return [
    row.unschedulable
      ? {
          id: "uncordon",
          label: NODE_ACTION_LABEL.uncordon,
          // The same glyph as `Cordon`, because it is the same switch: the
          // label is what says which way it is being thrown, and a second
          // picture for the reverse of one action is a second thing to read.
          icon: Icons.cordon,
          onSelect: () => open({ type: "cordon", name: row.name, unschedulable: false, context }),
        }
      : {
          id: "cordon",
          // The design's crossed circle — nothing new gets scheduled here.
          label: NODE_ACTION_LABEL.cordon,
          icon: Icons.cordon,
          onSelect: () => open({ type: "cordon", name: row.name, unschedulable: true, context }),
        },
    {
      id: "drain",
      label: NODE_ACTION_LABEL.drain,
      // The design's wave — everything flows off the node.
      icon: Icons.drain,
      // Danger-toned because it is: every pod on the node is evicted.
      danger: true,
      onSelect: () => open({ type: "drain", name: row.name, context }),
    },
    {
      id: "open",
      label: ROW_ACTION_LABEL.openTab,
      onSelect: () => openTab(detailRoute("Node", null, row.name), { clusterName: context }),
    },
    {
      id: "copy",
      label:
        copyStatus === "copied"
          ? "Copied"
          : copyStatus === "failed"
            ? "Copy failed"
            : ROW_ACTION_LABEL.copy,
      icon: copyStatus === "copied" ? Icons.check : copyStatus === "failed" ? Icons.warn : Icons.copy,
      closeOnSelect: false,
      onSelect: () =>
        void clipboard.write(
          copyKey,
          toKubectl({ ...kubectlBase, action: "get", output: "yaml" }),
        ),
    },
  ];
}

/* --------------------------------------------------------------- not ready */

/**
 * How bad, as an order.
 *
 * Emphatically NOT a ninth hand-paired label/tone table: no word and no colour
 * is named here. Core has already decided both, and this only says which of
 * core's verdicts an operator wants to read first — red before amber, and
 * amber before a state nobody recognised.
 *
 * Total over `HealthKind` rather than over the three flagged tones, so a tone
 * core starts flagging tomorrow sorts somewhere defined instead of `NaN`.
 */
const SEVERITY: Record<HealthKind, number> = {
  danger: 0,
  warning: 1,
  neutral: 2,
  info: 3,
  success: 4,
};

/** One unhealthy thing, whatever kind it is. */
interface NotReadyRow {
  /** The Kubernetes kind, as `detailRoute` spells it. */
  kind: string;
  name: string;
  namespace: string;
  /** The word, the tone and the dot — core's, decided together. */
  verdict: StatusVerdict;
  /** Ready out of desired, as the row itself reports it: `9/12`. */
  ratio: string;
}

/**
 * The trailing facts, each carrying the noun that says what it is.
 *
 * `StatusRow` takes `ReactNode`s and genuinely cannot know what a fact means;
 * naming them is the caller's job and nobody else's. The design draws the
 * bare figures of the mock — `checkout   9/12` — which reads to a screen
 * reader as a row called "Degraded checkout-api checkout 9/12": two values,
 * no nouns, and no way to tell a namespace from a ratio. `Inspector` settled
 * the same argument the same way, and for the same reason its facts read
 * "9/12 ready" rather than "9/12".
 */
function facts(row: NotReadyRow): string[] {
  return [`namespace ${row.namespace}`, `${row.ratio} ready`];
}

/**
 * Every unhealthy workload and pod, worst first.
 *
 * Two rules, and the section is the two of them:
 *
 * **Which rows qualify is core's `flagged`, never a predicate of this file's.**
 * The verdicts come off `deploymentVerdict`/`statefulSetVerdict`/
 * `daemonSetVerdict`/`podStatus`, which are the same `scaledStatus` and
 * `podStatus` that `resourceStatusLine` calls for a fetched object — so a row
 * here and the detail pane a click away cannot disagree about the same thing.
 * Badness is read as data, not off the colour: a Deployment scaled to zero is
 * neutral and absent, a Pending pod is amber and present, and a pod in a phase
 * core does not recognise is grey and present, because not recognising a state
 * is not the same as knowing it is fine.
 *
 * **The order is severity, not kind.** A list that put every Deployment above
 * every Pod would bury a crash-looping pod under four healthy-ish rollouts,
 * and the whole point of the section is that the worst thing is at the top.
 * Ties break on the name — the three workload lists and the pod list settle
 * independently, so concatenation order is not stable between renders, and a
 * list that reshuffled itself under the reader's cursor would be its own bug.
 */
function notReadyRows(workloads: OverviewWorkloads, unsettled: PodSummary[] | undefined): NotReadyRow[] {
  const rows: NotReadyRow[] = [];

  for (const row of workloads.deployments ?? []) {
    rows.push({
      kind: "Deployment",
      name: row.name,
      namespace: row.namespace,
      verdict: deploymentVerdict(row),
      ratio: row.ready,
    });
  }
  for (const row of workloads.statefulSets ?? []) {
    rows.push({
      kind: "StatefulSet",
      name: row.name,
      namespace: row.namespace,
      verdict: statefulSetVerdict(row),
      ratio: row.ready,
    });
  }
  for (const row of workloads.daemonSets ?? []) {
    rows.push({
      kind: "DaemonSet",
      name: row.name,
      namespace: row.namespace,
      verdict: daemonSetVerdict(row),
      // A DaemonSet reports two numbers where a Deployment reports the string.
      ratio: `${row.ready}/${row.desired}`,
    });
  }
  // Every pod that is not simply running — `Succeeded` ones included, which
  // core then leaves unflagged. The filter below is the only thing that
  // decides, and it is core's.
  for (const row of unsettled ?? []) {
    rows.push({
      kind: "Pod",
      name: row.name,
      namespace: row.namespace,
      verdict: podStatus(row),
      ratio: row.ready,
    });
  }

  return rows
    .filter((row) => row.verdict.flagged)
    .sort(
      (a, b) =>
        SEVERITY[a.verdict.health] - SEVERITY[b.verdict.health] || a.name.localeCompare(b.name),
    );
}

/**
 * `NOT READY` — the unhealthy workloads and pods, in one list.
 *
 * It reads two loaders and adds no fetch of its own: the pod list is the one
 * the Pods tile and the per-node counts already share.
 *
 * A refusal never reads as good news. "Nothing is unhealthy" is a claim about
 * everything that was checked, so a kind that could not be listed is said out
 * loud — beside the rows when some kinds answered, and in place of them when
 * none did. Silently showing three rows out of a possible six, or an empty
 * state for a cluster nobody was allowed to look at, is the failure this
 * section would be worst at.
 */
function NotReady({ context, overview }: { context: string; overview: OverviewData }) {
  const { workloads, pods } = overview;
  const rows = notReadyRows(workloads, pods.unsettled);

  // `workloads.error` is the whole fan-out failing; `workloads.refusals` is
  // one kind of it. Both are reasons this list may be shorter than the truth.
  //
  // A reason belongs here only when the thing it explains is ACTUALLY MISSING.
  // A failed refresh over rows this section already has is a different fact —
  // the rows are real, they are just not current — and `Stale` states that
  // once at the top of the screen. Repeating it here would be the same outage
  // announced twice and explained neither time.
  const failures = [
    pods.unsettled === undefined ? pods.error : undefined,
    workloads.deployments === undefined ? workloads.error : undefined,
    ...Object.values(workloads.refusals),
  ].filter((reason): reason is string => reason !== undefined && reason !== "");
  // Six kinds refused by one expired credential is one sentence, not six.
  const refusals = summarise(failures);

  const reload = () => {
    workloads.reload();
    pods.reload();
  };

  return (
    <Section title="Not ready" smallCaps padded={false}>
      {workloads.status === "loading" || pods.status === "loading" ? (
        <LoadingState label="Checking workloads and pods" />
      ) : failures.length > 0 && rows.length === 0 ? (
        <ErrorState
          title={`Could not check every workload on ${context}`}
          detail={refusals.detail}
          raw={refusals.raw}
          onRetry={reload}
        />
      ) : (
        <>
          {failures.length > 0 && (
            <Alert tone="warn" title="Some kinds could not be checked">
              {refusals.detail}
              <RawError text={refusals.raw ?? ""} className="mt-1" />
            </Alert>
          )}
          {/* A cap is not a failure, so it is not in `failures` — but it is
              the same kind of fact: the list is short, and a short list that
              does not say so is read as the whole truth. The backend stops
              fetching pod bodies past its cap rather than rebuilding the
              whole-cluster list this screen was rewritten to stop making. */}
          {pods.truncated && (
            <Alert tone="warn" title="More pods need a look than this list shows">
              This band is a summary, and it stops before the whole list. The pod list has all of
              them.
            </Alert>
          )}
          {rows.length === 0 && pods.truncated ? null : rows.length === 0 ? (
            <EmptyState
              title="Nothing is unhealthy"
              hint={`Every workload and pod in ${context} is where it should be.`}
            />
          ) : (
            <div className="flex flex-col">
              {rows.map((row) => (
                <StatusRow
                  // Kind and namespace as well as the name: a Deployment and
                  // its own pod share a prefix, and two namespaces may each
                  // run a `web`.
                  key={`${row.kind}/${row.namespace}/${row.name}`}
                  status={row.verdict.status}
                  kind={row.verdict.health}
                  // Data, not a reading of the tone — see `StatusRow`'s prop.
                  flagged={row.verdict.flagged}
                  name={row.name}
                  facts={facts(row)}
                  onActivate={() =>
                    openTab(detailRoute(row.kind, row.namespace, row.name), { clusterName: context })
                  }
                />
              ))}
            </div>
          )}
        </>
      )}
    </Section>
  );
}

/* ---------------------------------------------------------------- confirms */

/**
 * What a picked action is waiting to do, once the confirm is taken — and the
 * cluster it was picked ON.
 *
 * **`context` is captured when the reader picks the action, not when the
 * dialog renders.** Since #357 a dialog covers only its own tab, so the
 * cluster rail is live behind this one; `setActiveCluster` switches the active
 * cluster in place and nothing on this screen remounts, so `pending` survives
 * the switch while every prop around it becomes another cluster's. A drain
 * opened on production, confirmed after a rail click, would otherwise evict
 * every pod on staging's node of the same name. See `lib/clusterMoved`.
 */
type Pending = { context: string } & (
  | { type: "cordon"; name: string; unschedulable: boolean }
  | { type: "drain"; name: string }
);

function NodeConfirm({
  pending,
  context,
  moved,
  busy,
  error,
  onConfirm,
  onCancel,
}: {
  pending: Pending;
  /** The cluster this runs against — `pending.context`, never the live prop. */
  context: string;
  /** The cluster-moved banner and its tick, or `null` when the rail has not moved. */
  moved: ReactNode;
  busy: boolean;
  error: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const drain = pending.type === "drain";
  const label = drain
    ? NODE_ACTION_LABEL.drain
    : pending.unschedulable
      ? NODE_ACTION_LABEL.cordon
      : NODE_ACTION_LABEL.uncordon;
  const command = toKubectl({
    action: drain ? "drain" : pending.unschedulable ? "cordon" : "uncordon",
    kind: "Node",
    name: pending.name,
    namespace: null,
    context,
  });

  return (
    <ConfirmDialog
      title={`${label} node?`}
      // Cordoning is a reversible scheduling change with a button that undoes
      // it; draining evicts every pod on the node. Only one of the two is
      // destructive, and colouring both would say nothing about either.
      danger={drain}
      busy={busy}
      confirmLabel={label}
      onConfirm={onConfirm}
      onCancel={onCancel}
      message={
        <>
          {/* First, above the node's own name: it changes which machine that
              name refers to. */}
          {moved}
          <p style={{ marginTop: 0 }}>
            {drain ? (
              <>
                Drain <code>{pending.name}</code>? This evicts every pod on the node and stops new
                ones being scheduled to it.
              </>
            ) : pending.unschedulable ? (
              <>
                Cordon <code>{pending.name}</code>? No new pods will be scheduled to it; the pods
                already running there stay.
              </>
            ) : (
              <>
                Uncordon <code>{pending.name}</code>? It becomes schedulable again.
              </>
            )}
          </p>
          <KubectlPreview command={command} />
          {/* The dialog stays open on a refusal, so this is the whole of what
              the reader is told about why the action did not happen. */}
          {error && <FailureLine error={error} className="text-sev" />}
        </>
      }
    />
  );
}
