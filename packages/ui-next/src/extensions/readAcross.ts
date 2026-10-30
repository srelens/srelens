import { selectsNothing } from "@srelens/core";
import { scopesFor, type NamespaceFailure } from "../lib/resourceList";

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
