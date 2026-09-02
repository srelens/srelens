/**
 * The blocks a detail body is built from that more than one body needs — the
 * string list, the labels and annotations blocks, the conditions list, the
 * managed-pods table. Each of them was written between two and SIX times
 * before it was written once, and every copy had drifted from the others by
 * the time they were compared. (#331)
 *
 * The file was `ConditionsSection.tsx` while it held one block. It holds eight
 * now — `StringList`, `LabelsSection`, `AnnotationsSection`,
 * `AnnotationsToggle`, `AnnotationLines`, `ConditionsSection` and
 * `RelatedPodsSection`, and `NodePodsSection`, the same eight
 * `sections.test.tsx` sweeps for exactly
 * one definition of — so it is named for what it is: this design's shared
 * detail sections.
 */
import { useEffect, useState } from "react";
import {
  ageFromTimestamp,
  conditionKindWithReason,
  plural,
  podMetrics,
  podsForSelector,
  podsOnNode,
  podStatus,
  type Condition,
  type PodMetric,
  type PodSummary,
} from "@srelens/core";
import {
  Button,
  KV,
  LoadingState,
  PairList,
  StatusPill,
  Table,
  type Column,
} from "@srelens/ui-kit";
import { Section } from "./Section";
import { SectionFailure, useSectionList } from "./sectionList";
import { formatCpu, formatMemory } from "../../lib/kinds/columns";
import type { WorkloadSelector } from "../../lib/workloadSelector";
import { detailRoute } from "../../lib/detailRoute";
import { currentWorkspace, openTab, setTabView } from "../../lib/tabsStore";

/**
 * A formatted list, one item per line — a pod's IPs, an owner reference, a
 * container's ports, a certificate's SANs.
 *
 * One implementation, replacing SIX byte-identical ones: `CronJobBody`,
 * `GenericBody`, `PodBody`, `SecretBody`, `ServiceBody` and `WorkloadBody`
 * each carried their own, every one of them justified in a comment saying it
 * was too small to share. Six copies of four lines is still six places to fix
 * a wrapping bug in. (#331)
 */
export function StringList({ items }: { items: string[] }) {
  return (
    <ul className="flex flex-col gap-0.5">
      {items.map((item, i) => (
        <li key={`${item}-${i}`} className="font-mono text-[0.8125rem]">
          {item}
        </li>
      ))}
    </ul>
  );
}

export interface ConditionsSectionProps {
  /**
   * The conditions to print, in the order they should read. Ordering is the
   * caller's — a Pod's lifecycle runs PodScheduled to Ready
   * (`orderPodConditions`), a workload's does not — and nothing here reorders
   * them.
   */
  conditions: Condition[];
}

/**
 * An object's conditions: one row each, the condition's name beside its
 * status and reason.
 *
 * The one implementation, replacing three. Conditions used to render a
 * sortable four-column `Table` for a generic kind, a three-part flex row for a
 * Pod, and — for a Deployment, the kind the design's own frame illustrates —
 * a bare row of pills carrying neither the status value nor the reason, so the
 * one thing a reader opens the block for was the one thing missing. Three
 * renderings of the same data is three chances to disagree about it, and they
 * did. (#331)
 *
 * Conditions arrive as data, never as an object to read: the module has no
 * idea whether it is printing a Pod's, a Node's or a Deployment's, which is
 * what lets every body share it. `conditionKindWithReason` is core's severity
 * heuristic, so a condition is toned the same way wherever it appears in this
 * design. It is the `WithReason` variant on purpose: this design's mock draws
 * a `Progressing · True · ReplicaSetUpdated` amber and the completed
 * `NewReplicaSetAvailable` green, which is a reading of one controller's
 * vocabulary and so a decision this design makes on its own. Classic calls
 * plain `conditionKind` and tones without it; that split is what keeps a
 * change made for this mock out of a frozen app's screens.
 *
 * The name is `tinted`, which colours it for a bad state and leaves it plain
 * for a good one — red `Available` above a plain `ReplicaFailure`, both beside
 * their own toned dot. The asymmetry lives in `StatusPill`; this only says the
 * rule applies here.
 *
 * The status and reason read as one value, `False · MinimumReplicasUnavailable`,
 * with an em dash standing in when there is no reason — an empty half of a
 * two-part value reads as a rendering fault. The last-transition time the old
 * table carried is gone: the design has no column for it, and the block is
 * read for what the object is complaining about, not when it started.
 *
 * An object reporting no conditions renders nothing at all — not an empty
 * block, which would still draw its own rule against the block below it.
 */
export function ConditionsSection({ conditions }: ConditionsSectionProps) {
  if (conditions.length === 0) return null;
  return (
    <Section title="Conditions">
      {conditions.map((condition) => (
        <KV
          key={condition.type}
          k={<StatusPill status={condition.type} kind={conditionKindWithReason(condition)} tinted />}
          v={`${condition.status} · ${condition.reason || "—"}`}
        />
      ))}
    </Section>
  );
}

/**
 * The annotation `kubectl apply` writes: the whole manifest it last sent,
 * verbatim, as one line of JSON.
 */
const LAST_APPLIED = "kubectl.kubernetes.io/last-applied-configuration";

export interface AnnotationSplit {
  /** The annotations to print, in the object's own order. */
  shown: Array<[key: string, value: string]>;
  /** The keys held back, for a caller that wants to say so its own way. */
  withheld: string[];
}

/**
 * Split an annotation map into the part worth printing and the part that is
 * not.
 *
 * One key is held back, and only for how it reads: `last-applied-configuration`
 * is an entire manifest on a single line — kilobytes of JSON — and the design
 * prints annotations full-width and unwrapped, so on a real Deployment that one
 * value buries every other annotation under a screen or more of text in a pane
 * that is 352px wide. The design's four short lines are not what a cluster
 * looks like. Nothing is lost by holding it back: it is a copy of the object's
 * own spec, and the pane's YAML tab shows that in full, indented and
 * searchable, which is the better place to read it anyway.
 *
 * WHAT THIS IS NOT: it is not redaction, and no gate above it may be dropped on
 * its account. It happens to remove the one annotation through which a Secret's
 * base64 `data` map reaches the page, but that is a side effect of a
 * legibility rule, not a promise — any other annotation, on any kind, is
 * printed exactly as it arrives. `Secret` keeps its own gate in
 * `AnnotationsSection` below (`AnnotationsToggle`, which mounts nothing until
 * a reader asks), and a Secret must never be routed through this instead.
 * (#331)
 */
export function partitionAnnotations(annotations: Record<string, string>): AnnotationSplit {
  const entries = Object.entries(annotations);
  return {
    shown: entries.filter(([k]) => k !== LAST_APPLIED),
    withheld: entries.filter(([k]) => k === LAST_APPLIED).map(([k]) => k),
  };
}

/**
 * An object's annotations as full-width `key=value` lines, with the applied
 * manifest held back and a line saying where to read it instead.
 *
 * `breakValues` is not decoration: `PairList` truncates by default and no
 * longer writes the value into a row `title`, so wrapping is the only way a
 * long annotation can be read at all.
 *
 * Shared rather than written per body because every kind has this problem —
 * `Pod`, `Deployment`, `StatefulSet` and `ReplicaSet` print their annotations
 * with no gate at all — and a rule about what a pane withholds is worth
 * exactly one implementation. The heading belongs to the caller: this is the
 * inside of a `Section`, not the section.
 */
export function AnnotationLines({ annotations }: { annotations: Record<string, string> }) {
  const { shown, withheld } = partitionAnnotations(annotations);
  return (
    <>
      <PairList pairs={shown} breakValues />
      {withheld.length > 0 && (
        <p className="text-[0.75rem] text-muted">
          {withheld.join(", ")} {withheld.length === 1 ? "is" : "are"} not printed here — the whole manifest
          on one line. The YAML tab shows it in full.
        </p>
      )}
    </>
  );
}

/**
 * The object's labels, as a block of full-width `key=value` lines.
 *
 * `breakValues` is not decoration. `PairList` truncates by default and no
 * longer writes the value into a `title` attribute — that attribute was how a
 * Secret's whole applied manifest reached the DOM — so wrapping is now the
 * only way a long label is readable at all. Omitted outright when the object
 * has none, rather than shown as classic's chip widget does ("None").
 *
 * One implementation, replacing three identical ones (`GenericBody`,
 * `PodBody`, `WorkloadBody`). (#331)
 */
export function LabelsSection({ labels }: { labels: Record<string, string> }) {
  const pairs = Object.entries(labels);
  if (pairs.length === 0) return null;
  return (
    <Section title="Labels">
      <PairList pairs={pairs} breakValues />
    </Section>
  );
}

/**
 * Annotations, collapsed behind an explicit toggle and mounting nothing until
 * expanded — classic's `Expandable`/`ChipMap`.
 *
 * Reached by `Secret` alone; every other kind shows its annotations outright
 * (see {@link AnnotationsSection} below for why the exception is exactly one
 * kind wide). Nothing here uses `title`, `aria-label`, or any `data-*` for a
 * value; the toggle's own accessible name is just its visible "Show"/"Hide"
 * text, counting entries, never naming one.
 *
 * Deliberately not `PairList`, even now that `PairList` writes no `title`:
 * this is the one place a value must be absent from the document rather than
 * merely unshown, and a component that renders its pairs unconditionally
 * cannot promise that.
 */
function AnnotationsToggle({ annotations }: { annotations: Record<string, string> }) {
  const [open, setOpen] = useState(false);
  const entries = Object.entries(annotations);
  return (
    <div className="flex flex-col items-start gap-1">
      <Button type="button" variant="ghost" size="xs" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        {open ? "Hide" : `Show ${plural(entries.length, "annotation")}`}
      </Button>
      {open && (
        <ul className="flex flex-col gap-0.5">
          {entries.map(([k, v]) => (
            <li key={k} className="break-all font-mono text-[0.8125rem]">
              {k}={v}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * The object's annotations — open, the way the design draws them, on every
 * kind but `Secret`.
 *
 * DO NOT "simplify" the exception away. A `kubectl apply`-managed Secret
 * carries its ENTIRE applied manifest, base64 `data` map included, inside the
 * `kubectl.kubernetes.io/last-applied-configuration` annotation, and
 * `k8s.getObject`'s Secret redaction blanks `data`/`stringData` only — it
 * never touches `metadata.annotations`. So for this one kind an annotation
 * value IS the secret, and the toggle is what keeps it out of the document
 * until a reader asks for it, exactly as `SecretBody` keeps each `data` value
 * out until it is revealed.
 *
 * The kit fixed the other half of this: `PairList` used to put every value in
 * a row `title`, so a value the reader saw three characters of was sitting
 * whole in the markup. That fix is why every other kind can now open — an
 * annotation on a ConfigMap or a Deployment holds that object's own spec,
 * which the pane shows anyway — but it does nothing for text that is visible
 * on purpose, which is what a Secret's annotation would be.
 *
 * Every other kind goes through the shared {@link AnnotationLines}, so there
 * is one rule about how annotations print and what a pane holds back. Note
 * what that does NOT do: holding `last-applied-configuration` back is a
 * LEGIBILITY rule — a manifest on one line buries every other annotation in a
 * 352px pane — and it is not redaction. It happens to drop the annotation a
 * Secret's `data` map arrives in, and that is a side effect, not a promise:
 * every other annotation on every kind is still printed exactly as it
 * arrives. The `Secret` branch therefore stays whatever `AnnotationLines`
 * withholds, and a Secret must never be routed through it instead.
 *
 * WHY THIS LIVES HERE AND NOWHERE ELSE. It was written three times —
 * `GenericBody`'s with the `Secret` branch, `PodBody`'s and `WorkloadBody`'s
 * without it. That was safe only by accident: the four kinds those two bodies
 * serve are `SELF_DESCRIBING_KINDS`, none of which can be a Secret. So a
 * security gate rested on a membership list two files away, and adding a
 * fifth kind to that set would have run the ungated copy with nothing
 * failing. `kind` is required rather than optional for the same reason — a
 * caller has to say which kind it is drawing, and cannot get the gate by
 * default. (#331)
 */
export function AnnotationsSection({
  kind,
  annotations,
}: {
  kind: string;
  annotations: Record<string, string>;
}) {
  if (Object.keys(annotations).length === 0) return null;
  return (
    <Section title="Annotations">
      {kind === "Secret" ? (
        <AnnotationsToggle annotations={annotations} />
      ) : (
        <AnnotationLines annotations={annotations} />
      )}
    </Section>
  );
}

interface RelatedPod extends PodSummary {
  cpu?: number;
  memory?: number;
}

/**
 * `Status` reads `podStatus`, NOT `phaseKind(p.phase)`. A pod whose container
 * sits in a back-off loop still reports phase "Running", so a column reading
 * the phase alone drew a crash-looping pod green in a table headed by a
 * Deployment the reader had opened BECAUSE it was degraded. `PodSummary`
 * already carries the waiting reason for exactly this; the list rows and the
 * detail header read the same function. (#331)
 */
const RELATED_POD_COLUMNS: Column<RelatedPod>[] = [
  { key: "name", header: "Name", render: (p) => <span className="font-mono">{p.name}</span> },
  { key: "node", header: "Node", render: (p) => <span className="font-mono">{p.node || "—"}</span> },
  { key: "ready", header: "Ready", render: (p) => p.ready },
  // `formatCpu`/`formatMemory`, the same two the list and the Workloads table
  // render these very fields through. They were formatted twice: one pod read
  // "2 410m" / "3.1 Gi" in the list and "2.410" / "3174 Mi" in the workload's
  // own Pods table, two panes apart. (#331)
  { key: "cpu", header: "CPU", render: (p) => (p.cpu != null ? formatCpu(p.cpu) : "—") },
  { key: "memory", header: "Memory", render: (p) => (p.memory != null ? formatMemory(p.memory) : "—") },
  {
    key: "status",
    header: "Status",
    render: (p) => {
      const status = podStatus(p);
      return <StatusPill status={status.status} kind={status.health} tinted />;
    },
  },
];

/**
 * The pods a workload manages, matched by a label selector — classic's
 * `ManagedPods`. Fetched live via core's `podsForSelector`/`podMetrics`
 * (metrics best-effort, same as classic: a missing metrics-server must not
 * hide the pods). Name and Node are `ResourceLink`s in classic that navigate
 * to the Pod/Node object; here they render as plain mono text — see the task
 * report for the full inert-value list.
 *
 * One implementation, replacing two identical ones: `WorkloadBody` and
 * `GenericBody` each carried their own copy of this and of its column table,
 * and both drew a pod's status from its phase alone. A fix applied to one copy
 * and not the other is how two panels start disagreeing about one pod. (#331)
 *
 * Loading renders inside the `Section`, never beside it: a bare `LoadingState`
 * between two sections breaks the `.section + .section` chain and leaves both
 * gaps unruled.
 */
export function RelatedPodsSection({
  context,
  namespace,
  selector,
}: {
  context: string;
  namespace: string;
  /** The workload's WHOLE `LabelSelector`. Both halves, because a pod is the
   *  workload's only when it satisfies both — see {@link WorkloadSelector}. */
  selector: WorkloadSelector;
}) {
  // Both halves, so a workload whose requirements changed under an unchanged
  // set of equality labels is re-read rather than left showing the pods of
  // the selector before it. A string, so it is the selector's IDENTITY without
  // a new object each render.
  const selectorKey = JSON.stringify([selector.matchLabels, selector.matchExpressions]);
  const state = useSectionList<RelatedPod[]>(true, [context, namespace, selectorKey], async () => {
    const [podsOut, metricsOut] = await Promise.all([
      podsForSelector(context, namespace, selector.matchLabels, selector.matchExpressions),
      // Metrics are best-effort: a missing metrics-server must not hide pods.
      podMetrics(context, namespace).catch((): { metrics?: PodMetric[] } => ({ metrics: [] })),
    ]);
    // The PODS are what this block is about, so only their failure is the
    // block's failure — usage columns simply stay empty.
    if (podsOut.error) return { error: podsOut.error };
    const usage = new Map((metricsOut.metrics ?? []).map((m) => [m.name, m]));
    return {
      data: (podsOut.pods ?? []).map((p) => {
        const m = usage.get(p.name);
        return { ...p, cpu: m?.cpuMillicores, memory: m?.memoryMiB };
      }),
    };
  });

  return (
    // The block STAYS on a refusal, with the reason in it. It used to `return
    // null`, so "this Deployment has no pods" and "srelens was refused" drew
    // the identical screen — see {@link useSectionList}.
    <Section title="Pods">
      {state.status === "loading" ? (
        <LoadingState label="Loading pods" />
      ) : state.status === "error" ? (
        <SectionFailure error={state.error} />
      ) : (
        <Table columns={RELATED_POD_COLUMNS} data={state.data ?? []} getRowKey={(p) => p.name} emptyText="No pods" />
      )}
    </Section>
  );
}

const NODE_POD_LIMIT = 12;
const NODE_POD_AGE_TICK_MS = 30_000;

interface NodePodRow extends PodSummary {
  liveAge: string;
}

const NODE_POD_COLUMNS: Column<NodePodRow>[] = [
  { key: "name", header: "Pod", render: (pod) => <span className="font-mono">{pod.name}</span> },
  {
    key: "namespace",
    header: "Namespace",
    render: (pod) => <span className="font-mono">{pod.namespace || "—"}</span>,
  },
  { key: "liveAge", header: "Age", align: "end", render: (pod) => pod.liveAge },
];

/** Pods scheduled on a Node, queried by `spec.nodeName` across namespaces. */
export function NodePodsSection({ context, node }: { context: string; node: string }) {
  const state = useSectionList<PodSummary[]>(true, [context, node], async () => {
    const out = await podsOnNode(context, node);
    if (out.error) return { error: out.error };
    const pods = [...(out.pods ?? [])].sort(
      (a, b) => a.namespace.localeCompare(b.namespace) || a.name.localeCompare(b.name),
    );
    return { data: pods };
  });
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const tick = setInterval(() => setNow(Date.now()), NODE_POD_AGE_TICK_MS);
    return () => clearInterval(tick);
  }, []);

  const all = state.data ?? [];
  const rows: NodePodRow[] = all.slice(0, NODE_POD_LIMIT).map((pod) => ({
    ...pod,
    liveAge: ageFromTimestamp(pod.createdAt, now),
  }));
  const openPod = (pod: NodePodRow) =>
    openTab(detailRoute("Pod", pod.namespace, pod.name), { clusterName: context });
  const viewAll = () => {
    openTab("/k/pods", { clusterName: context });
    setTabView(currentWorkspace().activeId, { filter: node, filterKey: "node" });
  };

  return (
    <Section title={state.status === "ready" ? `Pods (${all.length})` : "Pods"} id="Pods">
      {state.status === "loading" ? (
        <LoadingState label={`Loading pods on ${node}`} />
      ) : state.status === "error" ? (
        <SectionFailure error={state.error} />
      ) : (
        <>
          <Table
            columns={NODE_POD_COLUMNS}
            data={rows}
            getRowKey={(pod) => `${pod.namespace}/${pod.name}`}
            emptyText="No pods on this node"
            onRowClick={openPod}
            onRowActivate={openPod}
          />
          {all.length > NODE_POD_LIMIT && (
            <div className="mt-2 flex justify-end">
              <Button
                size="xs"
                onClick={viewAll}
                aria-label={`View all ${all.length} pods on ${node}`}
              >
                View all
              </Button>
            </div>
          )}
        </>
      )}
    </Section>
  );
}
