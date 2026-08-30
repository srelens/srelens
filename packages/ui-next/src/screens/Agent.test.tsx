import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { AgentInfo, ClusterContext } from "@srelens/core";
import { Agent } from "./Agent";
import { resetContexts, setContexts } from "../lib/clusters";
import { defaultState } from "../lib/tabs";
import * as tabs from "../lib/tabsStore";

/**
 * Only the backend boundary is doubled — `listAgents`/`listPrompts`/
 * `listSkills`/`getPrompt` for the composer it mounts, `listSessions` for the
 * rail beside it. The active-cluster store stays real, exactly like
 * `Terminals.test.tsx`, because the one thing this suite exists to pin is
 * that the screen hands `Composer` the REAL active context rather than a
 * hardcoded blank.
 */
const { listAgents, listPrompts, listSkills, listSessions, getPrompt } = vi.hoisted(() => ({
  listAgents: vi.fn(),
  listPrompts: vi.fn(),
  listSkills: vi.fn(),
  listSessions: vi.fn(),
  getPrompt: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  listAgents,
  listPrompts,
  listSkills,
  listSessions,
  getPrompt,
}));

const { useAgentRun, askAgent, stopAgentRun, chooseAgent, clearAgentRun, setSkillActive } = vi.hoisted(() => ({
  useAgentRun: vi.fn(),
  askAgent: vi.fn(),
  stopAgentRun: vi.fn(),
  chooseAgent: vi.fn(),
  clearAgentRun: vi.fn(),
  setSkillActive: vi.fn(),
}));
vi.mock("../lib/agentRun", () => ({
  useAgentRun,
  askAgent,
  stopAgentRun,
  chooseAgent,
  clearAgentRun,
  setSkillActive,
}));

const CLAUDE: AgentInfo = {
  kind: "claude",
  label: "Claude",
  available: true,
  path: "/c",
  version: "1",
  installUrl: "",
  gated: false,
};

const CTX: ClusterContext = {
  name: "prod-eu",
  stableId: "prod",
  cluster: "prod",
  server: "https://prod",
  isCurrent: true,
  sourceFile: "/home/dana/.kube/config",
  authKind: "client certificate",
};

function runState(
  overrides: { turns?: unknown[]; gates?: unknown[]; busy?: boolean; activeSkills?: string[] } = {},
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
  listAgents.mockResolvedValue([CLAUDE]);
  listPrompts.mockResolvedValue([]);
  listSkills.mockResolvedValue([]);
  listSessions.mockResolvedValue([]);
  useAgentRun.mockReturnValue(runState());
  resetContexts();
  tabs.setState(defaultState([]));
});

describe("the agent screen", () => {
  it("draws the run's transcript over the one shared store the dock also reads", async () => {
    useAgentRun.mockReturnValue(
      runState({ turns: [{ id: 1, role: "user", text: "Diagnose checkout-api 5xx", calls: [], at: 1 }] }),
    );
    render(<Agent route="/agent" />);
    expect(await screen.findByText("Diagnose checkout-api 5xx")).toBeTruthy();
  });

  it("mounts the composer full — no compact copy, and a real send control once agents load", async () => {
    render(<Agent route="/agent" />);
    expect(await screen.findByRole("textbox", { name: /ask the agent/i })).toBeTruthy();
  });

  it("draws the rail's three sections beside the transcript", async () => {
    render(<Agent route="/agent" />);
    expect(await screen.findByText("Recent runs")).toBeTruthy();
    expect(screen.getByText("Skills")).toBeTruthy();
    expect(screen.getByText("MCP clients")).toBeTruthy();
  });

  it("starts a fresh run from the header's New question control", async () => {
    render(<Agent route="/agent" />);
    await userEvent.click(await screen.findByRole("button", { name: /new question/i }));
    expect(clearAgentRun).toHaveBeenCalledTimes(1);
  });

  it("hands the composer the real active cluster's name, never a hardcoded blank", async () => {
    setContexts([CTX]);
    tabs.setState(defaultState([CTX]));
    listPrompts.mockResolvedValue([
      { name: "diagnose", description: "Diagnose a workload", arguments: [{ name: "context", required: true, description: null }] },
    ]);
    getPrompt.mockResolvedValue("Diagnose prod-eu");
    render(<Agent route="/agent" />);
    const box = await screen.findByRole("textbox", { name: /ask the agent/i });
    await userEvent.type(box, "/");
    await userEvent.click(await screen.findByText("diagnose"));
    expect(getPrompt).toHaveBeenCalledWith("diagnose", { context: "prod-eu" });
  });

  it("heads the transcript with when the run started, off the first turn's own timestamp", async () => {
    const at = new Date(2026, 0, 1, 14, 4).getTime();
    useAgentRun.mockReturnValue(
      runState({ turns: [{ id: 1, role: "user", text: "checkout-api is throwing 5xx", calls: [], at }] }),
    );
    render(<Agent route="/agent" />);
    expect(await screen.findByText("started 14:04")).toBeTruthy();
  });

  it("draws no started time for a run with no turns yet", async () => {
    render(<Agent route="/agent" />);
    await screen.findByRole("textbox", { name: /ask the agent/i });
    expect(screen.queryByText(/^started /)).toBeNull();
  });

  it("tells the reader this screen and the console dock share one run", async () => {
    render(<Agent route="/agent" />);
    expect(
      await screen.findByText("Continue this run from the console at the bottom of the window"),
    ).toBeTruthy();
  });
});
