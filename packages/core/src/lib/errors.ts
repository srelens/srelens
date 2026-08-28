/**
 * User-facing error formatting.
 *
 * Backend capability errors reach the UI as raw strings — e.g. the
 * `CapabilityError` Display prefix (`handler error: …`) wrapped around a
 * hand-written message like `list namespaces timed out`. Rendering those
 * verbatim looks broken and tells the user nothing actionable.
 *
 * `describeError` turns a raw error into a short title + an actionable detail,
 * classifying the common cluster-connectivity failure modes. Everything is
 * pure and string-based, so it works for both thrown `Error`s and the
 * `{ error: string }` shapes the lib layer returns.
 */

import { parseClusterLoginRequired } from "./clusterLogin";
import { isTauri } from "../transport/platform";

export interface FriendlyError {
  /** Short, human headline for the failure. */
  title: string;
  /** One or two sentences on what happened and what to check. */
  detail: string;
  /** The original message, cleaned of internal prefixes — kept for diagnostics. */
  raw: string;
}

/** `CapabilityError`'s Display prefix; internal noise the user shouldn't see. */
const HANDLER_PREFIX = /^\s*handler error:\s*/i;

/** Normalize any thrown value to a clean message, stripping internal prefixes. */
export function cleanErrorMessage(input: unknown): string {
  const raw = input instanceof Error ? input.message : String(input ?? "");
  return raw.replace(HANDLER_PREFIX, "").trim();
}

/**
 * Parse an apiserver Forbidden message into an actionable sentence naming the
 * verb, resource, and namespace (or cluster scope). Returns null when the text
 * doesn't match the standard shape.
 */
export function describeForbidden(raw: string): string | null {
  // Split from one pattern into two passes: the alternation of two lazy
  // dot-alls made a message that never completes either branch quadratic
  // (js/polynomial-redos, #49). API error text is not length-bounded.
  const m = /cannot (\w+) resource "([^"]+)"/.exec(raw);
  if (!m) return null;
  const [, verb, resource] = m;
  const rest = raw.slice(m.index + m[0].length);

  // Both markers are checked explicitly, and neither means null. The message
  // has to SAY where it applies: a truncated or aggregated-API error carries
  // the prefix without a scope, and defaulting such a message to "at the
  // cluster scope" would state something the apiserver never said, while
  // hiding the generic RBAC guidance describeError would otherwise give.
  // Namespace is tried first, matching the order of the alternation this
  // replaced, so a message carrying both reads as namespaced.
  const namespace = /in the namespace "([^"]+)"/.exec(rest)?.[1];
  if (namespace) {
    return `You don't have permission to ${verb} ${resource} in ${namespace}.`;
  }
  if (rest.includes("at the cluster scope")) {
    return `You don't have permission to ${verb} ${resource} at the cluster scope.`;
  }
  return null;
}

/**
 * The ServiceAccount's namespace named in a Forbidden error, if the denied user
 * is a service account (`system:serviceaccount:<namespace>:<name>`). This is the
 * namespace such a credential is typically scoped to — useful when the kubeconfig
 * context doesn't declare a namespace.
 */
export function serviceAccountNamespace(error: string): string | null {
  const m = error.match(/system:serviceaccount:([a-z0-9][a-z0-9-]*):/i);
  return m ? m[1] : null;
}

/** True when an error looks like an exec-auth credential plugin failing to run
 *  (a missing kubectl-oidc_login / aws / gke-gcloud-auth-plugin, etc.) — the
 *  case the Toolbox diagnoses and can often fix. */
export function isExecAuthError(input: unknown): boolean {
  const lower = cleanErrorMessage(input).toLowerCase();
  return /auth exec|exec plugin|getting credentials|executable .* not found|exec: .*not found|no such file or directory/.test(
    lower,
  );
}

/** Classify a raw error into a friendly, actionable message. */
export function describeError(input: unknown): FriendlyError {
  const raw = cleanErrorMessage(input);
  const lower = raw.toLowerCase();

  if (isExecAuthError(raw)) {
    // The right guidance differs by platform: desktop can install and run the
    // plugin locally; the web container can't run exec plugins at all, so an
    // OIDC cluster must be re-added with its issuer/client and signed in
    // through the browser instead.
    if (!isTauri()) {
      return {
        title: "This cluster needs OIDC sign-in",
        detail:
          "This context authenticates through an exec plugin (e.g. kubelogin), which can't run in the srelens container. If it's an OIDC cluster, add it under Settings → Contexts → Add cluster with its API server, issuer and client ID, then sign in through your browser. (Non-OIDC plugins like aws or gke-gcloud-auth-plugin need the image extended with the tool and cloud credentials.)",
        raw,
      };
    }
    return {
      title: "Auth plugin couldn't run",
      detail:
        "This context authenticates through an exec credential plugin (e.g. kubectl-oidc_login, aws, or gke-gcloud-auth-plugin) that couldn't be found or run. Open the Toolbox to see exactly which tool is missing and install it.",
      raw,
    };
  }

  if (/timed out|timeout|deadline exceeded/.test(lower)) {
    // The remedy is platform-specific: the Settings slider only exists on the
    // desktop, so pointing a web user at it would be an impossible
    // instruction. The server honours SRELENS_TIMEOUT_SECS instead.
    const raiseIt = isTauri()
      ? "raise Request timeout in Settings → Kubernetes"
      : "ask whoever runs this srelens server to raise its SRELENS_TIMEOUT_SECS setting";
    return {
      title: "Request timed out",
      detail: `The Kubernetes API server didn't respond in time. Large clusters can need longer than the default — ${raiseIt}. If it still times out, check that the cluster is reachable: for a remote cluster confirm your VPN or network connection and that the current context points at the right server.`,
      raw,
    };
  }
  if (/connection refused|failed to connect|connect error|no route to host|network is unreachable|unreachable/.test(lower)) {
    return {
      title: "Can't reach the cluster",
      detail:
        "The connection to the API server was refused. Make sure the cluster is running and the server address in your kubeconfig context is correct.",
      raw,
    };
  }
  if (/no such host|failed to lookup|name or service not known|dns|could not resolve|cannot resolve/.test(lower)) {
    return {
      title: "Cluster address not found",
      detail:
        "The API server hostname couldn't be resolved. Check the server URL in your kubeconfig context and your DNS or network connection.",
      raw,
    };
  }
  if (/cluster_login_required|NEEDS_CLUSTER_LOGIN/.test(raw) || parseClusterLoginRequired(raw)) {
    return {
      title: "Cluster sign-in required",
      detail:
        "This cluster uses OIDC. Use the “Sign in” prompt (or Settings → Contexts) to sign in, then retry.",
      raw,
    };
  }
  if (/unauthorized|\b401\b|invalid bearer|expired token/.test(lower)) {
    return {
      title: "Not authorized",
      detail:
        "The cluster rejected your credentials. Your token or client certificate may have expired — refresh your kubeconfig credentials and try again.",
      raw,
    };
  }
  if (/forbidden|\b403\b/.test(lower)) {
    return {
      title: "Access denied",
      detail:
        describeForbidden(raw) ??
        "Your account doesn't have permission for this on the cluster. Check your RBAC roles, or switch to a context with the right access.",
      raw,
    };
  }
  if (/certificate|x509|\btls\b|self.signed|unknown authority/.test(lower)) {
    return {
      title: "Couldn't verify the cluster",
      detail:
        "The cluster's TLS certificate couldn't be verified. It may be self-signed or expired, or the certificate-authority data in your kubeconfig may be missing or wrong.",
      raw,
    };
  }

  return {
    title: "Something went wrong",
    detail: raw || "An unexpected error occurred.",
    raw,
  };
}
