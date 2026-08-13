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
// rather than assuming kubeconfig hygiene.
const SAFE_UNQUOTED = /^[A-Za-z0-9._@:/-]+$/;

// Characters that stay ACTIVE inside double quotes on POSIX platforms and so
// can execute code when the copied command is pasted: `$` and backtick
// (command/variable substitution in bash/zsh AND PowerShell), `!` (history
// expansion in interactive bash/zsh), plus `"` and `\` (would terminate or
// re-arm the quoting itself). Any of these forces the single-quote tier.
const POSIX_DOUBLE_QUOTE_UNSAFE = /[$`!"\\]/;

// The Windows equivalent, covering BOTH shells a value pasted there can hit:
// inside double quotes cmd.exe still expands `%VAR%`, treats `"` as the
// closing quote, and un-escapes a trailing `\"`; PowerShell still expands `$`
// and backtick; `!` expands under cmd's delayed expansion. A value carrying
// any of these has NO representation that is inert in both shells — cmd has
// no single-quote syntax at all (so `&` would chain commands right through
// single quotes) — and gets the placeholder tier instead of a quoting
// pretense.
const WINDOWS_DOUBLE_QUOTE_UNSAFE = /[$`!"%\\]/;

const IS_WINDOWS = typeof navigator !== "undefined" && navigator.userAgent.includes("Windows");

/**
 * Quote one value for the platform's paste targets, or refuse: `what` names
 * the value ("context", "namespace", "name") for the fill-in placeholder
 * emitted when a hostile value cannot be represented safely on Windows.
 */
function shellQuote(value: string, what: string, windows: boolean): string {
  if (SAFE_UNQUOTED.test(value)) return value;
  if (windows) {
    // Inside double quotes, cmd.exe treats & | < > ^ ( ) and spaces literally
    // — and so do PowerShell and the POSIX shells — so this tier is inert in
    // every shell a Windows user pastes into. The common "name with a space"
    // case lands here and stays copy-paste-runnable.
    if (!WINDOWS_DOUBLE_QUOTE_UNSAFE.test(value)) return `"${value}"`;
    // No safe representation exists (see WINDOWS_DOUBLE_QUOTE_UNSAFE): emit
    // a fill-in placeholder so the pasted command errors asking for the
    // value rather than ever executing part of a hostile one. Angle brackets
    // are redirection in cmd but sit inside double quotes here.
    return `"<enter ${what}>"`;
  }
  // POSIX: odd but inert values (spaces, parens, unicode) keep double quotes;
  // anything carrying expansion syntax gets single quotes, inside which
  // bash/zsh (and PowerShell) expand nothing.
  if (!POSIX_DOUBLE_QUOTE_UNSAFE.test(value)) return `"${value}"`;
  // ...unless the value ALSO contains an apostrophe: the POSIX '\'' escape
  // is bash-only — PowerShell (a real paste target on macOS/Linux too)
  // doesn't treat backslash as a quote escape, so the text between two
  // embedded apostrophes lands OUTSIDE any string there and $() would
  // execute. Expansion syntax + apostrophe has no representation inert in
  // bash/zsh AND pwsh at once → same placeholder refusal as the Windows
  // tier.
  if (value.includes("'")) return `"<enter ${what}>"`;
  return `'${value}'`;
}

/**
 * Convert a resource/action description into the equivalent kubectl command.
 *
 * `evict` has no single-line kubectl equivalent — the Eviction API has no
 * dedicated CLI verb, unlike a plain delete — so it's intentionally excluded
 * from `action`. Callers show an explanatory note instead of a command (see
 * `KubectlPreview`'s `note` prop).
 */
export function toKubectl(input: KubectlInput, windows: boolean = IS_WINDOWS): string {
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
  const qName = shellQuote(name, "name", windows);

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
    parts.push("-n", shellQuote(ns, "namespace", windows));
  }

  parts.push("--context", shellQuote(context, "context", windows));

  if (output) {
    parts.push("-o", output);
  }

  return parts.join(" ");
}
