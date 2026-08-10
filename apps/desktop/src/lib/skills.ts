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
