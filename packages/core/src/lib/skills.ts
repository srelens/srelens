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

/** What a skill may be called: the same rule the backend holds a name to. */
const SKILL_NAME = /^[A-Za-z0-9._-]+$/;

/** Why `name` cannot be a skill's name, or `null` when it can. */
export function skillNameProblem(name: string): string | null {
  if (name === "") return "Give the skill a name.";
  if (!SKILL_NAME.test(name)) return "Use letters, numbers, dots, dashes and underscores only, with no spaces.";
  return null;
}

/**
 * Read a skill out of a file's text, for installing one the reader has on
 * disk (#851).
 *
 * The file srelens itself writes starts with front matter — `---`, a `name:`
 * and a `description:` line, `---` — and the instructions follow. A file with
 * no front matter is still a skill someone wrote by hand: its instructions are
 * the whole text, and its name is suggested from the file's own.
 *
 * Nothing here decides whether the result may be installed. The name it
 * returns is a suggestion for a form the reader can still change, and is
 * checked there with {@link skillNameProblem}.
 */
export function parseSkillFile(text: string, fileName: string): Skill {
  const normalized = text.replace(/\r\n/g, "\n");
  const fromFile = fileName
    .replace(/^.*[\\/]/, "")
    .replace(/\.(md|markdown|txt)$/i, "")
    .trim()
    .replace(/\s+/g, "-")
    .replace(/[^A-Za-z0-9._-]/g, "");
  const closing = normalized.startsWith("---\n") ? normalized.indexOf("\n---\n", 3) : -1;
  if (closing === -1) return { name: fromFile, description: "", body: normalized };
  let name = "";
  let description = "";
  for (const line of normalized.slice(4, closing).split("\n")) {
    if (line.startsWith("name:")) name = line.slice("name:".length).trim();
    else if (line.startsWith("description:")) description = line.slice("description:".length).trim();
  }
  return { name: name || fromFile, description, body: normalized.slice(closing + "\n---\n".length) };
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
