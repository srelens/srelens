import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { kubectlPodStatus } from "./kubectlPodStatus";
import type { K8sObject } from "./manifest";

// The same cases `crates/kube/tests/pod_status.rs` runs through the backend's
// `kubectl_status`: one file, two ports, one answer per pod.
// Not `new URL(template, import.meta.url)`: Vite rewrites that form as an asset import.
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../../..");
const cases: { name: string; pod: K8sObject; status: string }[] = JSON.parse(
  readFileSync(resolve(repoRoot, "crates/kube/tests/fixtures/pod_status_cases.json"), "utf8"),
);

describe("kubectlPodStatus — the cases the backend's kubectl_status runs", () => {
  it("has every case the backend runs", () => {
    expect(cases.length).toBeGreaterThanOrEqual(24);
  });

  it.each(cases.map((c) => [c.name, c] as const))("%s", (_name, c) => {
    expect(kubectlPodStatus(c.pod)).toBe(c.status);
  });
});
