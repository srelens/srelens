import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeCommandMock } = vi.hoisted(() => ({ invokeCommandMock: vi.fn() }));
vi.mock("../transport/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../transport/transport")>();
  return { ...actual, invokeCommand: invokeCommandMock };
});

import { listSkills, loadSkill, saveSkill, deleteSkill, type Skill, type SkillMeta, revealSkill, skillsDirPath, parseSkillFile, skillNameProblem } from "./skills";

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

  it("skillsDirPath asks for the folder skill files are kept in", async () => {
    invokeCommandMock.mockResolvedValue("/home/dana/.config/srelens/assistant/skills");
    await expect(skillsDirPath()).resolves.toBe("/home/dana/.config/srelens/assistant/skills");
    expect(invokeCommandMock).toHaveBeenCalledWith("skills_dir_path");
  });

  it("revealSkill names the skill to select, or none for the folder itself", async () => {
    invokeCommandMock.mockResolvedValue(undefined);
    await revealSkill("team-runbook");
    expect(invokeCommandMock).toHaveBeenLastCalledWith("skill_reveal", { name: "team-runbook" });
    await revealSkill();
    expect(invokeCommandMock).toHaveBeenLastCalledWith("skill_reveal", { name: null });
  });

  describe("parseSkillFile", () => {
    it("reads the name, description and instructions out of a file srelens wrote", () => {
      const text = "---\nname: team-rollback\ndescription: How this team rolls back\n---\nStep 1.\n\nStep 2.\n";
      expect(parseSkillFile(text, "whatever.md")).toEqual({
        name: "team-rollback",
        description: "How this team rolls back",
        body: "Step 1.\n\nStep 2.\n",
      });
    });

    it("reads a file with Windows line endings the same", () => {
      const text = "---\r\nname: a\r\ndescription: b\r\n---\r\nBody\r\n";
      expect(parseSkillFile(text, "a.md")).toEqual({ name: "a", description: "b", body: "Body\n" });
    });

    it("takes a file with no front matter as instructions, named for the file", () => {
      expect(parseSkillFile("Just check the events.\n", "/Users/dana/notes/My Runbook.md")).toEqual({
        name: "My-Runbook",
        description: "",
        body: "Just check the events.\n",
      });
    });

    it("takes a file whose front matter never closes as instructions, whole", () => {
      const text = "---\nname: x\nStep 1\n";
      expect(parseSkillFile(text, "x.md")).toEqual({ name: "x", description: "", body: text });
    });

    it("falls back to the file's name when the front matter gives none", () => {
      expect(parseSkillFile("---\ndescription: d\n---\nB", "C:\\skills\\pvc-resize.markdown").name).toBe("pvc-resize");
    });

    it("keeps a rule line inside the instructions as part of them", () => {
      const text = "---\nname: a\n---\nOne\n---\nTwo\n";
      expect(parseSkillFile(text, "a.md").body).toBe("One\n---\nTwo\n");
    });
  });

  describe("skillNameProblem", () => {
    it("accepts the names the backend accepts", () => {
      for (const name of ["oomkilled", "team-rollback", "v1.2_final", "A"]) expect(skillNameProblem(name)).toBeNull();
    });

    it("says what is wrong with a name the backend would refuse", () => {
      expect(skillNameProblem("")).toBe("Give the skill a name.");
      for (const name of ["my skill", "a/b", "..\\x", "émile", "a b"]) expect(skillNameProblem(name)).toMatch(/letters, numbers/);
    });
  });

  it("deleteSkill calls skill_delete with the name", async () => {
    invokeCommandMock.mockResolvedValue(undefined);

    await deleteSkill("gone");
    expect(invokeCommandMock).toHaveBeenCalledWith("skill_delete", { name: "gone" });
  });
});
