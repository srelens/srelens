import type { ReactNode } from "react";
import { ageFromTimestamp, type K8sObject, type ResourceStatusLine } from "@srelens/core";
import {
  Button,
  Inspector,
  KV,
  LoadingState,
  type InspectorProps,
  type TabItem,
} from "@srelens/ui-kit";
import { FailureState } from "../../lib/errorCopy";
import { Icons } from "../../lib/icons";
import { CUSTOM_RESOURCE_ACTIONS } from "../../lib/kinds/custom";
import { DetailActions } from "./DetailActions";
import { SectionMemory, Section } from "./Section";
import {
  describeTarget,
  useDetailPaneState,
  useDetailSubject,
  PANE_CONTAINERS,
  PANE_DETAILS,
  PANE_EVENTS,
  PANE_METRICS,
  PANE_YAML,
} from "./detailData";
import type { DetailFact } from "./facts";

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
  namespace: string | null;
  name: string;
  /**
   * The list's own two controls. Optional only because a peek can be drawn
   * outside the list — nothing else about this screen varies with it.
   */
  peek?: ResourceDetailViewPeek;
}

/**
 * The design's third header line, for a kind that has one.
 *
 * THIS SCREEN'S, and the full tab draws the same verdict as a strip of metric
 * tiles instead, off the very same `ResourceStatusLine`: the shared layer
 * reads it once and hands it to whichever screen is drawing, so the two can
 * never disagree about a subject's health while looking nothing alike.
 *
 * `resourceStatusLine` decides the word, its tone and the unhealthy dot
 * together, off the fetched object; the age is not in its answer because it
 * is not a health fact, so it comes off the metadata here. `null` back from it
 * is an ANSWER, not a gap — a ConfigMap has no health and a custom resource's
 * `status` is its own operator's business — and it takes the whole line with
 * it, age included: a lone age with nothing to qualify it reads as the rest
 * having gone missing.
 *
 * Two things about the facts that are silent when wrong, both of them
 * `InspectorFact`'s doing:
 *
 * - `label` is never drawn — it is an `sr-only` `dt`. So the VALUE carries its
 *   own noun ("9/12 ready", not "9/12"), which is the user's ruling over the
 *   kit's objection, and the label is the term a screen reader hears. It has
 *   to say something the value does not. "Progress" rather than "Ready"
 *   because the phrase is not always about readiness: a Job's reads
 *   "3/3 complete".
 * - a fact defaults to normal ink. Only the age is quiet in the mock, so only
 *   the age asks for a tone.
 */
function statusHeader(
  line: ResourceStatusLine | null,
  object: K8sObject,
): Pick<InspectorProps, "status" | "statusKind" | "facts" | "flagged"> {
  if (!line) return {};
  const facts: InspectorProps["facts"] = [];
  if (line.readyText) facts.push({ label: "Progress", value: line.readyText });
  const created = object.metadata?.creationTimestamp;
  // Only when there is one: `ageFromTimestamp` answers "—" for an absent
  // stamp, and an em dash in the header is noise, not information.
  if (created) facts.push({ label: "Age", value: ageFromTimestamp(created), tone: "muted" });
  // `HealthKind` and `StatusKind` are the same five words by construction —
  // core says so in `k8sHealth`'s own comment — so the verdict passes straight
  // through rather than being re-mapped into a second opinion.
  return { status: line.status, statusKind: line.health, facts, flagged: line.flagged };
}

/**
 * The design's other header affordance: promote this subject to a tab of its
 * own. Outlined and labelled, beside the close rather than instead of it —
 * the mock draws two separate controls, and a peek that could only be left by
 * closing it is a peek the reader has to re-find.
 */
function OpenTabButton({ onClick }: { onClick: () => void }) {
  const Glyph = Icons.openTab;
  return (
    <Button type="button" variant="outline" size="xs" onClick={onClick}>
      {/* The word is the accessible name; the glyph only decorates it. */}
      <Glyph size={12} aria-hidden="true" />
      Open tab
    </Button>
  );
}

/**
 * THE PEEK'S OWN FACT LAYOUT: a label column and a value column, read down.
 *
 * The mock's two-column list, in a pane about 352px wide — the label in muted
 * ink, the value beside it, one fact per line. The full tab reads the very
 * same facts across three columns of label-above-value and builds that itself;
 * this screen has never seen that layout and cannot be changed by it. What the
 * two share is the LIST (`detailFacts`), not a line of its markup. (#331)
 *
 * Untitled, because the design heads the first block of a detail with nothing:
 * the header above has already given the name, the kind and the namespace. An
 * untitled block also cannot fold, which is what keeps a detail from opening
 * showing nothing at all.
 *
 * A subject with no facts draws no block: an empty section still has its
 * padding and still rules against whatever follows it.
 */
function PeekFacts({ facts }: { facts: DetailFact[] }) {
  if (facts.length === 0) return null;
  return (
    // Named, so a test can hold this screen to drawing the WHOLE derived list
    // rather than a fact or two off it — a screen that quietly dropped three
    // of them would otherwise look exactly right. A class rather than a
    // wrapper: the block has to stay a direct sibling of what follows it, or
    // `.section + .section` draws no hairline against it.
    <Section className="fact-list">
      {facts.map((fact) => (
        <KV key={fact.label} k={fact.label} v={fact.value} mono={fact.mono} />
      ))}
    </Section>
  );
}

/**
 * The detail PEEK: one subject, identified at the top, its panes beneath, its
 * actions along the bottom — the pane `mock-detail-pane.md` draws, inside the
 * resource list.
 *
 * ONE OF TWO SCREENS, and it knows nothing about the other. Spec rule R-5 said
 * the peek and the full tab were one component differing by a prop; the user's
 * full-tab mock retired it, and the two now have their own chrome, their own
 * tab labels and their own layouts — a compact header and a footer action bar
 * here, a breadcrumb, a metric strip and a three-column fact grid there.
 *
 * What they still share is everything that is not a look, and it is shared
 * through `detailData` rather than by being written twice: one read of the
 * object, one lazy-load rule per pane, one target gate, one table of per-kind
 * bodies, one derivation of a kind's facts, one set of actions. So a fact can
 * be laid out differently in the two screens and cannot be derived
 * differently. Nothing this file renders is restyled anywhere else.
 */
export function ResourceDetailView({ context, kind, namespace, name, peek }: ResourceDetailViewProps) {
  const subject = useDetailSubject({ context, kind, namespace, name });
  const { object, status, error, descriptor, statusLine, hasContainers, hasMetrics } = subject;

  // THIS SCREEN'S STRIP: `Details Containers YAML Events Metrics`, the mock's
  // five. Containers is a tab of its own here because the pane is 352px wide
  // and a table of containers is not; the full tab folds the same fact into
  // its Overview. Metrics trails the panes every kind has because it is the
  // one nothing offers yet — no descriptor sets `panes.metrics` — and getting
  // the order right now is cheaper than remembering it later.
  const tabs: TabItem[] = [
    { id: PANE_DETAILS, label: "Details" },
    ...(hasContainers ? [{ id: PANE_CONTAINERS, label: "Containers" }] : []),
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

  const subtitle = namespace ? `${kind} · ${namespace}` : kind;
  // Offered on every state, not only the settled one: a resource that is slow
  // to load, or that failed to, is exactly the one a reader wants in a tab of
  // its own rather than in a peek that the next row click will replace.
  const actions = peek && <OpenTabButton onClick={peek.onOpenTab} />;

  if (status === "loading") {
    return (
      <Inspector name={name} subtitle={subtitle} actions={actions} onClose={peek?.onClose}>
        <LoadingState label={`Loading ${describeTarget(kind, namespace, name)}`} />
      </Inspector>
    );
  }

  if (status === "error" || !object) {
    // Names the object that failed, not just "failed" — several panes can be
    // open at once, and a bare failure doesn't say which one broke.
    return (
      <Inspector name={name} subtitle={subtitle} actions={actions} onClose={peek?.onClose}>
        <FailureState title={`Could not load ${describeTarget(kind, namespace, name)}`} error={error} />
      </Inspector>
    );
  }

  // Read once: the header draws the verdict, and the footer's Ask asks a
  // different question of an unhealthy subject than of a healthy one.
  const header = statusHeader(statusLine, object);

  // The peek's Details pane: a flat run of sibling blocks, the lead facts
  // first and Labels and Annotations last. Nothing may be wrapped around any
  // one of them — `.section + .section` is what draws the hairlines between
  // them, so a div around one quietly removes the rule on both its sides.
  let pane: ReactNode =
    active === PANE_DETAILS ? (
      <>
        <PeekFacts facts={subject.facts} />
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
      subtitle={subtitle}
      {...header}
      actions={actions}
      tabs={tabs}
      activeTab={active}
      onTabChange={selectTab}
      tabsLabel="Resource views"
      onClose={peek?.onClose}
      // The design's bar. Nothing about it comes from `peek`: the actions a
      // subject offers are the kind's, not the screen's, and the full tab
      // draws the very same row in its header through the very same
      // component. It is not offered on the loading or error states above —
      // Suspend/Resume reads the object's own `spec`, and half of these
      // actions are writes against something the pane could not even read.
      footer={
        <DetailActions
          context={context}
          kind={kind}
          namespace={namespace}
          name={name}
          // A kind outside `K8S_KIND` has no descriptor at all, which is
          // precisely the custom-resource case — so it inherits the very
          // action set `customDescriptor` gives one, Delete withheld and all.
          actions={descriptor?.actions ?? CUSTOM_RESOURCE_ACTIONS}
          flagged={header.flagged ?? false}
          suspended={object.spec?.suspend === true}
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
    </Inspector>
  );
}
