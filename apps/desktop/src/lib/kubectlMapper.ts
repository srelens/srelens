/**
 * Pure mapper: given a resource action description, produce the equivalent
 * `kubectl` command string. This is presentational only — it does not execute
 * anything, just generates the CLI equivalent for display/copy.
 */

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
    | "evict"
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

/**
 * Convert a resource/action description into the equivalent kubectl command.
 */
export function toKubectl(input: KubectlInput): string {
  const { action, kind, name, context, namespace, output, replicas } = input;
  const kindLower = kind.toLowerCase();
  const ns = namespace || "";

  const parts: string[] = ["kubectl"];

  switch (action) {
    case "get":
    case "describe":
      parts.push(action, kindLower, name);
      break;

    case "delete":
      parts.push("delete", kindLower, name);
      break;

    case "scale":
      parts.push("scale", `${kindLower}/${name}`, `--replicas=${replicas}`);
      break;

    case "rollout-restart":
      parts.push("rollout", "restart", `${kindLower}/${name}`);
      break;

    case "cordon":
      parts.push("cordon", name);
      break;

    case "uncordon":
      parts.push("uncordon", name);
      break;

    case "drain":
      parts.push("drain", name, "--ignore-daemonsets", "--delete-emptydir-data");
      break;

    case "evict":
      parts.push("delete", "pod", name, "--grace-period=0");
      break;

    case "cronjob-suspend":
      parts.push(
        "patch",
        "cronjob",
        name,
        "-p",
        "'{\"spec\":{\"suspend\":true}}'",
      );
      break;

    case "cronjob-resume":
      parts.push(
        "patch",
        "cronjob",
        name,
        "-p",
        "'{\"spec\":{\"suspend\":false}}'",
      );
      break;

    case "cronjob-trigger":
      parts.push("create", "job", `--from=cronjob/${name}`, `${name}-manual`);
      break;
  }

  if (ns) {
    parts.push("-n", ns);
  }

  parts.push("--context", context);

  if (output) {
    parts.push("-o", output);
  }

  return parts.join(" ");
}
