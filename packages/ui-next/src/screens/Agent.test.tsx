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
// The screen is desktop-only past the web branch: `askAgent` starts with
// `chat_start`, and the web command dispatcher has no `chat_*` arm. jsdom is
// not Tauri, so `isTauri` is forced true here or every test below would be
// asserting about the web explanation. That branch has its own test.
const { listAgents, listPrompts, listSkills, listSessions, getPrompt, isTauri } = vi.hoisted(() => ({
  isTauri: vi.fn(() => true),
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
  isTauri,
}));

const {
  useAgentRun,
  askAgent,
  stopAgentRun,
  chooseAgent,
  clearAgentRun,
  dismissAgentError,
  setSkillActive,
  useRunSummaries,
  getActiveRunKey,
  selectRun,
  restoreRuns,
  openSavedRun,
} = vi.hoisted(() => ({
  useAgentRun: vi.fn(),
  askAgent: vi.fn(),
  stopAgentRun: vi.fn(),
  chooseAgent: vi.fn(),
  clearAgentRun: vi.fn(),
  dismissAgentError: vi.fn(),
  setSkillActive: vi.fn(),
  useRunSummaries: vi.fn(() => []),
  getActiveRunKey: vi.fn(() => null),
  selectRun: vi.fn(),
  restoreRuns: vi.fn(async () => {}),
  openSavedRun: vi.fn(async () => {}),
}));
vi.mock("../lib/agentRun", () => ({
  useAgentRun,
  askAgent,
  stopAgentRun,
  chooseAgent,
  clearAgentRun,
  dismissAgentError,
  setSkillActive,
  useRunSummaries,
  getActiveRunKey,
  selectRun,
  restoreRuns,
  openSavedRun,
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
  overrides: {
    turns?: unknown[];
    gates?: unknown[];
    busy?: boolean;
    activeSkills?: string[];
    error?: string;
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
  isTauri.mockReturnValue(true);
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

  /**
   * This screen has no prompt of its own. The dock is the one prompt in the
   * app — same component, every screen — which is what "just use same console
   * dock, don't rebuild anything" settled. The bespoke `Composer` that used to
   * mount here is deleted.
   */
  it("mounts no prompt of its own — the dock is the bar here too", () => {
    render(<Agent route="/agent" />);
    expect(screen.queryByRole("textbox")).toBeNull();
    // And no second Send: the dock's is the only one.
    expect(screen.queryByRole("button", { name: /^send$/i })).toBeNull();
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

  it("draws the agent picker in the rail, where the screen's own controls live", async () => {
    setContexts([CTX]);
    tabs.setState(defaultState([CTX]));
    render(<Agent route="/agent" />);
    // The picker moved here when the screen lost its bar: the dock is the
    // prompt, and this rail already held Skills.
    // "Agent" appears twice — the rail's own head and this section — so the
    // picker itself is what to assert on.
    expect(await screen.findByRole("button", { name: /claude/i })).toBeTruthy();
  });

  it("heads the transcript with when the run started, off the FIRST turn's own timestamp", async () => {
    // A fixture of one turn cannot tell "the first turn's `at`" apart from
    // "the last turn's `at`" — they're the same value. Two turns, with the
    // later one second, is what makes `turns[0].at` → `turns.at(-1).at` an
    // actual mutation rather than a no-op.
    const at = new Date(2026, 0, 1, 14, 4).getTime();
    const later = new Date(2026, 0, 1, 15, 30).getTime();
    useAgentRun.mockReturnValue(
      runState({
        turns: [
          { id: 1, role: "user", text: "checkout-api is throwing 5xx", calls: [], at },
          { id: 2, role: "agent", text: "Looking into it", calls: [], at: later },
        ],
      }),
    );
    render(<Agent route="/agent" />);
    expect(await screen.findByText("started 14:04")).toBeTruthy();
    expect(screen.queryByText(/15:30/)).toBeNull();
  });

  it("draws no started time for a run with no turns yet", async () => {
    render(<Agent route="/agent" />);
    // The rail is the settle signal now: the composer this used to wait on is
    // gone, since the dock is the prompt on every screen.
    await screen.findByText("Recent runs");
    expect(screen.queryByText(/^started /)).toBeNull();
  });

  // I5: #386's exclusion is scoped to `RunsRail`'s `Recent runs` list
  // (`SessionMeta` genuinely has no counts) — this pane describes the LIVE
  // run, and the store counts every tool call an agent has made in it
  // (`Turn.calls[]`). Dropping the count alongside duration was applying
  // #386 to a figure it does not cover.
  it("heads the pane with the calls the store has actually observed, across every turn", async () => {
    useAgentRun.mockReturnValue(
      runState({
        turns: [
          {
            id: 1,
            role: "agent",
            text: "a",
            at: 1,
            calls: [
              { id: "c1", tool: "k8s.listPods", args: {}, status: "ok" },
              { id: "c2", tool: "k8s.scale", args: {}, status: "ok" },
            ],
          },
          {
            id: 2,
            role: "agent",
            text: "b",
            at: 2,
            calls: [{ id: "c3", tool: "k8s.getPod", args: {}, status: "ok" }],
          },
        ],
      }),
    );
    render(<Agent route="/agent" />);
    expect(await screen.findByText(/3 calls/)).toBeTruthy();
  });

  it("shows no call count for a run with turns but no tool calls yet — an absent reading renders no reading", async () => {
    useAgentRun.mockReturnValue(
      runState({ turns: [{ id: 1, role: "user", text: "checkout-api is throwing 5xx", calls: [], at: 1 }] }),
    );
    render(<Agent route="/agent" />);
    await screen.findByText(/^started /);
    expect(screen.queryByText(/call/i)).toBeNull();
  });

  it("tells the reader this screen and the console dock share one run", async () => {
    render(<Agent route="/agent" />);
    // §5's sentence named "the console at the bottom of the window" — but the
    // dock does not render on this screen any more (it put a second input box
    // under this screen's own, reported as a duplicate text box). The fact §5
    // wanted the reader to have is that it is ONE conversation; the sentence
    // says that without naming a control that is not there.
    // §5's own sentence, and true again: the dock IS at the bottom of the
    // window on this screen, because it is the prompt here too.
    expect(
      await screen.findByText("Continue this run from the console at the bottom of the window"),
    ).toBeTruthy();
  });

  /**
   * P2 (#392 review round 4). The store carried a run-level failure in
   * `error` — a submission refused because a turn was already in flight, a
   * `cancelChat` that did not land — and NOTHING rendered it. So an `ask()`
   * chip pressed mid-turn was refused in silence, which is the exact
   * behaviour the refusal was added to prevent.
   */
  it("shows a run-level failure rather than leaving the store holding it", () => {
    useAgentRun.mockReturnValue(
      runState({ error: "srelens is still answering the last question." }),
    );
    render(<Agent route="/agent" />);
    expect(screen.getByText(/still answering the last question/i)).toBeTruthy();
  });

  it("lets the reader put that failure away", async () => {
    useAgentRun.mockReturnValue(runState({ error: "srelens is still answering." }));
    render(<Agent route="/agent" />);
    await userEvent.click(screen.getByRole("button", { name: /dismiss/i }));
    expect(dismissAgentError).toHaveBeenCalledTimes(1);
  });

  /**
   * P1 (#392 review round 7). In the browser every question 404s —
   * `api_command.rs` has no `chat_*` or `agent_list` arm — so this screen is
   * where a reader is TOLD, rather than left to discover it from a failed
   * send. The dock hides itself for the same reason; this is the surface that
   * carries the explanation.
   */
  it("says the agent is a desktop feature instead of drawing a composer that cannot work", () => {
    isTauri.mockReturnValue(false);
    render(<Agent route="/agent" />);
    expect(screen.getByTestId("agent-desktop-only").textContent).toMatch(/runs in the srelens desktop app/i);
    // No composer, because there is nothing for it to reach.
    expect(screen.queryByRole("textbox")).toBeNull();
    // And not dressed as a failure: nothing has failed, nothing was asked.
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("still says what DOES work in the browser, rather than only what does not", () => {
    isTauri.mockReturnValue(false);
    render(<Agent route="/agent" />);
    expect(screen.getByText(/srelens server/)).toBeTruthy();
  });
});