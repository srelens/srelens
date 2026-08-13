import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeCommandMock } = vi.hoisted(() => ({ invokeCommandMock: vi.fn() }));
vi.mock("../transport/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../transport/transport")>();
  return { ...actual, invokeCommand: invokeCommandMock };
});

import { listSkills, loadSkill, saveSkill, deleteSkill, type Skill, type SkillMeta } from "./skills";

describe("skills", () => {
  beforeEach(() => invokeCommandMock.mockReset());

  it("listSkills calls skills_list with no args and returns the metas as-is", async () => {
    const metas: SkillMeta[] = [
      { name: "alpha", description: "First skill" },
      { name: "zeta", description: "Last skill" },
    ];
    invokeCommandMock.mockResolvedValue(metas);

    await expect(listSkills()).resolves.toEqual(metas);
    expect(invokeCommandMock).toHaveBeenCalledWith("skills_list");
  });

  it("loadSkill calls skill_load with the name and returns the full skill", async () => {
    const skill: Skill = {
      name: "crashloop-triage",
      description: "Systematic triage for a pod that keeps restarting",
      body: "Step 1: check the exit code.",
    };
    invokeCommandMock.mockResolvedValue(skill);

    await expect(loadSkill("crashloop-triage")).resolves.toEqual(skill);
    expect(invokeCommandMock).toHaveBeenCalledWith("skill_load", { name: "crashloop-triage" });
  });

  it("saveSkill calls skill_save with the skill wrapped under a `skill` key", async () => {
    const skill: Skill = {
      name: "crashloop-triage",
      description: "Systematic triage for a pod that keeps restarting",
      body: "Step 1: check the exit code.",
    };
    invokeCommandMock.mockResolvedValue(undefined);

    await saveSkill(skill);
    expect(invokeCommandMock).toHaveBeenCalledWith("skill_save", { skill });
  });

  it("deleteSkill calls skill_delete with the name", async () => {
    invokeCommandMock.mockResolvedValue(undefined);

    await deleteSkill("gone");
    expect(invokeCommandMock).toHaveBeenCalledWith("skill_delete", { name: "gone" });
  });
});
