import {
  asArray,
  asRecord,
  formatStorageSize,
  formatTaint,
  orderTaints,
  parseTaints,
  str,
  taintTimeAddedText,
  type K8sObject,
} from "@srelens/core";
import { EmptyState, KV, StatusPill } from "@srelens/ui-kit";
import { Section } from "./Section";
import { NodePodsSection } from "./sections";

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

  const addresses = asArray(status.addresses).map(asRecord);
  const internalIps = addresses
    .filter((a) => str(a.type) === "InternalIP")
    .map((a) => str(a.address))
    .filter(Boolean);
  const externalIps = addresses
    .filter((a) => str(a.type) === "ExternalIP")
    .map((a) => str(a.address))
    .filter(Boolean);
  const hostnames = addresses
    .filter((a) => str(a.type) === "Hostname")
    .map((a) => str(a.address))
    .filter(Boolean);

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
      <KV k="Internal IP" v={internalIps.join(", ")} mono />
      {externalIps.length > 0 && <KV k="External IP" v={externalIps.join(", ")} mono />}
      {hostnames.length > 0 && <KV k="Hostname" v={hostnames.join(", ")} mono />}
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
      <KV k="Memory" v={`${formatStorageSize(str(allocatable.memory))} / ${formatStorageSize(str(capacity.memory))}`} />
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
 *
 * Rows and not `KV`s: a taint is data, and the key column clipped a long one at
 * a third of the pane. (#1001)
 */
function TaintsSection({ object }: { object: K8sObject }) {
  const taints = orderTaints(parseTaints(object.spec));
  return (
    <Section title="Taints">
      {taints.length === 0 ? (
        <EmptyState compact title="No taints" />
      ) : (
        <ul className="flex flex-col gap-1 text-[0.75rem]">
          {taints.map((taint) => (
            <li key={`${taint.key}:${taint.effect}`} className="flex flex-wrap items-baseline gap-x-3">
              <span className="font-mono [overflow-wrap:anywhere]">{formatTaint(taint)}</span>
              <span className="text-muted">{taintTimeAddedText(taint)}</span>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

/**
 * A Node's Overview pane: Info and Capacity, in classic's own order
 * (`NodeBody`), its Taints, then the pods scheduled on it, which only a cluster
 * can answer. `relatedPodSelector` has no case for "Node", so `GenericBody`
 * fetches no related pods for one; its unheaded identity block and its
 * Conditions, Labels and Annotations blocks still wrap this body, which is
 * why none of them is repeated here.
 *
 * Every block is returned as a sibling, never wrapped: `.section + .section`
 * is what draws the hairline between two blocks, so an element around any of
 * them would take the rule out on both sides of it.
 */
export function NodeDetailsBody({ object, context }: { object: K8sObject; context?: string }) {
  return (
    <>
      <InfoSection object={object} />
      <CapacitySection object={object} />
      <TaintsSection object={object} />
      {context && <NodePodsSection context={context} node={str(object.metadata?.name)} />}
    </>
  );
}
