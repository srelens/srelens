/**
 * Pure mapper: given a resource action description, produce the equivalent
 * `kubectl` command string. This is presentational only — it does not execute
 * anything, just generates the CLI equivalent for display/copy.
 */

import { kindToResource } from "./access";

export interface KubectlInput {
  action:
    | "get"
    | "describe"
    | "delete"
    | "scale"
    | "rollout-restart"
    | "cordon"
    | "uncordon"
    | "drain"
    | "cronjob-suspend"
    | "cronjob-resume"
    | "cronjob-trigger";
  kind: string;
  name: string;
  context: string;
  namespace?: string | null;
  /** For get commands: output format (e.g. "yaml", "json"). */
  output?: string;
  /** For scale: target replica count. */
  replicas?: number;
}

// Resource names are DNS-1123 and always shell-safe in practice, but
// kubeconfig context names are user-chosen and can contain spaces or other
// shell-active characters. Quote anything outside the safe unquoted set
// rather than assuming kubeconfig hygiene. Double quotes (not single) —
// single quotes aren't a quoting character in cmd.exe, so a single-quoted
// value would still split on spaces there (we ship Windows builds).
const SAFE_UNQUOTED = /^[A-Za-z0-9._@:/-]+$/;
function shellQuote(value: string): string {
  return SAFE_UNQUOTED.test(value) ? value : `"${value.replace(/[\\"]/g, "\\$&")}"`;
}

/**
 * Convert a resource/action description into the equivalent kubectl command.
 *
 * `evict` has no single-line kubectl equivalent — the Eviction API has no
 * dedicated CLI verb, unlike a plain delete — so it's intentionally excluded
 * from `action`. Callers show an explanatory note instead of a command (see
 * `KubectlPreview`'s `note` prop).
 */
export function toKubectl(input: KubectlInput): string {
  const { action, kind, name, context, namespace, output, replicas } = input;
  // Prefer the authoritative kind→resource table (mirrors the backend's own
  // GVR mapping) over a bare lowercase, which drifts for kinds whose plural
  // isn't just "+s" (Ingress → ingresses). Falls back to lowercasing for
  // CRDs/unknown kinds not in the table.
  const kindLower = kindToResource(kind)?.resource ?? kind.toLowerCase();
  const ns = namespace || "";

  const parts: string[] = ["kubectl"];

  // Kubernetes names are DNS-1123 and always fall in SAFE_UNQUOTED, so this
  // is a no-op for every real name today — quoting it anyway is cheap
  // defense-in-depth and matches "name" being one of the three values the
  // review called out, alongside namespace and context.
  const qName = shellQuote(name);

  switch (action) {
    case "get":
    case "describe":
      parts.push(action, kindLower, qName);
      break;

    case "delete":
      parts.push("delete", kindLower, qName);
      break;

    case "scale":
      parts.push("scale", `${kindLower}/${qName}`, `--replicas=${replicas}`);
      break;

    case "rollout-restart":
      parts.push("rollout", "restart", `${kindLower}/${qName}`);
      break;

    case "cordon":
      parts.push("cordon", qName);
      break;

    case "uncordon":
      parts.push("uncordon", qName);
      break;

    case "drain":
      // --force covers the unmanaged bare pods the backend also evicts,
      // which plain `kubectl drain` refuses to touch without it.
      parts.push("drain", qName, "--ignore-daemonsets", "--delete-emptydir-data", "--force");
      break;

    case "cronjob-suspend":
      // Double-quoted with escaped inner quotes, not single-quote wrapped:
      // this form is valid in bash/zsh/PowerShell *and* cmd.exe.
      parts.push("patch", "cronjob", qName, "-p", '"{\\"spec\\":{\\"suspend\\":true}}"');
      break;

    case "cronjob-resume":
      parts.push("patch", "cronjob", qName, "-p", '"{\\"spec\\":{\\"suspend\\":false}}"');
      break;

    case "cronjob-trigger":
      // The app suffixes the Job name with Date.now() (crates/kube/src/cronjobs.rs),
      // so the previewed name can't match exactly; $(date +%s) at least keeps
      // a copy-pasted re-run from colliding with AlreadyExists the way a
      // fixed "-manual" suffix would. Left unquoted (unlike qName above): the
      // composed token intentionally carries shell syntax that quoting would
      // neutralize.
      parts.push("create", "job", `--from=cronjob/${qName}`, `${name}-manual-$(date +%s)`);
      break;
  }

  if (ns) {
    parts.push("-n", shellQuote(ns));
  }

  parts.push("--context", shellQuote(context));

  if (output) {
    parts.push("-o", output);
  }

  return parts.join(" ");
}
