import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

vi.mock("../lib/notify", () => ({ notify: { success: vi.fn(), error: vi.fn(), info: vi.fn() } }));

const skillsLibMock = vi.hoisted(() => ({
  listSkills: vi.fn(),
  loadSkill: vi.fn(),
  saveSkill: vi.fn(),
  deleteSkill: vi.fn(),
}));
vi.mock("../lib/skills", () => skillsLibMock);

import { SkillsPanel } from "./SkillsPanel";
import type { Skill, SkillMeta } from "../lib/skills";

const METAS: SkillMeta[] = [
  { name: "alpha", description: "First skill" },
  { name: "zeta", description: "Last skill" },
];

const ALPHA: Skill = { name: "alpha", description: "First skill", body: "Body for alpha" };

describe("SkillsPanel", () => {
  beforeEach(() => {
    skillsLibMock.listSkills.mockReset().mockResolvedValue(METAS);
    skillsLibMock.loadSkill.mockReset().mockResolvedValue(ALPHA);
    skillsLibMock.saveSkill.mockReset().mockResolvedValue(undefined);
    skillsLibMock.deleteSkill.mockReset().mockResolvedValue(undefined);
  });

  it("lists skills by name and description", async () => {
    render(<SkillsPanel onClose={vi.fn()} />);

    expect(await screen.findByText("alpha")).toBeTruthy();
    expect(screen.getByText("First skill")).toBeTruthy();
    expect(screen.getByText("zeta")).toBeTruthy();
    expect(screen.getByText("Last skill")).toBeTruthy();
  });

  it('"New skill" opens an empty editor', async () => {
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");

    fireEvent.click(screen.getByRole("button", { name: "New skill" }));

    const nameInput = (await screen.findByLabelText("Skill name")) as HTMLInputElement;
    const descInput = screen.getByLabelText("Skill description") as HTMLInputElement;
    const bodyInput = screen.getByLabelText("Skill body") as HTMLTextAreaElement;
    expect(nameInput.value).toBe("");
    expect(descInput.value).toBe("");
    expect(bodyInput.value).toBe("");
  });

  it("selecting a skill loads it into the editor", async () => {
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");

    fireEvent.click(screen.getByText("alpha"));

    expect(skillsLibMock.loadSkill).toHaveBeenCalledWith("alpha");
    await waitFor(() => {
      expect((screen.getByLabelText("Skill name") as HTMLInputElement).value).toBe("alpha");
    });
    expect((screen.getByLabelText("Skill description") as HTMLInputElement).value).toBe("First skill");
    expect((screen.getByLabelText("Skill body") as HTMLTextAreaElement).value).toBe("Body for alpha");
  });

  it("Save calls saveSkill with the exact typed fields and refreshes the list", async () => {
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");

    fireEvent.click(screen.getByRole("button", { name: "New skill" }));
    fireEvent.change(await screen.findByLabelText("Skill name"), { target: { value: "crashloop-triage" } });
    fireEvent.change(screen.getByLabelText("Skill description"), {
      target: { value: "Systematic triage for a pod that keeps restarting" },
    });
    fireEvent.change(screen.getByLabelText("Skill body"), { target: { value: "Step 1: check the exit code." } });

    skillsLibMock.listSkills.mockResolvedValueOnce([
      ...METAS,
      { name: "crashloop-triage", description: "Systematic triage for a pod that keeps restarting" },
    ]);

    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(skillsLibMock.saveSkill).toHaveBeenCalledTimes(1));
    expect(skillsLibMock.saveSkill).toHaveBeenCalledWith({
      name: "crashloop-triage",
      description: "Systematic triage for a pod that keeps restarting",
      body: "Step 1: check the exit code.",
    });
    // The list is refreshed after a successful save.
    await waitFor(() => expect(skillsLibMock.listSkills).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("crashloop-triage")).toBeTruthy();
  });

  it("Delete, once confirmed, calls deleteSkill with the skill's name", async () => {
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");

    fireEvent.click(screen.getByRole("button", { name: "Delete alpha" }));
    expect(await screen.findByText(/Delete "alpha"/)).toBeTruthy();

    skillsLibMock.listSkills.mockResolvedValueOnce([METAS[1]]);
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(skillsLibMock.deleteSkill).toHaveBeenCalledWith("alpha"));
  });

  it("Delete does not call deleteSkill when the confirmation is cancelled", async () => {
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");

    fireEvent.click(screen.getByRole("button", { name: "Delete alpha" }));
    await screen.findByText(/Delete "alpha"/);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(skillsLibMock.deleteSkill).not.toHaveBeenCalled();
  });
});
