import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RunsRail } from "./RunsRail";
import { askAgent, getAgentRun, resetAgentRun } from "../../lib/agentRun";

const { listSessions, loadSession, saveSession, deleteSession, listSkills, startChat, sendChat, listAgents } =
  vi.hoisted(() => ({
  listSessions: vi.fn(),
  loadSession: vi.fn(),
  saveSession: vi.fn(),
  deleteSession: vi.fn(),
  listSkills: vi.fn(),
  startChat: vi.fn(),
  sendChat: vi.fn(),
  listAgents: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  listSessions,
  loadSession,
  saveSession,
  deleteSession,
  listSkills,
  startChat,
  sendChat,
  listAgents,
}));

beforeEach(() => {
  vi.clearAllMocks();
  listSessions.mockResolvedValue([]);
  saveSession.mockResolvedValue(undefined);
  deleteSession.mockResolvedValue(undefined);
  listSkills.mockResolvedValue([]);
  startChat.mockResolvedValue("sess-1");
  sendChat.mockResolvedValue(null);
  listAgents.mockResolvedValue([
    { kind: "claude", label: "Claude", available: true, gated: false, path: "/c" },
  ]);
  // The real store, not a double — the whole point of this suite's switch
  // test is proving the rail reaches the SAME `activeSkills` the composer
  // writes to, and a mocked `useAgentRun` can only ever assert the render,
  // never whether the write landed anywhere real.
  resetAgentRun();
});

describe("the agent screen's rail", () => {
  /**
   * `Recent runs` lists THIS window's conversations now, and every entry
   * opens. It used to list `listSessions()` — the sessions CLASSIC persisted —
   * which the reader reported as wrong from use: entries 14 to 22 days old
   * from a different UI, none of their own conversations in it (nothing in the
   * new design persists, #395), and rows that were plain `<div>`s so clicking
   * did nothing.
   */
  it("says nothing has been asked yet when there is nothing asked and nothing saved", async () => {
    render(<RunsRail />);
    expect(await screen.findByText(/no questions yet/i)).toBeTruthy();
  });

  /**
   * Conversations are persisted now (#395). The rail lists what is on disk as
   * well as what this window holds, so a restart does not look like a fresh
   * install — which is what "conversations are still not showing up" was.
   *
   * The earlier defect was different and is still fixed: it listed CLASSIC's
   * sessions as if they were the reader's runs, and the rows could not be
   * opened at all.
   */
  it("lists a conversation saved by an earlier session, and opens it on click", async () => {
    listSessions.mockResolvedValue([
      { id: "s1", title: "check mongodb deployment and find optimisations", createdAt: 1, updatedAt: 2 },
    ]);
    loadSession.mockResolvedValue({
      id: "s1",
      title: "check mongodb deployment and find optimisations",
      createdAt: 1,
      updatedAt: 2,
      contexts: [],
      skills: [],
      cliSessionId: null,
      agentKind: "claude",
      messages: [
        {
          v: 1,
          key: "prod-eu|Pod||m01-prod-04-mongodb-0",
          label: "Pod/m01-prod-04-mongodb-0",
          turns: [{ id: 1, role: "user", text: "check mongodb deployment", calls: [], at: 1 }],
          gates: [],
        },
      ],
    });
    render(<RunsRail />);

    const row = await screen.findByRole("button", { name: /check mongodb deployment/i });
    // Marked as on disk rather than counted, since its transcript is not
    // loaded until it is opened.
    expect(row.textContent).toMatch(/saved/i);

    await userEvent.click(row);
    // Hydrated into a real run, under its own subject key.
    await vi.waitFor(() => {
      expect(getAgentRun().turns.map((t) => t.text)).toContain("check mongodb deployment");
    });
  });

  it("lists a conversation once one has been started, by its subject", async () => {
    sendChat.mockResolvedValue(null);
    await askAgent("summarise this stream", {
      about: { cluster: "prod-eu", namespace: "ns", kind: "Pod", name: "ai-editor", surface: "logs" },
      route: "/logs/Pod/ns/ai-editor",
    });
    render(<RunsRail />);
    // The subject, which is what a reader recognises — not the question text
    // and not the route.
    expect(await screen.findByRole("button", { name: /Pod\/ai-editor/ })).toBeTruthy();
  });

  it("opens the conversation that is clicked", async () => {
    sendChat.mockResolvedValue(null);
    await askAgent("about the list", { about: { cluster: "prod-eu" }, route: "/k/statefulsets" });
    await askAgent("about the pod", {
      about: { cluster: "prod-eu", namespace: "ns", kind: "Pod", name: "ai-editor" },
      route: "/k/Pod/ns/ai-editor",
    });
    render(<RunsRail />);

    // The pod was asked last, so it is active; clicking the list switches.
    await userEvent.click(screen.getByRole("button", { name: /statefulsets/ }));
    expect(getAgentRun().turns.map((t) => t.text)).toContain("about the list");
  });

  it("marks which conversation is on screen", async () => {
    sendChat.mockResolvedValue(null);
    await askAgent("a", { about: { cluster: "prod-eu" }, route: "/k/statefulsets" });
    render(<RunsRail />);
    expect(screen.getByRole("button", { name: /statefulsets/ }).getAttribute("aria-current")).toBe("true");
  });

  it("says a conversation is answering rather than counting its questions", async () => {
    sendChat.mockImplementation(() => new Promise<string | null>(() => {}));
    void askAgent("long one", { about: { cluster: "prod-eu" }, route: "/k/statefulsets" });
    await vi.waitFor(() => expect(sendChat).toHaveBeenCalled());
    render(<RunsRail />);
    expect(await screen.findByText(/answering/i)).toBeTruthy();
  });

  it("says the read failed, rather than claiming there are no saved skills, when listSkills rejects", async () => {
    listSkills.mockRejectedValue(new Error("no such command: skill_list"));
    render(<RunsRail />);
    expect(await screen.findByText(/could not be checked/i)).toBeTruthy();
    expect(await screen.findByText(/skill_list/)).toBeTruthy();
    expect(screen.queryByText(/no skills saved/i)).toBeNull();
  });

  it("writes an activated skill into the shared run store, not merely its own render", async () => {
    listSkills.mockResolvedValue([{ name: "Rollout forensics", description: "Correlates a revision diff" }]);
    render(<RunsRail />);
    const toggle = await screen.findByRole("switch", { name: /activate rollout forensics/i });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(getAgentRun().activeSkills).toEqual([]);

    await userEvent.click(toggle);

    // Observable in the STORE, not merely in this component's own re-render —
    // the assertion a mocked `useAgentRun` could never make, and the one that
    // tells "wired" apart from "inert".
    expect(getAgentRun().activeSkills).toEqual(["Rollout forensics"]);
    expect(toggle.getAttribute("aria-checked")).toBe("true");

    await userEvent.click(toggle);
    expect(getAgentRun().activeSkills).toEqual([]);
  });
});
