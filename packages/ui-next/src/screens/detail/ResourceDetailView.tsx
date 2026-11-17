import { ExtensionResourceSlot } from "../../extensions/Extensions";
import { ExtensionPanelSlot } from "../../extensions/ExtensionPanelSlot";
import { ExtensionProviderSlot } from "../../extensions/ExtensionProviderSlot";
import { ExtensionRelatedSlot } from "../../extensions/ExtensionRelatedSlot";
import { ExtensionPodSlot } from "../../extensions/ExtensionPodTools";
import type { ReactNode } from "react";
import type { ResourceStatusLine } from "@srelens/core";
import { Inspector, LoadingState, type InspectorProps } from "@srelens/ui-kit";
import { FailureState } from "../../lib/errorCopy";
import { CUSTOM_RESOURCE_ACTIONS } from "../../lib/kinds/custom";
import { CopyNameButton } from "./CopyNameButton";
import { DetailActions } from "./DetailActions";
import { factsBelow, useMetricTiles } from "./metricTiles";
import { MetricStrip, OpenTabButton, PeekFacts } from "./ResourceFrame";
import { SectionMemory } from "./Section";
import {
  describeTarget,
  detailTabs,
  useDetailPaneState,
  useDetailSubject,
  PANE_CONTAINERS,
  PANE_DETAILS,
  PANE_EVENTS,
  PANE_METRICS,
  PANE_YAML,
} from "./detailData";

/**
 * The peek host's own controls, both of them, in one object.
 *
 * The design gives the peek two affordances the full tab has no use for:
 * dismiss the peek, and promote what is in it to a tab. They are not two
 * facts — they are one fact ("this is the peek") wearing two buttons — so they
 * arrive together rather than as two optional callbacks. A host cannot pass
 * one without the other.
 */
export interface ResourceDetailViewPeek {
  /** Dismiss the peek. Also what its Escape key reaches. */
  onClose: () => void;
  /**
   * Promote this subject to a tab of its own. The host's, not this screen's:
   * the list already mints that route for a row's double click, and one
   * expression producing both is what stops the button and the gesture
   * drifting onto two tabs for one resource.
   */
  onOpenTab: () => void;
}

export interface ResourceDetailViewProps {
  context: string;
  /** The Kubernetes kind ("Pod", "Deployment", ...) — the same value
   *  `detailRoute` and `useObject` take, not the list screen's slug. */
  kind: string;
  /** The API group — a custom kind only; see `detailRoute`. */
  group?: string;
  namespace: string | null;
  name: string;
  /**
   * The list's own two controls. Optional only because a peek can be drawn
   * outside the list — nothing else about this screen varies with it.
   */
  peek?: ResourceDetailViewPeek;
}

/**
 * The design's third header line, for a kind that has one: the verdict alone.
 *
 * `resourceStatusLine` decides the word, its tone and the unhealthy dot
 * together, off the fetched object. `null` back from it is an ANSWER, not a
 * gap: a ConfigMap has no health, and it takes the whole line with it.
 *
 * The line once carried the ready figure and the age beside the word. Both are
 * tiles at the top of Overview now (#1074), and the same two figures an inch
 * apart is the noise #1070 took out of this pane.
 */
function statusHeader(line: ResourceStatusLine | null): Pick<InspectorProps, "status" | "statusKind" | "flagged"> {
  if (!line) return {};
  // `HealthKind` and `StatusKind` are the same five words by construction —
  // core says so in `k8sHealth`'s own comment — so the verdict passes straight
  // through rather than being re-mapped into a second opinion.
  return { status: line.status, statusKind: line.health, flagged: line.flagged };
}

/**
 * The detail PEEK: one subject, identified at the top with its quick tools
 * under the name, its panes beneath — the pane `mock-detail-pane.md` draws,
 * inside the resource list, with the tools moved up from its footer (#1072).
 *
 * ONE OF TWO SCREENS, and it knows nothing about the other. Spec rule R-5 said
 * the peek and the full tab were one component differing by a prop; the user's
 * full-tab mock retired it, and the two now have their own chrome and their
 * own layouts — a compact header with quick tools under the name here, a
 * breadcrumb and a three-column fact grid there. The strip of tiles is the
 * one block both draw, through the one `MetricStrip` (#1074).
 *
 * What they still share is everything that is not a look, and it is shared
 * through `detailData` rather than by being written twice: one read of the
 * object, one lazy-load rule per pane, one target gate, one table of per-kind
 * bodies, one derivation of a kind's facts, one set of actions. So a fact can
 * be laid out differently in the two screens and cannot be derived
 * differently. Nothing this file renders is restyled anywhere else.
 */
export function ResourceDetailView({ context, kind, group, namespace, name, peek }: ResourceDetailViewProps) {
  const subject = useDetailSubject({ context, kind, namespace, name, group });
  const { object, status, error, descriptor, statusLine, hasContainers, hasMetrics } = subject;
  // Before any early return: the usage it reads is never conditional on the
  // object having arrived.
  const tiles = useMetricTiles(subject, { context, kind, namespace, name });

  // Containers is a tab of its own here because the pane is 352px wide and a
  // table of containers is not; the full tab folds the same fact into its
  // Overview. No descriptor sets `panes.metrics` yet, and getting the order
  // right now is cheaper than remembering it later.
  const tabs = detailTabs([
    ...(hasContainers ? [{ id: PANE_CONTAINERS, label: "Containers" }] : []),
    ...(hasMetrics ? [{ id: PANE_METRICS, label: "Metrics" }] : []),
  ]);
  const { active, selectTab, yamlPane, eventsPane } = useDetailPaneState({
    context,
    kind,
    namespace,
    name,
    tabs,
    group,
  });

  const subtitle = namespace ? `${kind} · ${namespace}` : kind;
  // Offered on every state, not only the settled one: a resource that is slow
  // to load, or that failed to, is exactly the one a reader wants in a tab of
  // its own rather than in a peek that the next row click will replace.
  const actions = peek && <OpenTabButton onClick={peek.onOpenTab} />;
  // On every state too, and for the same reason: the name is the route's, not
  // the object's, so it is as copyable while loading or after a failed read
  // as it is over a pane full of facts — and a resource that will not load is
  // one a reader is about to go and ask kubectl about.
  const copyName = <CopyNameButton name={name} />;

  if (status === "loading") {
    return (
      <Inspector name={name} nameAction={copyName} subtitle={subtitle} actions={actions} onClose={peek?.onClose}>
        <LoadingState label={`Loading ${describeTarget(kind, namespace, name)}`} />
      </Inspector>
    );
  }

  if (status === "error" || !object) {
    // Names the object that failed, not just "failed" — several panes can be
    // open at once, and a bare failure doesn't say which one broke.
    return (
      <Inspector name={name} nameAction={copyName} subtitle={subtitle} actions={actions} onClose={peek?.onClose}>
        <FailureState
          title={`Could not load ${describeTarget(kind, namespace, name)}`}
          error={error}
          onRetry={subject.reload}
        />
      </Inspector>
    );
  }

  // Read once: the header draws the verdict, and the tools' Ask asks a
  // different question of an unhealthy subject than of a healthy one.
  const header = statusHeader(statusLine);
  // The tiles above them have said these.
  const facts = factsBelow(subject.facts, tiles);

  // The peek's Overview pane: the tab's strip of tiles (#1074), then a flat
  // run of sibling blocks, the lead facts first and Labels and Annotations
  // last. The strip is on Overview alone: two rows of tiles above a manifest
  // are most of a 352px pane, and the header's word says the verdict on every
  // pane. Nothing may be wrapped around any one of them — `.section +
  // .section` is what draws the hairlines between them, so a div around one
  // quietly removes the rule on both its sides.
  let pane: ReactNode =
    active === PANE_DETAILS ? (
      <>
        <MetricStrip tiles={tiles} />
        <PeekFacts facts={facts} />
        {subject.body}
        {subject.labels}
        {subject.annotations}
      </>
    ) : null;
  if (active === PANE_CONTAINERS) pane = subject.containersPane;
  else if (active === PANE_METRICS) pane = subject.metricsPane;
  else if (active === PANE_YAML) pane = yamlPane;
  else if (active === PANE_EVENTS) pane = eventsPane;

  return (
    <Inspector
      name={name}
      nameAction={copyName}
      subtitle={subtitle}
      {...header}
      actions={actions}
      tabs={tabs}
      activeTab={active}
      onTabChange={selectTab}
      tabsLabel="Resource views"
      onClose={peek?.onClose}
      // The quick tools, under the name as classic's drawer has them (#1072).
      // Nothing about them comes from `peek`: the actions a subject offers are
      // the kind's, not the screen's, and the full tab draws the very same row
      // in its header through the very same component. They are not offered
      // on the loading or error states above — Suspend/Resume reads the
      // object's own `spec`, and half of these actions are writes against
      // something the pane could not even read.
      toolbar={
        <DetailActions
          context={context}
          kind={kind}
          group={group}
          namespace={namespace}
          name={name}
          // A kind outside `K8S_KIND` has no descriptor at all, which is
          // precisely the custom-resource case — so it inherits the very
          // action set `customDescriptor` gives one, Delete withheld and all.
          actions={descriptor?.actions ?? CUSTOM_RESOURCE_ACTIONS}
          flagged={header.flagged ?? false}
          suspended={object.spec?.suspend === true}
          unschedulable={object.spec?.unschedulable === true}
          // Those two are read off this pane's object, so a write made from the
          // bar has to bring a new one: without it a node cordoned from here
          // went on being offered Cordon.
          onChanged={subject.refresh}
        />
      }
    >
      {/* Every titled block inside opens shut on a first visit and stays as
          the reader last left it for this kind. Wrapped here rather than in
          the shared layer because the pane is this screen's composition; the
          MEMORY is shared all the same, since it is keyed by kind in a store
          neither screen owns. A provider renders no element, so the run of
          sections beneath is still a run of direct siblings and every
          hairline is unchanged. (`lib/sectionFolds.ts`) */}
      <SectionMemory kind={kind}>{pane}</SectionMemory>
        {active === PANE_DETAILS && <ExtensionPanelSlot context={context} resource={object}/>}
        {active === PANE_DETAILS && <ExtensionProviderSlot context={context} resource={object}/>}
        {active === PANE_DETAILS && <ExtensionRelatedSlot context={context} resource={object}/>}
        {active === PANE_DETAILS && <ExtensionPodSlot context={context} resource={object}/>}
        {active === PANE_DETAILS && <ExtensionResourceSlot context={context} kind={object.kind ?? kind} group={object.apiVersion?.includes("/") ? object.apiVersion.split("/")[0] : ""} namespace={namespace} name={name} />}
    </Inspector>
  );
}
