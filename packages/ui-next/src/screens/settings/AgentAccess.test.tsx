import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";

import { AgentAccess } from "./AgentAccess";

describe("AgentAccess", () => {
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

  it("names every capability that always needs confirmation, in order", () => {
    render(<AgentAccess />);
    const chips = screen.getAllByTestId("gated-capability").map((c) => c.textContent);
    expect(chips).toEqual([
      "node.drain",
      "pod.evict",
      "resource.delete",
      "workload.scale",
      "rollout.undo",
      "helm.uninstall",
    ]);
  });

  it("tells the reader the agent can read without asking", () => {
    render(<AgentAccess />);
    expect(screen.getByText(/read .*without asking|reads .*without asking/i)).toBeTruthy();
  });
});
