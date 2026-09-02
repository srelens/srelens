import {
  asRecord,
  formatTaint,
  orderTaints,
  parseTaints,
  str,
  taintTimeAddedText,
  type K8sObject,
} from "@srelens/core";
import { KV, StatusPill } from "@srelens/ui-kit";
import { Section } from "./Section";

/**
 * A Node's runtime identity — classic's "Info" section, ported fact-for-fact:
 * whether the node is schedulable, its kubelet, OS image, kernel, container
 * runtime and CPU architecture. Node conditions (Ready, MemoryPressure, ...)
 * come from `GenericBody`'s Conditions block, not from here.
 */
function InfoSection({ object }: { object: K8sObject }) {
  const spec = asRecord(object.spec);
  const status = asRecord(object.status);
  const info = asRecord(status.nodeInfo);
  const cordoned = spec.unschedulable === true;

  return (
    <Section title="Info">
      <KV
        k="Scheduling"
        v={
          <StatusPill
            status={cordoned ? "Disabled (cordoned)" : "Enabled"}
            kind={cordoned ? "warning" : "success"}
          />
        }
      />
      <KV k="Kubelet" v={str(info.kubeletVersion)} />
      <KV k="OS image" v={str(info.osImage)} />
      <KV k="Kernel" v={str(info.kernelVersion)} />
      <KV k="Container runtime" v={str(info.containerRuntimeVersion)} />
      <KV k="Architecture" v={str(info.architecture)} />
    </Section>
  );
}

/**
 * A Node's capacity — classic's "Capacity" section: CPU, memory and pod
 * counts, each shown as "allocatable / capacity" the way classic does.
 */
function CapacitySection({ object }: { object: K8sObject }) {
  const status = asRecord(object.status);
  const capacity = asRecord(status.capacity);
  const allocatable = asRecord(status.allocatable);

  return (
    <Section title="Capacity">
      <KV k="CPU" v={`${str(allocatable.cpu)} / ${str(capacity.cpu)}`} />
      <KV k="Memory" v={`${str(allocatable.memory)} / ${str(capacity.memory)}`} />
      <KV k="Pods" v={`${str(allocatable.pods)} / ${str(capacity.pods)}`} />
    </Section>
  );
}

/**
 * A Node's taints, in full — the drill-down the list's count sends a reader to.
 *
 * Unlike the list, this reads the live object and keeps EVERY taint, including
 * the one Kubernetes adds when a node is cordoned. The list leaves that one out
 * because the SchedulingDisabled badge beside its count already says it; here
 * there is no such badge, and `kubectl describe node` shows it too, so leaving
 * it out would be the pane quietly disagreeing with kubectl.
 *
 * The section stays on a node with none and says so: an absent block cannot be
 * told apart from a block that failed to render, and "no taints" is a fact an
 * operator asking why a pod will not schedule specifically wants confirmed.
 *
 * `timeAdded` is set by Kubernetes only for `NoExecute` taints; the rest say
 * so in words, which `taintTimeAddedText` explains is not merely a wording
 * choice. (#426)
 */
function TaintsSection({ object }: { object: K8sObject }) {
  const taints = orderTaints(parseTaints(object.spec));
  return (
    <Section title="Taints">
      {taints.length === 0 ? (
        // "Taints" as the key under a section already headed Taints is the
        // word twice; the count is the fact, and it is the list's zero-state.
        <KV k="Count" v="0" />
      ) : (
        taints.map((taint) => (
          <KV
            key={`${taint.key}:${taint.effect}`}
            k={formatTaint(taint)}
            v={taintTimeAddedText(taint)}
            mono
          />
        ))
      )}
    </Section>
  );
}

/**
 * A Node's Details pane: Info and Capacity, in classic's own order
 * (`NodeBody`). `relatedPodSelector` has no case for "Node", so `GenericBody`
 * fetches no related pods for one; its unheaded identity block and its
 * Conditions, Labels and Annotations blocks still wrap this body, which is
 * why none of them is repeated here.
 *
 * Both blocks are returned as siblings, never wrapped: `.section + .section`
 * is what draws the hairline between two blocks, so an element around either
 * of them would take the rule out on both sides of it.
 */
export function NodeDetailsBody({ object }: { object: K8sObject }) {
  return (
    <>
      <InfoSection object={object} />
      <CapacitySection object={object} />
      <TaintsSection object={object} />
    </>
  );
}
