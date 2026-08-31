import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RunsRail } from "./RunsRail";
import { getAgentRun, resetAgentRun } from "../../lib/agentRun";

const { listSessions, listSkills } = vi.hoisted(() => ({
  listSessions: vi.fn(),
  listSkills: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  listSessions,
  listSkills,
}));

beforeEach(() => {
  vi.clearAllMocks();
  listSessions.mockResolvedValue([]);
  listSkills.mockResolvedValue([]);
  // The real store, not a double — the whole point of this suite's switch
  // test is proving the rail reaches the SAME `activeSkills` the composer
  // writes to, and a mocked `useAgentRun` can only ever assert the render,
  // never whether the write landed anywhere real.
  resetAgentRun();
});

describe("the agent screen's rail", () => {
  it("lists recent runs by title and when they were last touched", async () => {
    listSessions.mockResolvedValue([{ id: "s1", title: "Diagnose checkout-api 5xx", createdAt: 1, updatedAt: 2 }]);
    render(<RunsRail />);
    expect(await screen.findByText("Diagnose checkout-api 5xx")).toBeTruthy();
  });

  it("reads the last-touched time off updatedAt, not createdAt", async () => {
    // G3: a session whose two timestamps disagree, far enough apart that the
    // two readings render different words — `relativeTime`'s buckets are
    // minutes/hours/days, so one second apart would round to the same
    // "just now" either way and prove nothing.
    const now = Date.now();
    listSessions.mockResolvedValue([
      { id: "s1", title: "Diagnose", createdAt: now - 3 * 60 * 60 * 1000, updatedAt: now - 5 * 60 * 1000 },
    ]);
    render(<RunsRail />);
    await screen.findByText("Diagnose");
    expect(await screen.findByText("5m ago")).toBeTruthy();
    expect(screen.queryByText("3h ago")).toBeNull();
  });

  it("draws no call count, because none is stored", async () => {
    listSessions.mockResolvedValue([{ id: "s1", title: "Diagnose", createdAt: 1, updatedAt: 2 }]);
    render(<RunsRail />);
    await screen.findByText("Diagnose");
    expect(screen.queryByText(/calls/i)).toBeNull();
  });

  it("draws a skill's name and description, and no usage count", async () => {
    listSkills.mockResolvedValue([{ name: "Rollout forensics", description: "Correlates a revision diff" }]);
    render(<RunsRail />);
    expect(await screen.findByText("Rollout forensics")).toBeTruthy();
    expect(await screen.findByText("Correlates a revision diff")).toBeTruthy();
    expect(screen.queryByText(/used/i)).toBeNull();
    expect(screen.queryByText(/×/)).toBeNull();
  });

  it("says why there is no connected-clients list rather than drawing an empty one", async () => {
    render(<RunsRail />);
    expect(await screen.findByText(/does not know which clients are connected/i)).toBeTruthy();
  });

  it("says so when there are no recent runs, rather than drawing nothing at all", async () => {
    render(<RunsRail />);
    expect(await screen.findByText(/no recent runs/i)).toBeTruthy();
  });

  // I6: `.catch(() => setSessions([]))` rendered "No recent runs yet." for a
  // failed `listSessions` exactly as it would for an account with none — a
  // confident false statement standing in for a failure this rail never told
  // the reader about.
  it("says the read failed, rather than claiming there are no recent runs, when listSessions rejects", async () => {
    listSessions.mockRejectedValue(new Error("no such command: session_list"));
    render(<RunsRail />);
    expect(await screen.findByText(/could not be checked/i)).toBeTruthy();
    expect(await screen.findByText(/session_list/)).toBeTruthy();
    expect(screen.queryByText(/no recent runs/i)).toBeNull();
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
