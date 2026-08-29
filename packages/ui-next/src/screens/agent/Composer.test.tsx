import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Composer } from "./Composer";

const { listAgents, listPrompts, listSkills } = vi.hoisted(() => ({
  listAgents: vi.fn(), listPrompts: vi.fn(), listSkills: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()), listAgents, listPrompts, listSkills,
}));

beforeEach(() => {
  listPrompts.mockResolvedValue([{ name: "diagnose", description: "Diagnose a workload", arguments: [] }]);
  listSkills.mockResolvedValue([{ name: "Rollout forensics", description: "Correlates a revision diff" }]);
});

describe("the composer", () => {
  it("says which agents srelens can drive when none is installed, and offers no send", async () => {
    listAgents.mockResolvedValue([]);
    render(<Composer />);
    expect(await screen.findByText(/no agent/i)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /send/i })).toBeNull();
  });

  it("offers prompts and skills under the slash menu, each in its own group", async () => {
    listAgents.mockResolvedValue([{ kind: "claude", label: "Claude", available: true, path: "/c", version: "1", installUrl: "", gated: false }]);
    render(<Composer />);
    await userEvent.type(await screen.findByRole("textbox"), "/");
    await waitFor(() => expect(screen.getByText("diagnose")).toBeTruthy());
    expect(screen.getByText("Rollout forensics")).toBeTruthy();
  });

  it("does not offer an agent that is installed but gated", async () => {
    listAgents.mockResolvedValue([
      { kind: "claude", label: "Claude", available: true, path: "/c", version: "1", installUrl: "", gated: false },
      { kind: "codex", label: "Codex", available: true, path: "/x", version: "1", installUrl: "", gated: true },
    ]);
    render(<Composer />);
    await userEvent.click(await screen.findByRole("button", { name: /claude/i }));
    expect(screen.queryByRole("option", { name: /codex/i })).toBeNull();
  });
});
