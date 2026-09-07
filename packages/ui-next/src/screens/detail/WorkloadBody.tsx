import { useEffect, useState } from "react";
import {
  ageFromTimestamp,
  ageSortValue,
  asArray,
  asRecord,
  listReplicaSets,
  str,
  updateStrategy,
  type Condition,
  type K8sObject,
  type ReplicaSetSummary,
} from "@srelens/core";
import { KV, LoadingState, PairList, Table, type Column } from "@srelens/ui-kit";
import { Section } from "./Section";
import { SectionFailure, useSectionList, type SectionListState } from "./sectionList";
import type { DetailFact, FactsFor } from "./facts";
import {
  ConditionsSection,
  RelatedPodsSection,
  StringList,
} from "./sections";
import { SELF_DESCRIBING_KINDS } from "./GenericBody";
import { hasSelector, requirementText, selectorOf, type WorkloadSelector } from "../../lib/workloadSelector";

/** The annotation a Deployment records its current rollout number in. */
const REVISION_ANNOTATION = "deployment.kubernetes.io/revision";


/**
 * "RollingUpdate · surge 25% · unavailable 0" / "RollingUpdate · partition 2"
 * / "OnDelete".
 *
 * The form is this design's, read off frame A's Strategy row: a middle-dot
 * run rather than a parenthesised comma list, labels without their "max"
 * prefix, and surge named before unavailable. Where the mock and the build
 * disagree on a value's form, the mock wins.
 *
 * The FACTS come from core's `updateStrategy`, which every design shares; the
 * words are chosen here, and only here. Classic draws the same numbers as
 * "RollingUpdate (max unavailable 0, max surge 25%)" and is frozen, so a
 * shared formatter would have to pick one app's typography for both — it
 * briefly did, and retyped classic's rows by accident.
 *
 * One helper for every kind, so a DaemonSet's Update strategy row reads the
 * way a Deployment's does: the mock only draws the Deployment, but two forms
 * for one fact would be a worse answer than the one it does draw.
 */
function updateStrategyText(strategy: Record<string, unknown>): string {
  const { type, partition, maxSurge, maxUnavailable } = updateStrategy(strategy);
  const parts: string[] = [];
  if (partition != null) parts.push(`partition ${partition}`);
  if (maxSurge != null) parts.push(`surge ${maxSurge}`);
  if (maxUnavailable != null) parts.push(`unavailable ${maxUnavailable}`);
  return [type, ...parts].join(" · ");
}

/**
 * A selector as the pane reads it: the equality labels as `k=v` pairs, with
 * each requirement under them in Kubernetes' own syntax.
 *
 * BOTH halves, because a row that shows only the labels describes a wider set
 * of pods than the table below it lists — and a workload selected entirely by
 * expressions had no Selector row at all, on a pane that now finds its pods.
 */
function SelectorValue({ selector }: { selector: WorkloadSelector }) {
  return (
    <>
      <PairList pairs={Object.entries(selector.matchLabels)} breakValues />
      {selector.matchExpressions.length > 0 && (
        <StringList items={selector.matchExpressions.map(requirementText)} />
      )}
    </>
  );
}

/** The images a workload's pod template runs, each named once. */
function templateImages(spec: Record<string, unknown>): string[] {
  const containers = asArray(asRecord(asRecord(spec.template).spec).containers);
  return [...new Set(containers.map((c) => str(asRecord(c).image)).filter(Boolean))];
}

const DEPLOY_REVISION_COLUMNS: Column<ReplicaSetSummary>[] = [
  { key: "revision", header: "#", render: (r) => <span className="font-mono">{r.revision || "—"}</span> },
  { key: "name", header: "Name", render: (r) => <span className="font-mono">{r.name}</span> },
  { key: "pods", header: "Pods", render: (r) => `${r.ready}/${r.desired}` },
  { key: "age", header: "Age", getSortValue: ageSortValue, render: (r) => r.age },
];

/** A Deployment's rolled-out ReplicaSets, as {@link useSectionList} holds any
 *  block's own list — `idle` for a kind that never asks (see below). */
export type RevisionsState = SectionListState<ReplicaSetSummary[]>;

/**
 * The ReplicaSets a Deployment has rolled out, fetched once for the whole
 * body — classic's `DeployRevisions`, via core's `listReplicaSets`.
 *
 * Called by the SHARED layer, once per subject, and its result handed to both
 * things that need it: the table below, and the `Revision` fact
 * (`workloadFacts`), whose "(6m ago)" is the age of the ReplicaSet carrying
 * the current revision number. Two fetches of one list is one list too many,
 * and two lists that arrive at different moments is a pane that can show a
 * revision the table does not have — which is exactly what would happen if
 * the fact list and this body each fetched their own now that the fact list
 * is data drawn by two different screens.
 *
 * Deployment-only (`enabled`): classic never calls this for
 * StatefulSet/DaemonSet/ReplicaSet either, since only a Deployment has
 * revision history of its own. The hook still runs for every kind — hooks
 * must, and the shared layer calls it for every kind a detail can be opened
 * on — and simply fetches nothing.
 */
export function useDeployRevisions(context: string, namespace: string, ownerName: string, enabled: boolean): RevisionsState {
  return useSectionList<ReplicaSetSummary[]>(
    enabled && context !== "" && namespace !== "" && ownerName !== "",
    [context, namespace, ownerName],
    async () => {
      const out = await listReplicaSets(context, namespace, ownerName);
      return out.error ? { error: out.error } : { data: out.replicasets ?? [] };
    },
  );
}

/**
 * The revisions table itself — the fetched list, rendered. Name is a
 * `ResourceLink` in classic, and the whole row is `onRowClick`-navigable;
 * both render as plain mono text here — see the task report for the full
 * inert-value list. Classic's own component has no write action (no rollback
 * button, no menu) — only navigation — so nothing needed to be scoped out on
 * that account; it only ever SHOWS revisions.
 */
function DeployRevisionsSection({ state }: { state: RevisionsState }) {
  // `idle` alone draws nothing, and that is still right: a StatefulSet,
  // DaemonSet or ReplicaSet has no revision history, so nothing was asked for
  // and there is nothing to report. `error` used to be lumped in with it, which
  // is what made "this Deployment has never rolled out" and "srelens was
  // refused to list replicasets" the identical screen — see
  // {@link useSectionList}.
  if (state.status === "idle") return null;
  return (
    <Section title="Deploy Revisions">
      {state.status === "loading" ? (
        <LoadingState label="Loading revisions" />
      ) : state.status === "error" ? (
        <SectionFailure error={state.error} />
      ) : (
        <Table
          columns={DEPLOY_REVISION_COLUMNS}
          data={state.data ?? []}
          getRowKey={(r) => r.name}
          emptyText="No revisions"
        />
      )}
    </Section>
  );
}

/**
 * A Deployment/StatefulSet/ReplicaSet's facts, in the order the design's own
 * Deployment frame reads them: Replicas, Up to date, Strategy, Revision,
 * Selector, Min ready seconds, Namespace, Created, Image — with the
 * kind-specific extras (Managed by, a StatefulSet's Service and volume claim
 * templates) beside their own kin.
 *
 * DATA, so the two detail screens can lay one list out two ways — the peek
 * down a column, the full tab across three — without either drawing the
 * other's markup. `revisions` arrives from the shared layer, which fetches it
 * once for this list AND for the Deploy Revisions table below, so a rollout
 * cannot be in one and missing from the other.
 *
 * No heading and no `Name` fact: the pane's header has already given the name,
 * the kind and the namespace.
 *
 * NO STATUS ROW EITHER, and that is the fix rather than an omission. This
 * panel used to state a workload's health a second time, from
 * `availableReplicas >= desired` — and available is the subset of ready
 * replicas that have outlived `minReadySeconds`, so a Deployment with that
 * field set showed a header reading "Running · 12/12 ready" directly above a
 * panel reading "Pending". Two readings of one fact can disagree. The header
 * already says the word (through core's `resourceStatusLine`), the design's
 * own Deployment frame has no such row, and the numbers under it say the rest
 * — so the second reading is deleted, not re-pointed. The design DOES keep a
 * `Status` row on a Pod, where the phase is the pod's own vocabulary rather
 * than a count; `PodBody` renders it, from `resourceStatusLine`.
 *
 * `Replicas` reads "9 ready · 12 desired" — the design's form, and the same
 * two numbers the header and the list row show, off `status.readyReplicas`
 * like both of them. It replaces a five-number sentence ("12 desired, 9
 * updated, 12 total, 9 available, 0 unavailable") that made the reader find
 * the two that mattered; `Up to date` gets the row of its own the design
 * gives it, and the rest are on the YAML tab.
 *
 * `Strategy` is `updateStrategyText` below — core's `updateStrategy` facts in
 * this design's own words — for every kind. It always read the whole strategy
 * for a StatefulSet/DaemonSet; a Deployment alone read `spec.strategy.type`
 * and so printed "RollingUpdate" with the surge and unavailable clauses — the
 * two numbers that decide how a rollout behaves — dropped.
 *
 * Namespace and Managed by are a `ResourceLink`/`LinkedResources` in classic
 * that navigate; they render here as plain text (see the task report).
 */
export const workloadFacts: FactsFor = ({ kind, object, revisions }) => {
  const meta = object.metadata ?? {};
  const spec = asRecord(object.spec);
  const status = asRecord(object.status);
  const selector = selectorOf(object);
  const owners = meta.ownerReferences ?? [];
  const created = str(meta.creationTimestamp);

  const num = (v: unknown) => (v != null ? Number(v) : 0);
  const desired = num(spec.replicas);
  const ready = num(status.readyReplicas);
  const updated = num(status.updatedReplicas);
  const strategy =
    kind === "Deployment"
      ? updateStrategyText(asRecord(spec.strategy))
      : updateStrategyText(asRecord(spec.updateStrategy));

  // The number is the Deployment's own annotation; the age belongs to the
  // ReplicaSet carrying that revision, which may not have arrived yet — the
  // number alone is still a true fact, so it shows without waiting.
  const revision = str((meta.annotations ?? {})[REVISION_ANNOTATION]);
  const revisionAge = revisions?.find((r) => r.revision === revision)?.age;
  const revisionText = revisionAge ? `${revision} (${revisionAge} ago)` : revision;

  const serviceName = kind === "StatefulSet" ? str(spec.serviceName) : "";
  const volumeClaimTemplateNames =
    kind === "StatefulSet"
      ? asArray(spec.volumeClaimTemplates)
          .map((t) => str(asRecord(asRecord(t).metadata).name))
          .filter(Boolean)
      : [];
  const images = templateImages(spec);

  const facts: DetailFact[] = [
    { label: "Replicas", value: `${ready} ready · ${desired} desired` },
    { label: "Up to date", value: `${updated} of ${desired}` },
  ];
  if (strategy) facts.push({ label: "Strategy", value: strategy });
  if (revision) facts.push({ label: "Revision", value: revisionText });
  if (hasSelector(selector)) {
    facts.push({ label: "Selector", value: <SelectorValue selector={selector} /> });
  }
  if (spec.minReadySeconds != null) {
    facts.push({ label: "Min ready seconds", value: str(spec.minReadySeconds) });
  }
  if (owners.length > 0) {
    facts.push({
      label: "Managed by",
      value: <StringList items={owners.map((o) => `${o.kind}/${o.name}`)} />,
    });
  }
  if (serviceName) facts.push({ label: "Service", value: serviceName, mono: true });
  if (volumeClaimTemplateNames.length > 0) {
    facts.push({ label: "Volume claim templates", value: volumeClaimTemplateNames.join(", ") });
  }
  if (meta.namespace) facts.push({ label: "Namespace", value: str(meta.namespace), mono: true });
  if (created) facts.push({ label: "Created", value: `${ageFromTimestamp(created, Date.now())} ago` });
  if (images.length > 0) {
    facts.push({
      label: "Image",
      value:
        images.length === 1 ? <span className="font-mono">{images[0]}</span> : <StringList items={images} />,
    });
  }
  return facts;
};

/**
 * A DaemonSet's Scheduling block — classic's `DaemonSetBody`. Unlike the
 * other three workload kinds, a DaemonSet has no "replicas": its own numbers
 * are per-node (desired/current/ready/up-to-date/available across matching
 * nodes), read straight off `status`.
 */
function DaemonSetSchedulingSection({ object }: { object: K8sObject }) {
  const spec = asRecord(object.spec);
  const status = asRecord(object.status);
  const selector = selectorOf(object);
  const strategyText = updateStrategyText(asRecord(spec.updateStrategy));

  return (
    <Section title="Scheduling">
      {status.desiredNumberScheduled != null && <KV k="Desired" v={str(status.desiredNumberScheduled)} />}
      {status.currentNumberScheduled != null && <KV k="Current" v={str(status.currentNumberScheduled)} />}
      {status.numberReady != null && <KV k="Ready" v={str(status.numberReady)} />}
      {status.updatedNumberScheduled != null && <KV k="Up-to-date" v={str(status.updatedNumberScheduled)} />}
      {status.numberAvailable != null && <KV k="Available" v={str(status.numberAvailable)} />}
      {strategyText && <KV k="Update strategy" v={strategyText} />}
      {hasSelector(selector) && <KV k="Selector" v={<SelectorValue selector={selector} />} />}
    </Section>
  );
}



/**
 * The Details pane for Deployment, StatefulSet, DaemonSet and ReplicaSet —
 * classic's `WorkloadDetailView` (Deployment/StatefulSet/ReplicaSet) and
 * `DaemonSetBody` (DaemonSet), which classic renders as genuinely different
 * shapes (replica counts vs. per-node counts), not variations on one KV list —
 * on the design's own shape: a flat run of blocks divided by hairline rules,
 * not a stack of cards.
 *
 * Every block is a sibling of every other, with nothing wrapped around any of
 * them: `.section + .section` is what draws the rule between two blocks, so a
 * div — or a bare `LoadingState` — between two of them quietly removes the
 * rule on both sides. A block with nothing to say renders nothing at all.
 *
 * Conditions are rendered here ONLY for the `SELF_DESCRIBING_KINDS` — the same
 * gate related pods use, and for the same reason: a DaemonSet is wrapped by
 * `GenericBody`, which supplies them, so rendering them here too would show
 * them twice. Labels and Annotations are no longer rendered by any body at
 * all; the host places them once, which is what retired their half of this
 * guard.
 *
 * `kind` is the route's, handed down by `ResourceDetailView` — not `object.kind`,
 * which this read until the whole-branch review. The API server happens to
 * return `kind` on a single-object GET, so the two agreed; but the pane is
 * dispatched on the route's kind and a body that re-derives it is a second
 * source of truth for the fact its own dispatch turned on. Taking the prop
 * also retires an `if (!kind)` guard that returned a bare `EmptyState` into
 * the run of sections, breaking the `.section + .section` hairline chain:
 * `DETAILS_BODY[""]` is undefined, so no empty kind can reach this at all.
 * (#331)
 *
 * Related pods (classic's `ManagedPods`) follow the same rule. DaemonSet is
 * deliberately excluded: classic's `DaemonSetBody` renders ONLY its Scheduling
 * section — it is the generic `GenericDetail` wrapper that supplies a
 * DaemonSet's related pods, and `GenericBody` (this package's port of that
 * wrapper) already adds one via `relatedPodSelector` for every kind that isn't
 * self-describing.
 */
export function WorkloadDetailsBody({
  kind,
  object,
  context,
  revisions = { status: "idle" },
}: {
  kind: string;
  object: K8sObject;
  context: string;
  /** The shared layer's one read of this Deployment's ReplicaSets. Defaulted
   *  to idle so a body rendered on its own — in a test, or by a caller with
   *  no revisions to hand — draws no revisions table rather than throwing. */
  revisions?: RevisionsState;
}) {
  const meta = object.metadata ?? {};
  const spec = asRecord(object.spec);
  const namespace = str(meta.namespace);
  const selector = selectorOf(object);
  const selfDescribing = SELF_DESCRIBING_KINDS.has(kind);
  const conditions = asArray(asRecord(object.status).conditions) as unknown as Condition[];

  return (
    <>
      {/* A DaemonSet has no replicas to lead with — its numbers are per-node
          — so its Scheduling block is a titled section here rather than a
          lead fact list, and `detailFacts` gives it the identity facts every
          other wrapped kind gets. */}
      {kind === "DaemonSet" && <DaemonSetSchedulingSection object={object} />}
      <DeployRevisionsSection state={revisions} />
      {hasSelector(selector) && namespace && selfDescribing && (
        <RelatedPodsSection context={context} namespace={namespace} selector={selector} />
      )}
      {selfDescribing && <ConditionsSection conditions={conditions} />}
    </>
  );
}
