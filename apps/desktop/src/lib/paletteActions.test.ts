import { describe, it, expect } from "vitest";
import { PALETTE_ACTIONS, actionsForKind, paletteActionCapabilityIds } from "./paletteActions";

describe("paletteActions", () => {
  it("exposes actions with non-empty capability ids and labels", () => {
    expect(PALETTE_ACTIONS.length).toBeGreaterThan(0);
    for (const a of PALETTE_ACTIONS) {
      expect(a.capabilityId).toMatch(/^[a-z0-9]+\./);
      expect(a.label.length).toBeGreaterThan(0);
    }
  });
  it("filters actions applicable to a kind", () => {
    const forDeploy = actionsForKind("deployments").map((a) => a.capabilityId);
    expect(forDeploy).toContain("k8s.scale");
    expect(forDeploy).toContain("k8s.rolloutRestart");
    const forCm = actionsForKind("configmaps").map((a) => a.capabilityId);
    expect(forCm).not.toContain("k8s.scale");
  });
  it("reports its covered capability ids", () => {
    expect(paletteActionCapabilityIds().has("k8s.deleteResource")).toBe(true);
  });
});
