import { selectsNothing, type InstalledExtension } from "@srelens/core";
import { scopesFor, type NamespaceFailure } from "../lib/resourceList";

/**
 * Whether a reader lists a cluster-scoped kind. A built-in reader's arguments
 * are empty, so `k8s.listNodes` is named here as the host's
 * `builtin_reader_identity` names it.
 */
export function readsClusterScope(plugin: InstalledExtension, capability: string): boolean {
  const binding = plugin.manifest.capabilities.find((candidate) => candidate.name === capability);
  return binding?.target === "k8s.listNodes" || binding?.arguments.namespaced === false;
}

/** The namespaces a reader is asked about: none for a cluster-scoped kind, which has one answer. */
export const readerNamespaces = (plugin: InstalledExtension, capability: string, namespaces: string[]) =>
  readsClusterScope(plugin, capability) ? [] : namespaces;

/**
 * One read per selected namespace, as a built-in list does (#688): a credential
 * scoped to a few namespaces is refused the cluster-wide read that narrowing
 * afterwards would need. The namespaces that answered are the list, and the
 * rest are named failures. Only when none answered does the read itself fail.
 */
export async function readAcross<T>(
  namespaces: string[],
  read: (namespace: string) => Promise<T>,
): Promise<{ answers: T[]; failures: NamespaceFailure[] }> {
  if (selectsNothing(namespaces)) return { answers: [], failures: [] };
  const scopes = scopesFor(namespaces);
  const settled = await Promise.allSettled(scopes.map((namespace) => read(namespace)));
  const answers = settled.flatMap((result) => (result.status === "fulfilled" ? [result.value] : []));
  const refused = settled.flatMap((result, i) => (result.status === "rejected" ? [{ namespace: scopes[i], reason: result.reason as unknown }] : []));
  if (answers.length === 0) throw refused[0].reason;
  return {
    answers,
    failures: refused.map(({ namespace, reason }) => ({ namespace, error: reason instanceof Error ? reason.message : String(reason) })),
  };
}
