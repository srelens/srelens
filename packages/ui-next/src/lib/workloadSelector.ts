import { asArray, asRecord, type LabelSelectorRequirement } from "@srelens/core";

/**
 * Reading a workload's `LabelSelector` — the one question "which pods are
 * this workload's?" always comes down to.
 *
 * It lived inside `logSubject` when only the Logs screen asked it, and the
 * resource detail screen answered it separately by reading `matchLabels`
 * alone — the very half-answer this module exists to prevent. Whose pods a
 * workload owns is not a logs concern, so the reader sits here, once, and
 * both screens call it.
 */

/**
 * A workload's whole `LabelSelector`, both halves.
 *
 * `matchExpressions` is not optional detail: a pod is owned by the workload
 * only when it satisfies the equality labels **and** every expression, so
 * reading `matchLabels` alone is wrong twice over — a workload selected
 * entirely by expressions resolves to an empty selector, which the backend
 * deliberately answers with no pods (an empty selector would otherwise match
 * the whole namespace), and a workload with both halves resolves to a
 * selector broader than the real one, which names pods the workload never
 * owned.
 */
export interface WorkloadSelector {
  matchLabels: Record<string, string>;
  matchExpressions: LabelSelectorRequirement[];
}

/**
 * The selector on a workload's spec, read loosely so a spec that doesn't have
 * one just yields no pods.
 *
 * Requirements are passed on as they were found — the key and operator
 * verbatim, values narrowed to the strings among them. Nothing here corrects
 * an operator's spelling or drops a requirement it doesn't recognise: the
 * backend renders selectors, and it refuses one it cannot render. An error is
 * a far better outcome than a corrected selector, which is simply a different
 * selector naming different pods.
 */
export function selectorOf(object: unknown): WorkloadSelector {
  const spec = asRecord(asRecord(object).spec);
  const selector = asRecord(spec.selector);
  const matchLabels = (selector.matchLabels ?? {}) as Record<string, string>;
  const matchExpressions = asArray(selector.matchExpressions).map((entry) => {
    const requirement = asRecord(entry);
    return {
      key: typeof requirement.key === "string" ? requirement.key : "",
      operator: typeof requirement.operator === "string" ? requirement.operator : "",
      values: asArray(requirement.values).filter((v): v is string => typeof v === "string"),
    };
  });
  return { matchLabels, matchExpressions };
}

/**
 * Whether the selector names anything at all — EITHER half is enough.
 *
 * What the panels gate on: a selector's absence means there are no pods to
 * ask for, and a workload written entirely in expressions has a selector like
 * any other. Gating on `matchLabels` alone is how such a workload lost its
 * Pods panel outright rather than merely showing it empty.
 */
export function hasSelector(selector: WorkloadSelector): boolean {
  return Object.keys(selector.matchLabels).length > 0 || selector.matchExpressions.length > 0;
}

/**
 * One requirement in Kubernetes' own selector syntax — `app in (web, api)`,
 * `track notin (canary)`, `logging`, `!legacy` — for a reader's eyes only.
 *
 * DISPLAY ONLY. The requirement itself still travels to the backend exactly
 * as it was found; this never becomes a query. An operator this does not
 * recognise is printed as it was spelled rather than dropped or corrected,
 * for the same reason the reader above passes it through: the row must show
 * what the object actually says, including the part the cluster will refuse.
 */
export function requirementText(requirement: LabelSelectorRequirement): string {
  const values = (requirement.values ?? []).join(", ");
  switch (requirement.operator) {
    case "Exists":
      return requirement.key;
    case "DoesNotExist":
      return `!${requirement.key}`;
    case "In":
      return `${requirement.key} in (${values})`;
    case "NotIn":
      return `${requirement.key} notin (${values})`;
    default:
      return values ? `${requirement.key} ${requirement.operator} (${values})` : `${requirement.key} ${requirement.operator}`;
  }
}
