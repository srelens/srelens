import { describe, it, expect, vi, beforeEach } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
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

  // ---- The consent prompt this design does not have (#374) -------------

  /**
   * The sentence read "every change it proposes stops at a confirmation
   * prompt" — true of classic, false here. `McpConfirmDialog`
   * (`apps/desktop/src/App.tsx`) is the only listener for
   * `mcp://confirm-request`, and `main.tsx` mounts that tree or this one and
   * never both, so under the new design a destructive call blocks and is
   * DENIED on timeout with nothing on screen. A reader waiting to approve it
   * would have waited forever.
   */
  it("promises no approval prompt, because there is none to give it in", () => {
    render(<AgentAccess />);
    const consent = screen.getByTestId("agent-consent").textContent ?? "";
    expect(consent).not.toMatch(/stops at a confirmation prompt/i);
    // Not "you will be asked", not "prompt" as something the reader will see.
    expect(consent).toMatch(/no prompt/i);
    // And what actually happens instead is stated, not left out.
    expect(consent).toMatch(/refused/i);
  });

  it("still says the approval is required, which is the half that is true", () => {
    render(<AgentAccess />);
    const consent = screen.getByTestId("agent-consent").textContent ?? "";
    expect(consent).toMatch(/without approval/i);
    expect(consent).toMatch(/reads cluster state without asking/i);
  });

  /**
   * The copy assertions above are only as good as the fact behind them, and a
   * comment cannot fail. This scans the package for a consumer of the consent
   * event — the shape `Settings.test.tsx` uses to hold the `Deep links`
   * exclusion, which has now caught two rounds of drift on this branch.
   *
   * Whoever wires the listener in step 9 fails THIS test, and has to put the
   * promise of a prompt back deliberately — in the same commit, with the copy
   * corrected — rather than leaving a pane that quietly stopped being true.
   */
  it("has no consent-prompt consumer of its own, which is why the sentence says none", () => {
    const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
    const consumers: string[] = [];
    const walk = (dir: string) => {
      for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const path = join(dir, entry.name);
        if (entry.isDirectory()) {
          walk(path);
          continue;
        }
        if (!/\.tsx?$/.test(entry.name) || /\.test\.tsx?$/.test(entry.name)) continue;
        // Comments stripped: this very absence is discussed in prose in
        // `AgentAccess.tsx`, and a scan that read prose would find itself.
        //
        // `(?<!:)` on the line-comment half, unlike the sibling scan in
        // `Settings.test.tsx`. The mutation pass caught the naive form letting
        // a real consumer through: the tokens here contain `//`, so
        // `"mcp://confirm-request"` was read as a line comment from its own
        // scheme separator onwards and stripped down to `"mcp:` — the event
        // names could never match. Deep links needed no such care; none of its
        // tokens carry a slash.
        const source = readFileSync(path, "utf8").replace(/\/\*[\s\S]*?\*\/|(?<!:)\/\/.*/g, "");
        // Both halves of a wiring, so half of one is caught too: the event a
        // listener subscribes to, and the call and type any answer needs.
        if (
          /mcp:\/\/confirm-request|mcp:\/\/confirm-resolved|respondToConfirm|ConfirmRequest/.test(
            source,
          )
        ) {
          consumers.push(relative(root, path));
        }
      }
    };
    walk(root);
    expect(consumers).toEqual([]);
  });
});
