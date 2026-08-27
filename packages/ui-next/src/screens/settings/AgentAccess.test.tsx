import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";

/**
 * Only `isTauri`. The gated list must come from the real catalog — replacing
 * `gatedCapabilityIds` would put this file back where it was, comparing a
 * hardcoded array to a hardcoded array.
 */
const core = vi.hoisted(() => ({ isTauri: vi.fn(() => true) }));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

import { gatedCapabilityIds } from "@srelens/core";
import { AgentAccess } from "./AgentAccess";

describe("AgentAccess", () => {
  beforeEach(() => {
    core.isTauri.mockReturnValue(true);
  });

  it("says reading is always on, and does not offer to change it", () => {
    render(<AgentAccess />);
    const read = screen.getByRole("switch", { name: /read cluster state/i }) as HTMLButtonElement;
    expect(read.disabled).toBe(true);
    expect(read.getAttribute("aria-checked")).toBe("true");
  });

  it("draws no switch for a setting that does not exist", () => {
    render(<AgentAccess />);
    expect(screen.queryByRole("switch", { name: /propose changes/i })).toBeNull();
    expect(screen.queryByRole("switch", { name: /read secrets/i })).toBeNull();
    expect(screen.getAllByRole("switch")).toHaveLength(1);
  });

  /**
   * Against the registry, not against a second hardcoded list.
   *
   * What this replaced compared one hardcoded array to another: it could not
   * fail for the reason its name gave, and it did not — the six ids it pinned
   * (`node.drain`, `pod.evict`, `resource.delete`, `workload.scale`,
   * `rollout.undo`, `helm.uninstall`) are not capabilities srelens registers,
   * and the audit table one panel below rendered the real ones six inches
   * away.
   */
  it("names every confirm-gated capability the backend registers, and no other", () => {
    render(<AgentAccess />);
    const chips = screen.getAllByTestId("gated-capability").map((c) => c.textContent);
    expect(chips).toEqual(gatedCapabilityIds("desktop"));
  });

  it("counts the chips it drew, so the sentence cannot claim a different set", () => {
    render(<AgentAccess />);
    const chips = screen.getAllByTestId("gated-capability");
    expect(chips.length).toBeGreaterThan(6);
    expect(screen.getByTestId("gated-count").textContent).toContain(String(chips.length));
  });

  /**
   * The heading claims completeness ("Never without confirmation"), so a
   * reader must be able to look up any gated id and find it. Three the
   * invented six missed, each gated a different way: `k8s.applyManifest`
   * changes without being destructive, `k8s.getSecret` is a gated READ, and
   * `toolbox.installKubectl` changes the host rather than a cluster.
   */
  it("leaves out no gated capability a reader might look for", () => {
    render(<AgentAccess />);
    const chips = screen.getAllByTestId("gated-capability").map((c) => c.textContent);
    for (const id of ["k8s.applyManifest", "k8s.getSecret", "toolbox.installKubectl"]) {
      expect(chips).toContain(id);
    }
  });

  /**
   * A web build does not register the host settings capabilities
   * (`web_registry_omits_host_desktop_settings`), so naming `settings.set`
   * there would be this pane inventing an id again, one instead of six.
   */
  it("does not name a capability the running build has no registry entry for", () => {
    core.isTauri.mockReturnValue(false);
    render(<AgentAccess />);
    const chips = screen.getAllByTestId("gated-capability").map((c) => c.textContent);
    expect(chips).toEqual(gatedCapabilityIds("web"));
    expect(chips).not.toContain("settings.set");
  });

  it("tells the reader the agent can read without asking", () => {
    render(<AgentAccess />);
    expect(screen.getByText(/read .*without asking|reads .*without asking/i)).toBeTruthy();
  });
});
