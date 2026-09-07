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

  // ---- The consent prompt this design now has (#374 item 1) --------------

  /**
   * For two rounds this sentence had to say the opposite. `McpConfirmDialog`
   * (`apps/desktop/src/App.tsx`) was the only listener for
   * `mcp://confirm-request`, `main.tsx` mounts that tree or this one and never
   * both, so a destructive call in the new design blocked and was DENIED on
   * timeout with nothing on screen — and a reader waiting to approve it would
   * have waited forever. `shell/AgentConsent.tsx` is the port, so the true
   * claim is back, and the assertions are inverted with it rather than deleted:
   * the pane must not drift back to hedging about a prompt that now exists.
   */
  it("promises the approval prompt, because there is one to give it in", () => {
    render(<AgentAccess />);
    const consent = screen.getByTestId("agent-consent").textContent ?? "";
    expect(consent).toMatch(/stops at a confirmation prompt/i);
    // And no leftover hedge from the round where there was none.
    expect(consent).not.toMatch(/no prompt/i);
    expect(consent).not.toMatch(/refused/i);
  });

  it("still says the approval is required, which was true throughout", () => {
    render(<AgentAccess />);
    const consent = screen.getByTestId("agent-consent").textContent ?? "";
    expect(consent).toMatch(/without approval/i);
    expect(consent).toMatch(/reads cluster state without asking/i);
  });

  /**
   * The copy assertion above is only as good as the fact behind it, and a
   * comment cannot fail. This scans the package for a consumer of the consent
   * event — the shape `Settings.test.tsx` uses to hold the `Deep links`
   * exclusion, which has now caught three rounds of drift on this branch.
   *
   * **The direction is inverted, not the test removed.** It used to assert
   * `[]`, so that whoever wired a listener would fail it and have to put the
   * promise of a prompt back deliberately. That happened. What it holds now is
   * the other half of the same seam: exactly ONE file in this package wires the
   * listener, and it is named — so deleting or moving that surface fails this
   * test, and whoever does it has to decide what the pane is allowed to promise
   * rather than leaving a sentence that quietly stopped being true.
   *
   * One and not "at least one", deliberately. Two listeners would both answer
   * one request, and only the winner's answer reaches the agent; the backend
   * broadcasts `mcp://confirm-resolved` precisely so a second surface can drop
   * a request it did not answer, and adding one is a decision, not a detail.
   */
  it("has exactly one consent-prompt consumer, which is why the sentence promises a prompt", () => {
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
        // Comments stripped: this very wiring is discussed in prose in
        // `AgentAccess.tsx` and in `shell/AgentConsent.tsx`'s own doc, and a
        // scan that read prose would count both of those as consumers — one of
        // them a settings pane that listens to nothing.
        //
        // `(?<!:)` on the line-comment half, unlike the sibling scan in
        // `Settings.test.tsx`. The mutation pass caught the naive form letting
        // a real consumer through: the tokens here contain `//`, so
        // `"mcp://confirm-request"` was read as a line comment from its own
        // scheme separator onwards and stripped down to `"mcp:` — the event
        // names could never match. Deep links needed no such care; none of its
        // tokens carry a slash.
        //
        // Where it still matters, stated from what the mutation pass actually
        // showed rather than from what would be tidy: replacing it with the
        // naive form does NOT fail this test today, because the consumer's
        // import line carries `respondToConfirm` and `ConfirmRequest` with no
        // slash in front of them and matches either way. What the lookbehind
        // buys is the case the four tokens exist for — HALF a wiring, a file
        // that subscribes to the event and answers nothing. Its only token is
        // the event name, the naive form eats it from its own scheme separator
        // onwards, and the scan would report a package with a dangling listener
        // as having none.
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
    expect(consumers).toEqual([join("shell", "AgentConsent.tsx")]);
  });
});
