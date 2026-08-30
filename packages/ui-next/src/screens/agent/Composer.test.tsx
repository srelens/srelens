import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describeError, type AgentInfo } from "@srelens/core";
import { Composer } from "./Composer";

const { listAgents, listPrompts, listSkills, getPrompt } = vi.hoisted(() => ({
  listAgents: vi.fn(),
  listPrompts: vi.fn(),
  listSkills: vi.fn(),
  getPrompt: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  listAgents,
  listPrompts,
  listSkills,
  getPrompt,
}));

const { useAgentRun, askAgent, stopAgentRun, chooseAgent } = vi.hoisted(() => ({
  useAgentRun: vi.fn(),
  askAgent: vi.fn(),
  stopAgentRun: vi.fn(),
  chooseAgent: vi.fn(),
}));
vi.mock("../../lib/agentRun", () => ({ useAgentRun, askAgent, stopAgentRun, chooseAgent }));

const CLAUDE: AgentInfo = {
  kind: "claude",
  label: "Claude",
  available: true,
  path: "/c",
  version: "1",
  installUrl: "",
  gated: false,
};
const CODEX_GATED: AgentInfo = {
  kind: "codex",
  label: "Codex",
  available: true,
  path: "/x",
  version: "1",
  installUrl: "",
  gated: true,
};

/** The store's shape, defaulted to idle-and-empty — every test overrides only
 *  the fields it cares about. */
function runState(overrides: { busy?: boolean; agentKind?: string; turns?: { id: number }[] } = {}) {
  return {
    turns: [],
    gates: [],
    busy: false,
    generation: 0,
    agentKind: "claude",
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  // A required "context" argument, exactly like every real builtin prompt
  // (`assistant_prompts.rs`) — the fixture that let C1 through the door.
  listPrompts.mockResolvedValue([
    { name: "diagnose", description: "Diagnose a workload", arguments: [{ name: "context", required: true, description: null }] },
  ]);
  listSkills.mockResolvedValue([{ name: "Rollout forensics", description: "Correlates a revision diff" }]);
  useAgentRun.mockReturnValue(runState());
  askAgent.mockResolvedValue(undefined);
});

describe("the composer", () => {
  it("says which agents srelens can drive once the read has landed, and offers no send", async () => {
    listAgents.mockResolvedValue([]);
    render(<Composer context="" />);
    expect(await screen.findByText(/no agent/i)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /send/i })).toBeNull();
  });

  it("says nothing about 'no agent' until the read has landed (Ruling N)", async () => {
    let resolveAgents!: (v: AgentInfo[]) => void;
    listAgents.mockReturnValue(
      new Promise<AgentInfo[]>((resolve) => {
        resolveAgents = resolve;
      }),
    );
    render(<Composer context="" />);
    // The read is still pending: nothing has been asserted about an absence.
    expect(screen.queryByText(/no agent/i)).toBeNull();
    await act(async () => {
      resolveAgents([]);
      await Promise.resolve();
    });
    expect(await screen.findByText(/no agent/i)).toBeTruthy();
  });

  it("offers prompts and skills under the slash menu, each in its own group", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    render(<Composer context="prod" />);
    await userEvent.type(await screen.findByRole("textbox"), "/");
    await waitFor(() => expect(screen.getByText("diagnose")).toBeTruthy());
    expect(screen.getByText("Rollout forensics")).toBeTruthy();
  });

  it("puts each item under its own labelled heading, not merely somewhere in the menu", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    render(<Composer context="prod" />);
    await userEvent.type(await screen.findByRole("textbox"), "/");
    await waitFor(() => expect(screen.getByText("diagnose")).toBeTruthy());
    const promptsGroup = screen.getByText("Prompts").parentElement as HTMLElement;
    const skillsGroup = screen.getByText("Skills").parentElement as HTMLElement;
    expect(within(promptsGroup).getByText("diagnose")).toBeTruthy();
    expect(within(promptsGroup).queryByText("Rollout forensics")).toBeNull();
    expect(within(skillsGroup).getByText("Rollout forensics")).toBeTruthy();
    expect(within(skillsGroup).queryByText("diagnose")).toBeNull();
  });

  it("does not offer an agent that is installed but gated", async () => {
    listAgents.mockResolvedValue([CLAUDE, CODEX_GATED]);
    render(<Composer context="" />);
    await userEvent.click(await screen.findByRole("button", { name: /claude/i }));
    expect(screen.queryByRole("option", { name: /codex/i })).toBeNull();
  });

  it("renders a prompt needing only context into the input (C1)", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    getPrompt.mockResolvedValue("Diagnose prod");
    render(<Composer context="prod" />);
    const box = await screen.findByRole("textbox");
    await userEvent.type(box, "/");
    await userEvent.click(await screen.findByText("diagnose"));
    await waitFor(() => expect(getPrompt).toHaveBeenCalledWith("diagnose", { context: "prod" }));
    await waitFor(() => expect((box as HTMLInputElement).value).toBe("Diagnose prod"));
  });

  it("refuses a prompt needing more than context, rather than firing a call that will be refused", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    listPrompts.mockResolvedValue([
      {
        name: "restart",
        description: "Restart a workload",
        arguments: [
          { name: "context", required: true, description: null },
          { name: "workload", required: true, description: null },
        ],
      },
    ]);
    render(<Composer context="prod" />);
    await userEvent.type(await screen.findByRole("textbox"), "/");
    await userEvent.click(await screen.findByText("restart"));
    expect(getPrompt).not.toHaveBeenCalled();
    expect(await screen.findByText(/workload/i)).toBeTruthy();
  });

  it("refuses a context-only prompt when no cluster is in context, rather than firing a call the backend will refuse (G1)", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    // The default fixture prompt needs only "context" — an empty `context`
    // prop must still be treated as unfillable, not as a filled string.
    render(<Composer context="" />);
    await userEvent.type(await screen.findByRole("textbox"), "/");
    await userEvent.click(await screen.findByText("diagnose"));
    expect(getPrompt).not.toHaveBeenCalled();
    expect(await screen.findByText(/cluster/i)).toBeTruthy();
  });

  it("surfaces a rejected getPrompt through describeError, never the raw backend string", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    getPrompt.mockRejectedValue(new Error("boom"));
    render(<Composer context="prod" />);
    await userEvent.type(await screen.findByRole("textbox"), "/");
    await userEvent.click(await screen.findByText("diagnose"));
    const detail = describeError(new Error("boom")).detail;
    expect(await screen.findByText(detail)).toBeTruthy();
  });

  it("activates a picked skill as a chip, deduped on re-pick", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    render(<Composer context="" />);
    const box = await screen.findByRole("textbox");
    await userEvent.type(box, "/");
    // Scoped to the menu's own "Skills" group throughout: once the chip
    // exists, "Rollout forensics" appears twice on screen (chip and reopened
    // menu item), and an unscoped query can't tell them apart.
    await userEvent.click(within(screen.getByText("Skills").parentElement as HTMLElement).getByText("Rollout forensics"));
    expect(screen.getAllByText("Rollout forensics")).toHaveLength(1);
    await userEvent.clear(box);
    await userEvent.type(box, "/");
    await userEvent.click(within(screen.getByText("Skills").parentElement as HTMLElement).getByText("Rollout forensics"));
    expect(screen.getAllByText("Rollout forensics")).toHaveLength(1);
  });

  it("removes an active skill's chip", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    render(<Composer context="" />);
    await userEvent.type(await screen.findByRole("textbox"), "/");
    await userEvent.click(await screen.findByText("Rollout forensics"));
    await userEvent.click(screen.getByRole("button", { name: /remove skill rollout forensics/i }));
    expect(screen.queryByText("Rollout forensics")).toBeNull();
  });

  it("passes active skills to askAgent", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    render(<Composer context="" />);
    const box = await screen.findByRole("textbox");
    await userEvent.type(box, "/");
    await userEvent.click(await screen.findByText("Rollout forensics"));
    await userEvent.clear(box);
    await userEvent.type(box, "check this{Enter}");
    expect(askAgent).toHaveBeenCalledWith("check this", { skills: ["Rollout forensics"] });
  });

  it("drops an active skill when the conversation clears", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    useAgentRun.mockReturnValue(runState({ turns: [{ id: 1 }] }));
    const { rerender } = render(<Composer context="" />);
    await userEvent.type(await screen.findByRole("textbox"), "/");
    await userEvent.click(await screen.findByText("Rollout forensics"));
    expect(screen.getByText("Rollout forensics")).toBeTruthy();
    useAgentRun.mockReturnValue(runState({ turns: [] }));
    rerender(<Composer context="" />);
    expect(screen.queryByText("Rollout forensics")).toBeNull();
  });

  it("submits on Enter when the slash menu is closed, and clears the input", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    render(<Composer context="" />);
    const box = await screen.findByRole("textbox");
    await userEvent.type(box, "hello{Enter}");
    expect(askAgent).toHaveBeenCalledWith("hello", { skills: [] });
    expect((box as HTMLInputElement).value).toBe("");
  });

  it("swallows Enter while the slash menu is open, rather than submitting the raw token", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    render(<Composer context="" />);
    const box = await screen.findByRole("textbox");
    await userEvent.type(box, "/{Enter}");
    expect(askAgent).not.toHaveBeenCalled();
  });

  it("submits trimmed text via Send and clears the input", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    render(<Composer context="" />);
    const box = await screen.findByRole("textbox");
    await userEvent.type(box, "  hi there  ");
    await userEvent.click(screen.getByRole("button", { name: /send/i }));
    expect(askAgent).toHaveBeenCalledWith("hi there", { skills: [] });
    expect((box as HTMLInputElement).value).toBe("");
  });

  it("shows Stop instead of Send while busy, and Stop calls stopAgentRun", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    useAgentRun.mockReturnValue(runState({ busy: true }));
    render(<Composer context="" />);
    // Wait for the full composer to settle in (the agent picker only exists
    // there) before asserting about Stop — `agents` starts unresolved, and a
    // "Stop" also renders, transiently, in the loading/no-agent branch below.
    await screen.findByRole("button", { name: /claude/i });
    expect(screen.queryByRole("button", { name: /^send$/i })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: /stop/i }));
    expect(stopAgentRun).toHaveBeenCalledTimes(1);
  });

  it("keeps Stop available if the agent list empties out mid-turn (P3)", async () => {
    listAgents.mockResolvedValue([]);
    useAgentRun.mockReturnValue(runState({ busy: true }));
    render(<Composer context="" />);
    // Settle past the transient "loading" wording onto the landed read,
    // rather than relying on the "Stop" button happening to be the same DOM
    // node either side of that transition.
    await screen.findByText(/no agent/i);
    await userEvent.click(screen.getByRole("button", { name: /stop/i }));
    expect(stopAgentRun).toHaveBeenCalledTimes(1);
  });
});
