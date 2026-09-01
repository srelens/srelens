import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { AgentInfo, ClusterContext } from "@srelens/core";
import { Agent } from "./Agent";
import { ConsoleProvider, useConsole } from "../console";
import { resetLock, __setKnownVaultMode } from "../shell/LockGate";
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
  useActiveRunKey,
  useRun,
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
  // The dock mounts inside this screen now, so the screen's suite meets the
  // dock's own store reads too.
  useActiveRunKey: vi.fn<() => string | null>(() => null),
  useRun: vi.fn(),
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
  useActiveRunKey,
  useRun,
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
  useRun.mockReturnValue(runState());
  // The dock hides itself over a covered workspace, and a fresh lock store has
  // read no vault mode — which counts as covered. Said out loud here, or every
  // test below asserts about a dock that correctly renders nothing. Same line,
  // same reason, as `Console.test.tsx` and `AgentConsent.test.tsx`.
  resetLock();
  __setKnownVaultMode("unlocked");
  resetContexts();
  tabs.setState(defaultState([]));
});

/** The screen mounts the dock inside its own column now, and the dock needs
 *  the provider. Wrapped rather than mocked: the defects this suite keeps
 *  finding are in the COMPOSITION of screen and dock, and a mocked dock cannot
 *  see them. */
/** Opens the console the way anything else in the app does — the dock starts
 *  collapsed, and a collapsed dock has no body to assert about. */
function Opener() {
  const { setOpen } = useConsole();
  return (
    <button type="button" onClick={() => setOpen(true)}>
      Open the console
    </button>
  );
}

function renderAgent() {
  return render(
    <ConsoleProvider>
      <Agent route="/agent" />
    </ConsoleProvider>,
  );
}

describe("the agent screen", () => {
  it("draws the run's transcript over the one shared store the dock also reads", async () => {
    useAgentRun.mockReturnValue(
      runState({ turns: [{ id: 1, role: "user", text: "Diagnose checkout-api 5xx", calls: [], at: 1 }] }),
    );
    renderAgent();
    // Scoped to the TRANSCRIPT: the head now names the conversation from the
    // same question, so an unscoped query matches both and proves neither.
    const log = await screen.findByRole("log");
    expect(log.textContent).toContain("Diagnose checkout-api 5xx");
  });

  /**
   * This screen has no prompt of its own. The dock is the one prompt in the
   * app — same component, every screen — which is what "just use same console
   * dock, don't rebuild anything" settled. The bespoke `Composer` that used to
   * mount here is deleted.
   */
  it("has exactly ONE prompt — the dock, mounted in its own column", async () => {
    renderAgent();
    // One, not zero: the dock lives inside this screen now, so its rail can be
    // a full-height sibling. And not two: the bespoke composer that used to sit
    // here is gone, which is what put a second input box on screen.
    // Scoped to the rail: the empty state's copy points the reader AT that
    // section by name, so a bare text query now matches twice.
    const rail = await screen.findByRole("complementary", { name: "Agent" });
    expect(within(rail).getByText("Recent runs")).toBeTruthy();
    expect(screen.getAllByRole("textbox")).toHaveLength(1);
  });

  it("draws the rail's three sections beside the transcript", async () => {
    renderAgent();
    const rail = await screen.findByRole("complementary", { name: "Agent" });
    expect(within(rail).getByText("Recent runs")).toBeTruthy();
    expect(within(rail).getByText("Skills")).toBeTruthy();
    expect(within(rail).getByText("MCP clients")).toBeTruthy();
  });

  it("starts a fresh run from the header's New question control", async () => {
    renderAgent();
    await userEvent.click(await screen.findByRole("button", { name: /new question/i }));
    expect(clearAgentRun).toHaveBeenCalledTimes(1);
  });

  it("gets its agent picker from the composer, not from the rail", async () => {
    setContexts([CTX]);
    tabs.setState(defaultState([CTX]));
    renderAgent();
    await screen.findByRole("complementary", { name: "Agent" });
    const picker = screen.getByRole("button", { name: /claude/i });
    // In the dock, which this screen mounts — not in the rail, where it briefly
    // lived. Choosing the agent is part of asking, and the composer is on every
    // screen while this rail is on one.
    const rail = screen.getByRole("complementary", { name: "Agent" });
    expect(rail.contains(picker)).toBe(false);
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
    renderAgent();
    const head = await screen.findByText(/^started /);
    expect(head.textContent).toContain("started 14:04");
    // Scoped to the HEAD. Every turn draws its own clock now, so the later
    // turn's 15:30 is legitimately on screen — what must not happen is the
    // head reading it.
    expect(head.textContent).not.toContain("15:30");
  });

  /**
   * Reported as "still no title shown": the derivation was wired into the rail
   * row and the saved session and NOTHING drew it on the screen, so the head
   * read `Agent` above `started 12:39 · 12 calls` and named the open
   * conversation nowhere.
   */
  it("names the open conversation in the head, from the question that opened it", async () => {
    useAgentRun.mockReturnValue(
      runState({
        turns: [
          {
            id: 1,
            role: "user",
            text: "show me what m01-prod-04-mongodb-0 is using",
            calls: [],
            at: new Date(2026, 0, 1, 12, 39).getTime(),
          },
        ],
      }),
    );
    renderAgent();
    // The filler opener is dropped and the first letter raised — the same
    // derivation the rail row uses, so one conversation is called one thing in
    // both places.
    await screen.findByText("What m01-prod-04-mongodb-0 is using");
  });

  it("names no conversation in the head when nothing has been asked", async () => {
    renderAgent();
    await screen.findByRole("complementary", { name: "Agent" });
    // The empty state has its own copy; a title over no conversation would be
    // a name for nothing.
    expect(screen.queryByTitle(/./)).toBeNull();
  });

  /**
   * The composition, not either half: this screen mounts the real dock, and
   * what was reported is what that dock drew INSIDE the full view — "Start
   * here" beneath a transcript, and a link to the screen already open.
   */
  it("mounts the dock as its own composer, offering neither suggestions nor a way here", async () => {
    const user = userEvent.setup();
    render(
      <ConsoleProvider>
        <Opener />
        <Agent route="/agent" />
      </ConsoleProvider>,
    );
    await screen.findByRole("complementary", { name: "Agent" });
    // The dock must be OPEN first. Collapsed it is a 34px strip with no body,
    // so both absences below would hold for a reason that has nothing to do
    // with this screen — the first draft of this test passed with the whole
    // fix reverted.
    await user.click(screen.getByRole("button", { name: "Open the console" }));
    expect(await screen.findByRole("textbox", { name: /prompt/i })).toBeTruthy();
    // This screen's own empty state says how to begin; the dock repeating it
    // is two explanations, one of them wrong about whether you have begun.
    expect(screen.queryByText(/start here/i)).toBeNull();
    expect(screen.queryByRole("button", { name: /full view/i })).toBeNull();
  });

  it("draws no started time for a run with no turns yet", async () => {
    renderAgent();
    // The rail is the settle signal now: the composer this used to wait on is
    // gone, since the dock is the prompt on every screen.
    await screen.findByRole("complementary", { name: "Agent" });
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
    renderAgent();
    expect(await screen.findByText(/3 calls/)).toBeTruthy();
  });

  it("shows no call count for a run with turns but no tool calls yet — an absent reading renders no reading", async () => {
    useAgentRun.mockReturnValue(
      runState({ turns: [{ id: 1, role: "user", text: "checkout-api is throwing 5xx", calls: [], at: 1 }] }),
    );
    renderAgent();
    await screen.findByText(/^started /);
    expect(screen.queryByText(/call/i)).toBeNull();
  });

  it("draws no line telling the reader to use the only prompt there is", () => {
    renderAgent();
    // §5's "Continue this run from the console at the bottom of the window"
    // said this screen and the dock were one conversation — worth saying when
    // the screen had its own composer and the dock was elsewhere. There is one
    // prompt now, directly beneath this transcript, so the sentence instructed
    // the reader to do the only thing they could.
    expect(screen.queryByText(/continue this run from the console/i)).toBeNull();
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
    renderAgent();
    expect(screen.getByText(/still answering the last question/i)).toBeTruthy();
  });

  it("lets the reader put that failure away", async () => {
    useAgentRun.mockReturnValue(runState({ error: "srelens is still answering." }));
    renderAgent();
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
    renderAgent();
    expect(screen.getByTestId("agent-desktop-only").textContent).toMatch(/runs in the srelens desktop app/i);
    // No composer, because there is nothing for it to reach.
    expect(screen.queryByRole("textbox")).toBeNull();
    // And not dressed as a failure: nothing has failed, nothing was asked.
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("still says what DOES work in the browser, rather than only what does not", () => {
    isTauri.mockReturnValue(false);
    renderAgent();
    expect(screen.getByText(/srelens server/)).toBeTruthy();
  });

  /** "Page looks empty" — it said nothing at all with no conversation open. */
  it("says what the agent actually does when there is no conversation yet", async () => {
    renderAgent();
    expect(await screen.findByText(/ask about this cluster/i)).toBeTruthy();
    // The three things none of which a blank pane implies: it drives a real
    // CLI, it reads through srelens's own tools, and a change stops for the
    // reader.
    expect(screen.getByText(/real agent CLI running on this machine/i)).toBeTruthy();
    expect(screen.getByText(/comes from the cluster rather than from memory/i)).toBeTruthy();
    expect(screen.getByText(/stops and asks you first/i)).toBeTruthy();
  });

  it("draws the transcript instead, once there is something in it", async () => {
    useAgentRun.mockReturnValue(
      runState({ turns: [{ id: 1, role: "user", text: "what is unhealthy", calls: [], at: 1 }] }),
    );
    renderAgent();
    expect(await screen.findByText("what is unhealthy")).toBeTruthy();
    // Not both: the explanation is for an empty screen, and leaving it above a
    // conversation would be a wall of text the reader has moved past.
    expect(screen.queryByText(/real agent CLI running on this machine/i)).toBeNull();
  });

  describe("the run head", () => {
    const withTurns = () =>
      useAgentRun.mockReturnValue(
        runState({
          turns: [
            { id: 1, role: "user", text: "why is it restarting", calls: [], at: Date.now() },
            { id: 2, role: "agent", text: "the pool shrank", calls: [], at: Date.now() },
          ],
        }),
      );

    it("sits the run's figures at the right-hand end", async () => {
      withTurns();
      renderAgent();
      const head = await screen.findByText(/^started /);
      // A figure about the run, beside the control that acts on the same run.
      // `ml-auto` is what pushes it right now that a title occupies the left —
      // the row itself no longer justifies its whole content to the end.
      expect(head.className).toContain("ml-auto");
    });

    it("copies the whole conversation from the head", async () => {
      const writeText = vi.fn<(text: string) => Promise<void>>().mockResolvedValue(undefined);
      Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
      withTurns();
      renderAgent();

      await userEvent.click(await screen.findByRole("button", { name: /copy the whole conversation/i }));
      // The same text the per-exchange control produces, so the two cannot
      // disagree about what the conversation says.
      expect(writeText).toHaveBeenCalledWith("why is it restarting\n\nthe pool shrank");
    });

    it("offers no copy over a conversation with nothing in it", async () => {
      renderAgent();
      await screen.findByRole("complementary", { name: "Agent" });
      expect(screen.queryByRole("button", { name: /copy the whole conversation/i })).toBeNull();
    });
  });
});