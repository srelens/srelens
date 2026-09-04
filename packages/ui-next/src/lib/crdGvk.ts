import { K8S_KIND, listCrds, type DynamicGvk } from "@srelens/core";

/** The list slug for each built-in Kubernetes kind — `Deployment` to
 *  `deployments` — which is also the set of kinds the backend resolves on its
 *  own. */
export const SLUG_BY_K8S_KIND: Record<string, string> = Object.fromEntries(
  Object.entries(K8S_KIND)
    .filter(([, k8sKind]) => k8sKind !== "")
    .map(([slug, k8sKind]) => [k8sKind, slug]),
);

/** Whether `k8s.getManifest` can find this kind without being told its group. */
export function isBuiltInKind(kind: string): boolean {
  return SLUG_BY_K8S_KIND[kind] !== undefined;
}

/**
 * The group, version and plural behind a custom kind, from the cluster's
 * CustomResourceDefinitions.
 *
 * `k8s.getManifest` resolves the built-in kinds itself and needs to be TOLD
 * about any other, so a screen that reads a custom resource's manifest has to
 * look this up first — the detail pane's YAML view did, and the editor opened
 * every custom resource to an error until it did the same. Shared here so the
 * two cannot drift.
 *
 * Refuses rather than guesses. A kind claimed by two CRDs in different
 * groups is real — two operators, one noun — and picking either would fetch
 * a manifest from possibly the wrong group and render it as though it were
 * the right one: a possibly-wrong success, which is worse than a failure,
 * because nothing on screen would say anything was ambiguous.
 */
export async function resolveCrdGvk(
  context: string,
  kind: string,
): Promise<{ crd?: DynamicGvk; error?: string }> {
  const result = await listCrds(context);
  if (result.error) {
    return { error: `Could not look up ${kind}'s CustomResourceDefinition: ${result.error}` };
  }
  const matches = result.crds?.filter((c) => c.kind === kind) ?? [];
  if (matches.length === 0) {
    return {
      error: `${kind} has no matching CustomResourceDefinition on this cluster, so its manifest cannot be resolved.`,
    };
  }
  if (matches.length > 1) {
    // Sorted and de-duplicated so the message reads the same whichever order
    // `listCrds` happened to return them in.
    const groups = [...new Set(matches.map((c) => c.group))].sort().join(", ");
    return {
      error: `${kind} is claimed by more than one CustomResourceDefinition on this cluster (${groups}), so its manifest cannot be resolved unambiguously.`,
    };
  }
  const match = matches[0];
  return { crd: { group: match.group, version: match.version, plural: match.plural } };
}
