import { ExtensionResourceSlot } from "../../extensions/Extensions";
import { ExtensionPanelSlot } from "../../extensions/ExtensionPanelSlot";
import { ExtensionProviderSlot } from "../../extensions/ExtensionProviderSlot";
import { ExtensionRelatedSlot } from "../../extensions/ExtensionRelatedSlot";
import { ExtensionPodSlot } from "../../extensions/ExtensionPodTools";
import type { ReactNode } from "react";
import { LoadingState } from "@srelens/ui-kit";
import { FailureState } from "../../lib/errorCopy";
import { CUSTOM_RESOURCE_ACTIONS } from "../../lib/kinds/custom";
import { DetailActions } from "./DetailActions";
import { factsBelow, useMetricTiles } from "./metricTiles";
import { ResourceTabFrame, TabFacts, TabMetadata, type Tile } from "./ResourceFrame";
import { SectionMemory } from "./Section";
import {
  describeTarget,
  detailTabs,
  useDetailPaneState,
  useDetailSubject,
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

  // Before any early return: the usage it reads is never conditional on the
  // object having arrived.
  const tiles = useMetricTiles(subject, { context, kind, namespace, name });

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

  // The figures above the grid have said these.
  const facts = factsBelow(subject.facts, tiles);

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
