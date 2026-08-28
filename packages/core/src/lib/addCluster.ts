// Cross-platform "Add cluster" + "Test connection", used by the Add-cluster
// form on both desktop and web.
//
// - Web stores the cluster server-side (POST /api/clusters) where a
//   srelens-managed OIDC token authenticates it.
// - Desktop synthesizes the kubeconfig (reusing the same Rust synthesis via a
//   capability, so output is identical) and saves it under the app config dir;
//   the user's native kubelogin/oidc-login plugin authenticates it.
//
// Both surfaces test-connect through capabilities (`k8s.synthesizeCluster‑
// Kubeconfig` + `k8s.testClusterConnection`), which run identically via the
// Tauri command bridge on desktop and `/api/capability/:id` on web.
import { isTauri } from "../transport/platform";
import { invokeCapability } from "../transport/transport";
import { savePastedKubeconfig } from "./files";
import { createCluster as createClusterWeb, type CreateClusterInput } from "./webClusters";
import { parse as parseYaml } from "yaml";

export type { CreateClusterInput };

export interface TestResult {
  reachable: boolean;
  version?: string | null;
  error?: string | null;
}

/**
 * Add a cluster from form fields. On web the server stores it and returns
 * nothing; on desktop the synthesized kubeconfig is saved locally and its file
 * path is returned so the caller can track it in the kubeconfig source list.
 */
export async function addCluster(input: CreateClusterInput): Promise<string | undefined> {
  if (!isTauri()) {
    await createClusterWeb(input);
    return undefined;
  }
  const { yaml } = await invokeCapability<{ yaml: string }>(
    "k8s.synthesizeClusterKubeconfig",
    input,
  );
  return savePastedKubeconfig(yaml, input.name.trim());
}

/** Test-connect a form-defined cluster before saving: synthesize, then probe. */
export async function testClusterForm(input: CreateClusterInput): Promise<TestResult> {
  const { yaml } = await invokeCapability<{ yaml: string }>(
    "k8s.synthesizeClusterKubeconfig",
    input,
  );
  return invokeCapability<TestResult>("k8s.testClusterConnection", {
    yaml,
    context: input.name.trim(),
  });
}

/**
 * The `current-context` a kubeconfig names, or null if it names none.
 *
 * Parsed with the YAML library rather than matched.
 *
 * A pattern here was a denial of service: `\s*` around a lazy body with a
 * trailing `\s*$` gave the engine several ways to split one run of spaces, and
 * a 4KB line took 52 seconds (js/polynomial-redos, #43). Hand-scanning instead
 * fixed that and then got the YAML wrong three times over — `#` inside a name,
 * `#` without preceding whitespace, `"prod\"live"`, `'prod''live'` — each a
 * name that no cluster matches, sent to testClusterConnection as if it were
 * real.
 *
 * The library already ships with this package for manifest editing, knows all
 * of those rules, and parses a 16KB hostile line in under 4ms. Writing a
 * fourth version of a YAML scalar reader was the wrong instinct.
 */
export function parseCurrentContext(yaml: string): string | null {
  let document: unknown;
  try {
    document = parseYaml(yaml);
  } catch {
    // Malformed YAML has no context to read, and the backend would reject the
    // same document anyway.
    return null;
  }
  if (typeof document !== "object" || document === null) return null;
  const context = (document as Record<string, unknown>)["current-context"];
  if (typeof context !== "string") return null;
  return context.trim() || null;
}

/** Test-connect a pasted/uploaded kubeconfig by its `current-context`. */
export async function testKubeconfigYaml(yaml: string): Promise<TestResult> {
  const context = parseCurrentContext(yaml);
  if (!context) throw new Error("kubeconfig has no current-context to test");
  return invokeCapability<TestResult>("k8s.testClusterConnection", { yaml, context });
}
