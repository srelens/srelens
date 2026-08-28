import { useEffect, useState } from "react";
import { listNamespaces } from "./workloads";
import { listContexts } from "./clusters";
import { serviceAccountNamespace } from "./errors";
import { isForbidden } from "./access";

export interface NamespaceOptions {
  /** null while loading. */
  namespaces: string[] | null;
  /** Non-empty when restricted credentials forced a single-namespace scope. */
  scope: string;
  /** Non-fatal error to surface (a real failure, not an RBAC restriction). */
  error: string;
}

/**
 * Load the namespace options for a context, with a subtle Forbidden fallback:
 * on a Forbidden `listNamespaces`, scope the view to a single namespace —
 * preferring the namespace the context's kubeconfig entry declares, else the
 * ServiceAccount namespace parsed out of the Forbidden error. On a
 * NON-Forbidden error, surface the real error and do NOT auto-scope.
 */
export function useNamespaceOptions(context: string, kubeconfigFiles: string[]): NamespaceOptions {
  const [namespaces, setNamespaces] = useState<string[] | null>(null);
  const [nsError, setNsError] = useState("");
  // Set when namespace listing was forbidden but the kubeconfig context
  // declares a bound namespace — the view is scoped to it instead of "all".
  const [nsScope, setNsScope] = useState("");
  // Stable dependency for the effect below: a fresh `kubeconfigFiles` array
  // identity every render must not refire the effect; only a real change to
  // the set of files should.
  const kubeconfigFilesKey = kubeconfigFiles.join(" ");

  useEffect(() => {
    let active = true;
    setNamespaces(null);
    setNsError("");
    setNsScope("");
    // Ensure the client cache knows about all configured kubeconfig files (incl.
    // pasted/additional) before we build a client for this context. Otherwise a
    // restored tab for a context from an additional file races the app's initial
    // listContexts and fails with "failed to load current context". We only need
    // the side effect (cache.set_paths); ignore the return, and if it errors
    // still proceed to listNamespaces (which surfaces its own error).
    const ready = kubeconfigFiles.length
      ? listContexts(kubeconfigFiles).then(() => undefined)
      : Promise.resolve(undefined);
    void ready.then(() => {
      if (!active) return;
      return listNamespaces(context).then((outcome) => {
      if (!active) return;
      if (outcome.error && isForbidden(outcome.error)) {
        // Forbidden — a namespace-restricted credential. Scope the view to a
        // single namespace instead of falling back to "all namespaces" (which
        // would just 403 again). Prefer the namespace the context's kubeconfig
        // entry declares; if it declares none, fall back to the ServiceAccount's
        // namespace named in the Forbidden error itself.
        void listContexts(kubeconfigFiles)
          .then((ctxOutcome) => {
            if (!active) return null;
            return ctxOutcome.contexts?.find((c) => c.name === context)?.namespace?.trim();
          })
          .catch(() => undefined)
          .then((declared) => {
            if (!active) return;
            const ns = declared || serviceAccountNamespace(outcome.error!);
            if (ns) {
              setNsError("");
              setNsScope(ns);
              setNamespaces([ns]);
            } else {
              setNsScope("");
              setNsError(outcome.error!);
              setNamespaces([]); // non-fatal: still render the toolbar + resource list
            }
          });
      } else if (outcome.error) {
        // A genuine failure (timeout, 5xx) — NOT a permission problem. Keep it
        // non-fatal (render the view against all namespaces), but surface the
        // REAL error rather than mischaracterizing it as an RBAC restriction,
        // and do NOT auto-scope to the context's declared namespace.
        setNsScope("");
        setNsError(outcome.error);
        setNamespaces([]);
      } else {
        setNsError("");
        setNsScope("");
        setNamespaces(outcome.namespaces ?? []);
      }
      // namespace stays "" = All namespaces by default
      });
    });
    return () => {
      active = false;
    };
  }, [context, kubeconfigFilesKey]);

  return { namespaces, scope: nsScope, error: nsError };
}
