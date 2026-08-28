import { asRecord, asArray, str } from "./k8sRaw";
import type { K8sObject } from "./manifest";

/**
 * One line per affinity type in use, e.g. "Node affinity: 2 required, 1
 * preferred". `nodeAffinity` counts `nodeSelectorTerms`; pod (anti-)affinity
 * count their rule arrays directly. Types with no rules are omitted.
 */
export function summarizeAffinity(affinity: Record<string, unknown>): string[] {
  const lines: string[] = [];
  const describe = (label: string, rule: Record<string, unknown>, requiredIsTerms: boolean) => {
    const required = requiredIsTerms
      ? asArray(asRecord(rule.requiredDuringSchedulingIgnoredDuringExecution).nodeSelectorTerms).length
      : asArray(rule.requiredDuringSchedulingIgnoredDuringExecution).length;
    const preferred = asArray(rule.preferredDuringSchedulingIgnoredDuringExecution).length;
    if (required === 0 && preferred === 0) return;
    const parts: string[] = [];
    if (required) parts.push(`${required} required`);
    if (preferred) parts.push(`${preferred} preferred`);
    lines.push(`${label}: ${parts.join(", ")}`);
  };
  describe("Node affinity", asRecord(affinity.nodeAffinity), true);
  describe("Pod affinity", asRecord(affinity.podAffinity), false);
  describe("Pod anti-affinity", asRecord(affinity.podAntiAffinity), false);
  return lines;
}

/**
 * The facts of a workload's update strategy: its type, and whichever of
 * `partition` / `maxSurge` / `maxUnavailable` the object actually sets.
 *
 * FACTS, NOT A SENTENCE, and deliberately so. Two designs read this — classic
 * draws "RollingUpdate (max unavailable 1)", the new design draws
 * "RollingUpdate · unavailable 1" off its mock — and a shared function that
 * returned one of those strings would put one design's typography inside the
 * other. It did, briefly: moving classic's helper into core and then restyling
 * it for the mock silently retyped a frozen app's Update strategy rows. So the
 * numbers are read once, here, and the words are chosen at each edge.
 *
 * Every field is the value as written (`"25%"`, `"1"`), not a number: a surge
 * or an unavailable may legally be a percentage string, and parsing one into a
 * number would have to invent a base to resolve it against. Absent stays
 * absent — `undefined` is "the object does not set this", distinct from a
 * `maxUnavailable: 0`, which is a real and strict setting ("take nothing down
 * while rolling") that a truthiness test would drop.
 */
export interface UpdateStrategy {
  /** `type`, defaulting to `RollingUpdate` as the API server does. */
  type: string;
  /** `rollingUpdate.partition` — a StatefulSet's staged-rollout cutoff. */
  partition?: string;
  /** `rollingUpdate.maxSurge` — extra pods allowed above the desired count. */
  maxSurge?: string;
  /** `rollingUpdate.maxUnavailable` — pods allowed down during a rollout. */
  maxUnavailable?: string;
}

/**
 * Read a `spec.strategy` / `spec.updateStrategy` into its facts.
 *
 * One reader for every workload kind: a Deployment's `strategy` and a
 * StatefulSet's or DaemonSet's `updateStrategy` are the same shape under
 * different names, and only the caller knows which field to hand over.
 */
export function updateStrategy(strategy: Record<string, unknown>): UpdateStrategy {
  const ru = asRecord(strategy.rollingUpdate);
  // `!= null` and not a truthiness test: `maxUnavailable: 0` is a real
  // setting — take nothing down while rolling — and the strictest one there is.
  const at = (key: string) => (ru[key] != null ? str(ru[key]) : undefined);
  return {
    type: str(strategy.type) || "RollingUpdate",
    partition: at("partition"),
    maxSurge: at("maxSurge"),
    maxUnavailable: at("maxUnavailable"),
  };
}

export function relatedPodSelector(kind: string, obj: K8sObject): Record<string, string> {
  const spec = asRecord(obj.spec);
  switch (kind) {
    case "Service":
      return asRecord(spec.selector) as Record<string, string>;
    case "DaemonSet":
    case "Job":
      return asRecord(asRecord(spec.selector).matchLabels) as Record<string, string>;
    case "PodDisruptionBudget":
      return asRecord(asRecord(spec.selector).matchLabels) as Record<string, string>;
    case "NetworkPolicy":
      return asRecord(asRecord(spec.podSelector).matchLabels) as Record<string, string>;
    default:
      return {};
  }
}
