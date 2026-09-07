import type { ReactNode } from "react";
import {
  ageFromTimestamp,
  asArray,
  asRecord,
  relatedPodSelector,
  str,
  type Condition,
  type K8sObject,
} from "@srelens/core";
import { ConditionsSection, RelatedPodsSection, StringList } from "./sections";
import type { DetailFact, FactsFor } from "./facts";

/**
 * The four kinds classic's `ObjectDetail` special-cases with their own
 * "Properties" section (`PodDetailsBody`, `WorkloadDetailsBody` for
 * Deployment/StatefulSet/ReplicaSet) — each already covers the same
 * Namespace/Created/Labels/Annotations/Controlled-by facts this wrapper's
 * identity block and its Labels/Annotations blocks would add, which is why
 * classic renders them without its generic wrapper (`GenericDetail`) at all.
 * Every other kind — including DaemonSet, which classic does NOT special-case
 * here even though it has its own body — falls through to `GenericBody`,
 * alone or with a `DETAILS_BODY` entry nested inside it.
 */
export const SELF_DESCRIBING_KINDS: ReadonlySet<string> = new Set([
  "Pod",
  "Deployment",
  "StatefulSet",
  "ReplicaSet",
]);


/**
 * A kind's identity, as facts — classic's `GenericDetail` "Metadata" section,
 * minus the two things the design's own frame settles differently.
 *
 * The lead fact list of every kind that has no list of its own: a Node, a
 * Service, a ConfigMap, a CronJob, a custom resource. The four
 * `SELF_DESCRIBING_KINDS` have their own (`podFacts`, `workloadFacts`), which
 * already cover these same facts — see {@link SELF_DESCRIBING_KINDS}.
 *
 * DATA rather than a section, because the two detail screens lay a fact list
 * out differently and share the derivation: `detailFacts` picks the list, the
 * peek reads it down a column and the full tab reads it across three. It used
 * to be a `Section` rendered here, which is why the tab had to restyle it from
 * above (`FactGrid`). (#331)
 *
 * No heading, and no heading is possible: the design heads the first block of
 * a detail with nothing — the pane's header has already given the name, the
 * kind and the namespace, and a "Metadata" bar under it is a second name for
 * the same thing.
 *
 * No `Name` row either, for the same reason — it repeated the header verbatim
 * on every kind, which is a carry-over from classic rather than a decision.
 * `Created` reads as an age alone (`84d ago`); the absolute stamp classic
 * appended is a second rendering of one fact in a 352px column.
 *
 * Labels and Annotations are no longer rows here at all — squeezed into the
 * value column of a fact list, a `key=value` pair had about a third of the
 * pane to be read in. They are blocks of their own below.
 *
 * Namespace and Controlled by are a `ResourceLink`/`LinkedResources` in
 * classic that navigate — Namespace to the Namespace object, Controlled by to
 * each owner's own kind/name; neither can navigate here (`PaneBody` has no
 * navigation contract — see the task report), so both render as plain text.
 *
 * An object with none of these facts yields an empty list, and a screen draws
 * no block at all for one: an empty section still has its padding and still
 * draws a rule against whatever follows it.
 */
export const identityFacts: FactsFor = ({ object }) => {
  const meta = object.metadata ?? {};
  const owners = meta.ownerReferences ?? [];
  const created = str(meta.creationTimestamp);
  const facts: DetailFact[] = [];
  if (meta.namespace) facts.push({ label: "Namespace", value: str(meta.namespace), mono: true });
  if (created) facts.push({ label: "Created", value: `${ageFromTimestamp(created, Date.now())} ago` });
  if (owners.length > 0) {
    facts.push({
      label: "Controlled by",
      value: <StringList items={owners.map((o) => `${o.kind}/${o.name}`)} />,
    });
  }
  return facts;
};

/**
 * The Details pane's fallback wrapper — classic's `GenericDetail`, on the
 * design's own shape: a flat run of blocks divided by hairline rules, not a
 * stack of cards. The kind's own `DETAILS_BODY` entry comes first (`children`,
 * classic's `KindBody`) where one exists, then related pods (where
 * `relatedPodSelector` finds a selector for this kind), then Conditions — the
 * order the design's own frames read in.
 *
 * The identity facts are NOT here. They are the pane's lead fact list, they
 * are data ({@link identityFacts}), and each screen lays them out itself above
 * this — the peek down a column, the tab across three. A section rendered here
 * could only ever produce one of those, which is why the tab used to restyle
 * it from above. (#331)
 *
 * Every block is a sibling of every other, with nothing wrapped around any of
 * them: `.section + .section` is what draws the rule between two blocks, so a
 * div around one would quietly remove the rule on both sides of it. A block
 * with nothing to say renders nothing at all rather than an empty section, and
 * the rules then land in the right places on their own — nothing counts blocks
 * or is told which one is first.
 *
 * Labels and Annotations are NOT here. They close every kind's detail, so
 * they are the host's to place rather than the body's — the peek stacks them
 * under the rest and the full tab reads them side by side, and a body that
 * rendered them itself could only ever produce one of those. They used to be
 * rendered in three files (here, `PodBody`, `WorkloadBody`), guarded in the
 * third by a `SELF_DESCRIBING_KINDS` check whose only job was to stop them
 * appearing twice; placing them once, above, retires that guard along with
 * the class of bug it was watching for. (#331)
 *
 * `ResourceDetailView` wraps every kind's Details pane in this component; for the
 * four `SELF_DESCRIBING_KINDS` it passes through `children` untouched, since
 * those kinds' own bodies already show the facts this wrapper would otherwise
 * duplicate. Adding a kind to `DETAILS_BODY` nests it here automatically, and
 * a kind with no entry still gets a complete, correct detail from this wrapper
 * alone.
 */
export function GenericBody({
  kind,
  object,
  context,
  children,
}: {
  kind: string;
  object: K8sObject;
  context: string;
  children?: ReactNode;
}) {
  if (SELF_DESCRIBING_KINDS.has(kind)) return <>{children}</>;

  const meta = object.metadata ?? {};
  const namespace = str(meta.namespace);
  const conditions = asArray(asRecord(object.status).conditions) as unknown as Condition[];
  // Core's `relatedPodSelector` answers with the equality half only — it is
  // also classic's reader, and classic is frozen — so the requirements half
  // is empty here rather than absent: the section takes a whole
  // `LabelSelector`, and an empty `matchExpressions` sends exactly the
  // payload this always sent. A DaemonSet, Job, PodDisruptionBudget or
  // NetworkPolicy written with `matchExpressions` still resolves through that
  // reader's equality half alone — a narrower instance of the same gap
  // WorkloadBody just closed, left because widening core's reader changes
  // what the frozen app sends.
  const podSelector = { matchLabels: relatedPodSelector(kind, object), matchExpressions: [] };
  const hasPodSelector = Object.keys(podSelector.matchLabels).length > 0;

  return (
    <>
      {children}
      {context && namespace && hasPodSelector && (
        <RelatedPodsSection context={context} namespace={namespace} selector={podSelector} />
      )}
      <ConditionsSection conditions={conditions} />
    </>
  );
}
