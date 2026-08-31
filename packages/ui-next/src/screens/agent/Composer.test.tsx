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

const { useAgentRun, askAgent, stopAgentRun, chooseAgent, setSkillActive } = vi.hoisted(() => ({
  useAgentRun: vi.fn(),
  askAgent: vi.fn(),
  stopAgentRun: vi.fn(),
  chooseAgent: vi.fn(),
  setSkillActive: vi.fn(),
}));
vi.mock("../../lib/agentRun", () => ({ useAgentRun, askAgent, stopAgentRun, chooseAgent, setSkillActive }));

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
function runState(
  overrides: {
    busy?: boolean;
    agentKind?: string;
    turns?: { id: number }[];
    activeSkills?: string[];
  } = {},
) {
  return {
    turns: [],
    gates: [],
    busy: false,
    generation: 0,
    agentKind: "claude",
    activeSkills: [],
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

  // I6: `.catch(() => setAgents([]))` rendered the identical "No agent is
  // available … install one to get started" a genuinely empty install
  // shows — exactly the absence Ruling N introduced three-state to stop
  // asserting.
  it("says the agent CLI read failed, rather than claiming none are installed, when listAgents rejects", async () => {
    listAgents.mockRejectedValue(new Error("no such command: agent_list"));
    render(<Composer context="" />);
    expect(await screen.findByText(/could not be checked/i)).toBeTruthy();
    expect(await screen.findByText(/agent_list/)).toBeTruthy();
    expect(screen.queryByText(/no agent is available/i)).toBeNull();
  });

  it("says the agent CLI read failed even while a turn is running, rather than the empty-install sentence", async () => {
    useAgentRun.mockReturnValue(runState({ busy: true }));
    listAgents.mockRejectedValue(new Error("no such command: agent_list"));
    render(<Composer context="" />);
    expect(await screen.findByText(/could not be checked/i)).toBeTruthy();
    expect(screen.queryByText(/no agent is available/i)).toBeNull();
    // Stop must still survive whatever the agent read is doing (P3).
    expect(screen.getByRole("button", { name: /stop/i })).toBeTruthy();
  });

  it("says the slash menu's read failed, rather than opening onto no matches, when listPrompts rejects", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    listPrompts.mockRejectedValue(new Error("no such command: prompt_list"));
    render(<Composer context="prod" />);
    await userEvent.type(await screen.findByRole("textbox"), "/");
    expect(await screen.findByText(/could not be loaded/i)).toBeTruthy();
    expect(await screen.findByText(/prompt_list/)).toBeTruthy();
    expect(screen.queryByText(/no matches/i)).toBeNull();
  });

  it("says the slash menu's read failed, rather than opening onto no matches, when listSkills rejects", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    listSkills.mockRejectedValue(new Error("no such command: skill_list"));
    render(<Composer context="prod" />);
    await userEvent.type(await screen.findByRole("textbox"), "/");
    expect(await screen.findByText(/could not be loaded/i)).toBeTruthy();
    expect(await screen.findByText(/skill_list/)).toBeTruthy();
    expect(screen.queryByText(/no matches/i)).toBeNull();
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

  it("picking a skill from the menu writes to the shared store, not local state (C1/Ruling S)", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    render(<Composer context="" />);
    await userEvent.type(await screen.findByRole("textbox"), "/");
    await userEvent.click(
      within(screen.getByText("Skills").parentElement as HTMLElement).getByText("Rollout forensics"),
    );
    // The write goes through `setSkillActive` — the SAME function
    // `RunsRail`'s switch calls — never a copy of the set kept here. Whether
    // the store then dedupes a re-pick is `setSkillActive`'s own contract,
    // pinned in `agentRun.test.ts`, not this component's to re-prove.
    expect(setSkillActive).toHaveBeenCalledWith("Rollout forensics", true);
  });

  it("renders a chip for each skill active in the store, and Remove deactivates it there", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    useAgentRun.mockReturnValue(runState({ activeSkills: ["Rollout forensics"] }));
    render(<Composer context="" />);
    expect(await screen.findByText("Rollout forensics")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: /remove skill rollout forensics/i }));
    expect(setSkillActive).toHaveBeenCalledWith("Rollout forensics", false);
  });

  it("sends only the question — the active skills askAgent applies are the store's own default, not an opt this composer passes", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    useAgentRun.mockReturnValue(runState({ activeSkills: ["Rollout forensics"] }));
    render(<Composer context="" />);
    const box = await screen.findByRole("textbox");
    await userEvent.type(box, "check this{Enter}");
    expect(askAgent).toHaveBeenCalledWith("check this");
  });

  it("submits on Enter when the slash menu is closed, and clears the input", async () => {
    listAgents.mockResolvedValue([CLAUDE]);
    render(<Composer context="" />);
    const box = await screen.findByRole("textbox");
    await userEvent.type(box, "hello{Enter}");
    expect(askAgent).toHaveBeenCalledWith("hello");
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
    expect(askAgent).toHaveBeenCalledWith("hi there");
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
