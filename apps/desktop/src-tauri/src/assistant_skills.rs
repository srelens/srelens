//! Disk-backed store for srelens-defined "skills" — reusable instruction
//! files an AI agent can draw on (Task 22; wired into a chat by Task 23).
//!
//! Each skill lives as one markdown file with a small front-matter header:
//!
//! ```text
//! ---
//! name: crashloop-triage
//! description: Systematic triage for a pod that keeps restarting
//! ---
//! <body markdown>
//! ```
//!
//! under `<config>/assistant/skills/<name>.md`. Front-matter parsing mirrors
//! the pattern in `srelens_mcp::prompts::parse_prompt_file` (strip the
//! leading `---\n`, split on `\n---\n`, normalize CRLF -> LF) but does NOT
//! pull in `serde_yaml` — that crate isn't already a dependency of this
//! crate, and a two-field header doesn't need a YAML parser: this hand-rolls
//! reading exactly the `name:` and `description:` lines instead.
//!
//! As with `assistant_history`, the `#[tauri::command]` wrappers at the
//! bottom only resolve the app config dir and delegate; every real decision
//! lives in the pure `fn`s above them, which take a `dir: &Path` so tests can
//! drive them against a throwaway temp directory without touching the real
//! app config.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// A full skill: its instructions plus the metadata used to pick it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// Picker metadata only — everything but `body`, so listing skills stays
/// cheap even once a body grows long.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
}

/// `<base>/assistant/skills` — where skill `.md` files live.
fn skills_dir(base: &Path) -> PathBuf {
    base.join("assistant").join("skills")
}

fn skill_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.md"))
}

/// Reject any name that isn't `^[A-Za-z0-9._-]+$` — in particular, one
/// containing `/` or `\`, so a skill can never resolve outside `skills_dir`
/// via a crafted name like `../evil` or `a/b`. No `regex` dependency: this is
/// small enough to check by hand, character by character.
fn validate_name(name: &str) -> Result<(), String> {
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "invalid skill name {name:?}: must match ^[A-Za-z0-9._-]+$ (no `/`, `\\`, or other separators)"
        ))
    }
}

/// Split `---\nname: ...\ndescription: ...\n---\n<body>` and parse the
/// header. Unlike `parse_prompt_file`, the header is hand-rolled rather than
/// YAML: only the `name:` and `description:` lines are recognised, and any
/// other line in the header is silently ignored (forward-compatible with a
/// file written for a newer srelens that adds more front-matter fields).
pub fn parse_skill(text: &str) -> Result<Skill, String> {
    // Normalize line endings: convert CRLF to LF so the parser works on
    // Windows-authored files without misleading error messages.
    let text = text.replace("\r\n", "\n");

    let rest = text
        .strip_prefix("---\n")
        .ok_or("missing front matter: the file must start with `---`")?;
    let (header, body) = rest
        .split_once("\n---\n")
        .ok_or("unterminated front matter: no closing `---` line")?;

    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    for line in header.lines() {
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("description:") {
            description = Some(value.trim().to_string());
        }
    }

    let name = match name {
        Some(n) if !n.is_empty() => n,
        _ => return Err("front matter `name` must not be empty".to_string()),
    };
    validate_name(&name)?;

    Ok(Skill {
        name,
        description: description.unwrap_or_default(),
        body: body.to_string(),
    })
}

/// Render a skill back to its on-disk markdown form. Round-trips with
/// `parse_skill`: `parse_skill(&to_markdown(&skill))` yields a `Skill` equal
/// to `skill` (given a name/description that don't themselves contain a
/// newline — this is a minimal hand-rolled format, not a general YAML
/// serializer).
pub fn to_markdown(skill: &Skill) -> String {
    format!(
        "---\nname: {}\ndescription: {}\n---\n{}",
        skill.name, skill.description, skill.body
    )
}

/// Write `<name>.md.tmp` then rename onto `<name>.md` — a crash mid-write (or
/// a concurrent read) never observes a half-written skill file.
fn write_skill_atomic(dir: &Path, skill: &Skill) -> Result<(), String> {
    validate_name(&skill.name)?;
    fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let path = skill_path(dir, &skill.name);
    let tmp = dir.join(format!("{}.md.tmp", skill.name));
    fs::write(&tmp, to_markdown(skill)).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("could not finalize {}: {e}", path.display()))
}

/// Read one skill's file back off disk by name. A missing file (unknown
/// name) is reported as a clear, name-carrying `Err` rather than a raw IO
/// message.
fn read_skill(dir: &Path, name: &str) -> Result<Skill, String> {
    validate_name(name)?;
    let path = skill_path(dir, name);
    let raw = fs::read_to_string(&path).map_err(|_| format!("no skill found named {name:?}"))?;
    parse_skill(&raw)
}

/// Remove a skill's file. Removing a name that has no file is not an error —
/// deleting is idempotent, matching `assistant_history::delete_session`.
fn delete_skill(dir: &Path, name: &str) -> Result<(), String> {
    validate_name(name)?;
    let path = skill_path(dir, name);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not delete {}: {e}", path.display())),
    }
}

/// Every `*.md` in `dir`, parsed to `SkillMeta` and sorted by name. A missing
/// directory is silent — a user who never created one has no skills, not an
/// error.
fn list_skills(dir: &Path) -> Result<Vec<SkillMeta>, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("could not read {}: {e}", dir.display())),
    };

    let mut metas = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| format!("could not read entry in {}: {e}", dir.display()))?.path();
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")) {
            continue;
        }
        let raw = fs::read_to_string(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
        let skill = parse_skill(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
        metas.push(SkillMeta { name: skill.name, description: skill.description });
    }
    metas.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(metas)
}

/// Resolve `<app config dir>/assistant/skills`, creating it if needed — the
/// one bit of app-specific wiring the pure helpers above don't do.
fn resolve_skills_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let dir = skills_dir(&base);
    fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// List saved skills (name + description only), sorted by name.
#[tauri::command]
pub fn skills_list(app: AppHandle) -> Result<Vec<SkillMeta>, String> {
    list_skills(&resolve_skills_dir(&app)?)
}

/// Load one full skill (including its body) by name.
#[tauri::command]
pub fn skill_load(app: AppHandle, name: String) -> Result<Skill, String> {
    read_skill(&resolve_skills_dir(&app)?, &name)
}

/// Persist a skill, creating or overwriting its file.
#[tauri::command]
pub fn skill_save(app: AppHandle, skill: Skill) -> Result<(), String> {
    write_skill_atomic(&resolve_skills_dir(&app)?, &skill)
}

/// Delete a skill's file.
#[tauri::command]
pub fn skill_delete(app: AppHandle, name: String) -> Result<(), String> {
    delete_skill(&resolve_skills_dir(&app)?, &name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty directory under the OS temp dir, unique per test so
    /// parallel test runs never collide. Removed on drop so a test's fixture
    /// files don't linger.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("srelens-assistant-skills-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sample_skill(name: &str) -> Skill {
        Skill {
            name: name.to_string(),
            description: format!("Description for {name}"),
            body: "Step 1: look at the pod.\n\nStep 2: check events.\n".to_string(),
        }
    }

    #[test]
    fn skills_dir_joins_assistant_skills_under_base() {
        let base = Path::new("/some/config/dir");
        assert_eq!(skills_dir(base), base.join("assistant").join("skills"));
    }

    /// Test-honesty note: `expected` is written out by hand here, not derived
    /// from `skill` via any code under test — so this genuinely checks that
    /// `to_markdown` followed by `parse_skill` preserves name/description/body,
    /// including a body with an internal blank line.
    #[test]
    fn skill_round_trips_through_markdown_front_matter() {
        let skill = Skill {
            name: "crashloop-triage".to_string(),
            description: "Systematic triage for a pod that keeps restarting".to_string(),
            body: "First check the exit code.\n\nThen check the logs.\n".to_string(),
        };

        let markdown = to_markdown(&skill);
        assert_eq!(
            markdown,
            "---\nname: crashloop-triage\ndescription: Systematic triage for a pod that keeps restarting\n---\nFirst check the exit code.\n\nThen check the logs.\n"
        );

        let parsed = parse_skill(&markdown).unwrap();
        assert_eq!(parsed.name, "crashloop-triage");
        assert_eq!(parsed.description, "Systematic triage for a pod that keeps restarting");
        assert_eq!(parsed.body, "First check the exit code.\n\nThen check the logs.\n");
        assert_eq!(parsed, skill);
    }

    #[test]
    fn parse_skill_rejects_a_file_with_no_front_matter() {
        let e = parse_skill("just a body\n").unwrap_err();
        assert!(e.contains("front matter"), "got: {e}");
    }

    #[test]
    fn parse_skill_rejects_unterminated_front_matter() {
        let e = parse_skill("---\nname: x\n").unwrap_err();
        assert!(e.contains("unterminated"), "got: {e}");
    }

    #[test]
    fn parse_skill_rejects_an_empty_name() {
        let e = parse_skill("---\nname: \ndescription: d\n---\nbody\n").unwrap_err();
        assert!(e.contains("name"), "got: {e}");
    }

    #[test]
    fn parse_skill_parses_crlf_line_endings() {
        let text = "---\r\nname: x\r\ndescription: d\r\n---\r\nbody\r\n";
        let f = parse_skill(text).unwrap();
        assert_eq!(f.name, "x");
        assert_eq!(f.description, "d");
        assert_eq!(f.body, "body\n", "body should not retain stray \\r");
    }

    #[test]
    fn save_then_load_returns_an_equal_skill() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path());
        let skill = sample_skill("s1");

        write_skill_atomic(&dir, &skill).unwrap();
        let loaded = read_skill(&dir, "s1").unwrap();

        assert_eq!(loaded, skill);
    }

    #[test]
    fn save_writes_via_a_tmp_file_that_does_not_linger() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path());
        let skill = sample_skill("atomic");

        write_skill_atomic(&dir, &skill).unwrap();

        assert!(skill_path(&dir, "atomic").exists(), "final file must exist");
        assert!(
            !dir.join("atomic.md.tmp").exists(),
            "the .tmp file must be renamed away, not left behind"
        );
    }

    #[test]
    fn list_returns_metas_sorted_by_name_ascending() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path());

        // Inserted out of alphabetical order on purpose: if `list_skills`
        // merely echoed insertion (or filesystem-listing) order, this would
        // not reliably fail, but sorting explicitly makes the guarantee real.
        write_skill_atomic(&dir, &sample_skill("zeta")).unwrap();
        write_skill_atomic(&dir, &sample_skill("alpha")).unwrap();
        write_skill_atomic(&dir, &sample_skill("mid")).unwrap();

        let names: Vec<String> = list_skills(&dir).unwrap().into_iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["alpha", "mid", "zeta"]);
    }

    #[test]
    fn list_metas_carry_the_description_but_not_the_body() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path());
        write_skill_atomic(&dir, &sample_skill("s1")).unwrap();

        let metas = list_skills(&dir).unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].name, "s1");
        assert_eq!(metas[0].description, "Description for s1");
    }

    #[test]
    fn list_of_a_missing_directory_is_empty_not_an_error() {
        let dir = std::env::temp_dir().join(format!("srelens-no-such-skills-dir-{}", uuid::Uuid::new_v4()));
        assert_eq!(list_skills(&dir).unwrap(), Vec::new());
    }

    #[test]
    fn delete_removes_the_file() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path());
        write_skill_atomic(&dir, &sample_skill("gone")).unwrap();
        write_skill_atomic(&dir, &sample_skill("stays")).unwrap();

        delete_skill(&dir, "gone").unwrap();

        assert!(!skill_path(&dir, "gone").exists(), "skill file should be removed");
        let remaining: Vec<String> = list_skills(&dir).unwrap().into_iter().map(|m| m.name).collect();
        assert_eq!(remaining, vec!["stays"]);
    }

    #[test]
    fn delete_of_an_unknown_name_is_not_an_error() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path());
        fs::create_dir_all(&dir).unwrap();
        assert!(delete_skill(&dir, "never-existed").is_ok());
    }

    #[test]
    fn load_of_unknown_name_returns_a_clear_err() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path());
        fs::create_dir_all(&dir).unwrap();

        let err = read_skill(&dir, "does-not-exist").unwrap_err();
        assert!(err.contains("does-not-exist"), "error should name the missing skill, got: {err}");
    }

    /// The load-bearing security test: a name that isn't a bare filename
    /// component must be rejected everywhere a name is accepted, so a
    /// crafted `Skill.name` (or command argument) can never resolve outside
    /// the skills directory.
    #[test]
    fn a_name_with_path_traversal_or_separators_is_rejected_everywhere() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path());
        fs::create_dir_all(&dir).unwrap();

        for bad in ["../evil", "a/b", "a\\b", "..", "../../etc/passwd"] {
            // `..` alone is charset-legal (only dots) but every OTHER case
            // here contains `/` or `\`, which is exactly what must be
            // rejected — `..` is included to confirm it's harmless even
            // though it passes the charset check (it can only ever produce
            // the inert filename `...md`, never traverse a directory).
            if bad == ".." {
                continue;
            }
            let save_err = write_skill_atomic(&dir, &sample_skill(bad)).unwrap_err();
            assert!(save_err.contains("invalid skill name"), "save({bad:?}) got: {save_err}");

            let load_err = read_skill(&dir, bad).unwrap_err();
            assert!(load_err.contains("invalid skill name"), "load({bad:?}) got: {load_err}");

            let delete_err = delete_skill(&dir, bad).unwrap_err();
            assert!(delete_err.contains("invalid skill name"), "delete({bad:?}) got: {delete_err}");
        }
    }
}
