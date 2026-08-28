import { describe, expect, it } from "vitest";
import catalog from "./capability-catalog.json";
import {
  CAPABILITY_CATALOG,
  HOST_ONLY_CAPABILITY_IDS,
  gatedCapabilityIds,
} from "./capabilities";

/**
 * The generated catalog is the source of truth here — held equal to the live
 * Rust registry by `capability_catalog_json_is_in_sync`. So these tests compare
 * the derivation against the FILE rather than against a list retyped in a test
 * body: a hardcoded expectation would be a third copy, and the reason this
 * module exists at all is that the new design's `Agent access` pane shipped a
 * hardcoded array of six ids the backend does not register, pinned by a test
 * that compared one hardcoded array to another.
 */
type Entry = { id: string; readOnly: boolean; destructive: boolean; requiresConfirm: boolean };
const entries = catalog as Entry[];

describe("gatedCapabilityIds", () => {
  it("is every confirm-gated capability the backend registers, and nothing else", () => {
    const want = entries.filter((c) => c.requiresConfirm).map((c) => c.id);
    expect(gatedCapabilityIds("desktop")).toEqual(want);
    expect(want.length).toBeGreaterThan(0);
  });

  it("names ids that exist, in the form the audit trail records", () => {
    const known = new Set(entries.map((c) => c.id));
    for (const id of gatedCapabilityIds("desktop")) expect(known.has(id)).toBe(true);
    // The invented set this replaced was `node.drain`, `pod.evict`,
    // `resource.delete`, `workload.scale`, `rollout.undo`, `helm.uninstall`.
    // Not one is registered, and the audit pane one panel below renders the
    // real ids — so both appeared on the same screen.
    for (const invented of ["node.drain", "pod.evict", "resource.delete", "workload.scale", "rollout.undo", "helm.uninstall"]) {
      expect(known.has(invented)).toBe(false);
    }
  });

  /**
   * The gate is `requiresConfirm`, not `destructive`. A pane that filtered on
   * `destructive` would drop `k8s.scale`, `k8s.applyManifest` and every toolbox
   * install — all gated, none flagged destructive.
   */
  it("includes gated capabilities that are not destructive", () => {
    const ids = gatedCapabilityIds("desktop");
    const gatedNonDestructive = entries.filter((c) => c.requiresConfirm && !c.destructive);
    expect(gatedNonDestructive.length).toBeGreaterThan(0);
    for (const c of gatedNonDestructive) expect(ids).toContain(c.id);
  });

  /** `k8s.getSecret` is read-only and gated — the case no other flag implies. */
  it("includes the read that returns secret material", () => {
    const secret = entries.find((c) => c.id === "k8s.getSecret");
    expect(secret).toBeTruthy();
    expect(secret?.readOnly).toBe(true);
    expect(gatedCapabilityIds("desktop")).toContain("k8s.getSecret");
  });

  it("leaves out the capabilities a web registry does not register", () => {
    const web = gatedCapabilityIds("web");
    const desktop = gatedCapabilityIds("desktop");
    expect(web).not.toContain("settings.set");
    expect(desktop).toContain("settings.set");
    expect(desktop.filter((id) => !HOST_ONLY_CAPABILITY_IDS.includes(id))).toEqual(web);
  });

  it("exposes the catalog it derives from, unchanged", () => {
    expect(CAPABILITY_CATALOG).toEqual(entries);
  });
});
