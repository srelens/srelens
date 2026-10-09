// Typed wrappers for the four skill commands (backend: `assistant_skills.rs`,
// Task 22) — a disk-backed store for srelens-defined "skills": reusable
// instruction files an AI agent can draw on. Field names are camelCase to
// mirror the Rust `Skill`/`SkillMeta` structs exactly
// (`#[serde(rename_all = "camelCase")]`) — no translation happens at this
// boundary.
import { invokeCommand } from "../transport/transport";

/** Picker metadata only — no `body`, so listing skills stays cheap even once
 * a body grows long. */
export interface SkillMeta {
  name: string;
  description: string;
  /** True for a srelens-shipped default skill with no user override — the UI
   * badges these and doesn't offer delete (there's no file to remove). */
  builtin?: boolean;
  /** Where the skill's file is, for opening and editing it by hand. Absent
   *  for a shipped default nobody has overridden: it has no file. (#851) */
  path?: string | null;
  /** True for a user file standing in for a shipped default of the same
   *  name: deleting it brings the default back rather than removing the
   *  skill. */
  overridesBuiltin?: boolean;
}

/** A full skill, including its instructions body. */
export interface Skill extends SkillMeta {
  body: string;
}

/** Saved skills, sorted by name. */
export function listSkills(): Promise<SkillMeta[]> {
  return invokeCommand("skills_list");
}

/** Load one full skill (including its body) by name. */
export function loadSkill(name: string): Promise<Skill> {
  return invokeCommand("skill_load", { name });
}

/** Persist a skill, creating or overwriting its file. */
export function saveSkill(skill: Skill): Promise<void> {
  return invokeCommand("skill_save", { skill });
}

/** Delete a skill's file. */
export function deleteSkill(name: string): Promise<void> {
  return invokeCommand("skill_delete", { name });
}

/** The folder the user's skill files are kept in. */
export function skillsDirPath(): Promise<string> {
  return invokeCommand("skills_dir_path");
}

/**
 * Open the skills folder in the OS file manager — with `name`'s file selected
 * where it has one and the platform can select a file.
 */
export function revealSkill(name?: string): Promise<void> {
  return invokeCommand("skill_reveal", { name: name ?? null });
}
