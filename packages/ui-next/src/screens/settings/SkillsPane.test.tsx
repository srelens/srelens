import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const core = vi.hoisted(() => ({ listSkills: vi.fn(), loadSkill: vi.fn(), deleteSkill: vi.fn() }));
vi.mock("@srelens/core", async (orig) => ({ ...(await orig<typeof import("@srelens/core")>()), ...core }));
const setSkillActive = vi.hoisted(() => vi.fn());
vi.mock("../../lib/agentRun", () => ({ setSkillActive }));

import { settingsStorage } from "@srelens/core";
import { SKILL_DEFAULTS_KEY, __resetSkillDefaultsForTests, getSkillDefaults } from "../../lib/skillDefaults";
import { SkillsPane } from "./SkillsPane";

/** Bodies sized so the estimates are round: a quarter of the characters. */
const BODIES: Record<string, string> = {
  "crashloop-triage": "x".repeat(4000),
  "pending-pod": "x".repeat(2000),
  "team-runbook": "x".repeat(800),
};
const METAS = [
  { name: "crashloop-triage", description: "Triage a pod stuck in CrashLoopBackOff", builtin: true },
  { name: "pending-pod", description: "Diagnose a pod stuck in Pending", builtin: true },
  { name: "team-runbook", description: "How this team rolls back a release", builtin: false },
];

const card = (name: string) => document.querySelector(`[data-skill="${name}"]`) as HTMLElement;
const toggle = (name: string) => screen.getByRole("switch", { name: `${name}: on for every new conversation` });
const cost = () => document.querySelector('[data-slot="skills-cost"]')?.textContent;

beforeEach(() => {
  settingsStorage.removeItem(SKILL_DEFAULTS_KEY);
  __resetSkillDefaultsForTests();
  setSkillActive.mockReset();
  core.listSkills.mockReset().mockResolvedValue(METAS);
  core.loadSkill.mockReset().mockImplementation(async (name: string) => ({
    ...METAS.find((m) => m.name === name)!,
    body: BODIES[name],
  }));
  core.deleteSkill.mockReset().mockResolvedValue(undefined);
});

async function shown() {
  render(<SkillsPane />);
  await screen.findByText("crashloop-triage");
}

describe("SkillsPane", () => {
  it("lists every installed skill with what it is and where it came from", async () => {
    await shown();
    expect(screen.getByText("Installed (3)")).toBeDefined();
    expect(within(card("crashloop-triage")).getByText("Triage a pod stuck in CrashLoopBackOff")).toBeDefined();
    expect(within(card("crashloop-triage")).getByText("Bundled")).toBeDefined();
    expect(within(card("crashloop-triage")).getByText("Ships with srelens")).toBeDefined();
    expect(within(card("team-runbook")).getByText("User")).toBeDefined();
  });

  it("says about how many tokens each skill adds when it is on", async () => {
    await shown();
    expect(within(card("crashloop-triage")).getByText("≈1k tokens when on")).toBeDefined();
    expect(within(card("pending-pod")).getByText("≈500 tokens when on")).toBeDefined();
    expect(within(card("team-runbook")).getByText("≈200 tokens when on")).toBeDefined();
  });

  it("starts with nothing on, and nothing added to a question", async () => {
    await shown();
    expect(toggle("crashloop-triage").getAttribute("aria-checked")).toBe("false");
    expect(cost()).toBe("≈0");
    expect(screen.getByText(/from 0 skills that are on/)).toBeDefined();
  });

  /**
   * The figure is what the reader's choices cost: a skill that is on has its
   * whole instructions sent with every question, so the total is the sum of
   * the ones that are on, and it moves as they are switched.
   */
  it("adds up what the skills that are on send with every question", async () => {
    await shown();
    await userEvent.click(toggle("crashloop-triage"));
    expect(cost()).toBe("≈1k");
    expect(screen.getByText(/from 1 skill that is on/)).toBeDefined();
    await userEvent.click(toggle("pending-pod"));
    expect(cost()).toBe("≈1.5k");
    await userEvent.click(toggle("crashloop-triage"));
    expect(cost()).toBe("≈500");
  });

  it("keeps a skill that is switched on, and turns it on for this window too", async () => {
    await shown();
    await userEvent.click(toggle("pending-pod"));
    expect(getSkillDefaults()).toEqual(["pending-pod"]);
    expect(toggle("pending-pod").getAttribute("aria-checked")).toBe("true");
    expect(setSkillActive).toHaveBeenCalledWith("pending-pod", true);
  });

  it("opens with the skills kept from before already on", async () => {
    settingsStorage.setItem(SKILL_DEFAULTS_KEY, JSON.stringify(["team-runbook"]));
    await shown();
    expect(toggle("team-runbook").getAttribute("aria-checked")).toBe("true");
    expect(cost()).toBe("≈200");
  });

  it("does not count a kept name whose skill is no longer installed", async () => {
    settingsStorage.setItem(SKILL_DEFAULTS_KEY, JSON.stringify(["deleted-long-ago", "pending-pod"]));
    await shown();
    expect(cost()).toBe("≈500");
    expect(screen.getByText(/from 1 skill that is on/)).toBeDefined();
  });

  it("lists a skill whose instructions cannot be read, without a size, and says it is not counted", async () => {
    core.loadSkill.mockImplementation(async (name: string) => {
      if (name === "pending-pod") throw new Error("could not read");
      return { ...METAS.find((m) => m.name === name)!, body: BODIES[name] };
    });
    await shown();
    expect(within(card("pending-pod")).getByText("size unknown")).toBeDefined();
    await userEvent.click(toggle("pending-pod"));
    expect(cost()).toBe("≈0");
    expect(screen.getByText("1 skill could not be read and is not counted.")).toBeDefined();
  });

  it("narrows the list to the skills that match the search, by name or description", async () => {
    await shown();
    await userEvent.type(screen.getByRole("textbox", { name: "Search installed skills" }), "rolls back");
    expect(card("team-runbook")).not.toBeNull();
    expect(card("crashloop-triage")).toBeNull();
    await userEvent.clear(screen.getByRole("textbox", { name: "Search installed skills" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Search installed skills" }), "PENDING");
    expect(card("pending-pod")).not.toBeNull();
    expect(card("team-runbook")).toBeNull();
  });

  it("says so when nothing matches the search", async () => {
    await shown();
    await userEvent.type(screen.getByRole("textbox", { name: "Search installed skills" }), "zzz");
    expect(screen.getByText("No skill matches")).toBeDefined();
  });

  it("reads the skills again on Refresh", async () => {
    await shown();
    core.listSkills.mockResolvedValue([...METAS, { name: "new-one", description: "Just added", builtin: false }]);
    core.loadSkill.mockResolvedValue({ name: "new-one", description: "", body: "abcd" });
    await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(await screen.findByText("new-one")).toBeDefined();
    expect(screen.getByText("Installed (4)")).toBeDefined();
  });

  it("offers to uninstall a user's skill, and not one that ships with the app", async () => {
    await shown();
    expect(within(card("team-runbook")).getByRole("button", { name: "Uninstall" })).toBeDefined();
    expect(within(card("crashloop-triage")).queryByRole("button", { name: "Uninstall" })).toBeNull();
  });

  it("asks before uninstalling, and leaves the skill alone when the answer is no", async () => {
    await shown();
    await userEvent.click(within(card("team-runbook")).getByRole("button", { name: "Uninstall" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(core.deleteSkill).not.toHaveBeenCalled();
  });

  it("uninstalls on confirmation, stops keeping it on, and reads the list again", async () => {
    settingsStorage.setItem(SKILL_DEFAULTS_KEY, JSON.stringify(["team-runbook"]));
    await shown();
    core.listSkills.mockResolvedValue(METAS.slice(0, 2));
    await userEvent.click(within(card("team-runbook")).getByRole("button", { name: "Uninstall" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Uninstall" }));
    await waitFor(() => expect(card("team-runbook")).toBeNull());
    expect(core.deleteSkill).toHaveBeenCalledExactlyOnceWith("team-runbook");
    expect(getSkillDefaults()).toEqual([]);
    expect(setSkillActive).toHaveBeenCalledWith("team-runbook", false);
  });

  it("says so when a skill could not be removed, and keeps it listed", async () => {
    core.deleteSkill.mockRejectedValue(new Error("permission denied"));
    await shown();
    await userEvent.click(within(card("team-runbook")).getByRole("button", { name: "Uninstall" }));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Uninstall" }));
    expect(await screen.findByText("Could not remove the skill")).toBeDefined();
    expect(card("team-runbook")).not.toBeNull();
  });

  it("says so when the skills could not be loaded, and tries again when asked", async () => {
    core.listSkills.mockRejectedValueOnce(new Error("store unavailable"));
    render(<SkillsPane />);
    expect(await screen.findByText("Could not load skills")).toBeDefined();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("crashloop-triage")).toBeDefined();
  });

  it("says there are none, rather than drawing an empty list", async () => {
    core.listSkills.mockResolvedValue([]);
    render(<SkillsPane />);
    expect(await screen.findByText("No skills")).toBeDefined();
    expect(screen.getByText("Installed (0)")).toBeDefined();
  });

  it("claims nothing it does not do: no registry, no on-demand loading", async () => {
    await shown();
    expect(screen.queryByText(/Browse/)).toBeNull();
    expect(screen.queryByText(/saved/)).toBeNull();
    expect(screen.getByText(/is sent with every question/)).toBeDefined();
  });
});
