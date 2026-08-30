import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { RunsRail } from "./RunsRail";

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
});

describe("the agent screen's rail", () => {
  it("lists recent runs by title and when they were last touched", async () => {
    listSessions.mockResolvedValue([{ id: "s1", title: "Diagnose checkout-api 5xx", createdAt: 1, updatedAt: 2 }]);
    render(<RunsRail />);
    expect(await screen.findByText("Diagnose checkout-api 5xx")).toBeTruthy();
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

  it("lets a skill be activated for this run via its own switch", async () => {
    listSkills.mockResolvedValue([{ name: "Rollout forensics", description: "Correlates a revision diff" }]);
    render(<RunsRail />);
    const toggle = await screen.findByRole("switch", { name: /activate rollout forensics/i });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
  });
});
