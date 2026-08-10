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
    /// True for a srelens-shipped default skill that has no user file
    /// overriding it — the UI badges these and doesn't offer delete (there's
    /// no file to remove). A user skill (or a user override of a default) is
    /// `false`.
    #[serde(default)]
    pub builtin: bool,
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
///
/// The skills folder is user-hand-editable (it's plain markdown files on
/// disk), so one broken file — bad front-matter, a stray hand-edit mistake —
/// must not take down the whole list. Mirrors how
/// `srelens_mcp::prompts::load_dir`/`resolve` tolerate a bad file: skip it,
/// keep the rest. `skill_load` of that same broken file still returns a
/// clear `Err` — only the LIST is tolerant; loading a specific, presumably
/// intentionally-selected file is not.
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
        let Ok(raw) = fs::read_to_string(&path) else {
            // Unreadable file (permissions, race with a concurrent delete,
            // etc.): skip it, same tolerance as a parse failure below.
            continue;
        };
        let Ok(skill) = parse_skill(&raw) else {
            // Malformed front matter or similar hand-edit mistake: skip this
            // file, but keep listing every other skill that DOES parse.
            continue;
        };
        metas.push(SkillMeta { name: skill.name, description: skill.description, builtin: false });
    }
    metas.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(metas)
}

/// The srelens-shipped default skills — reusable Kubernetes triage playbooks
/// so the Skills panel is useful out of the box, before a user writes any of
/// their own. They're plain instruction text (agent-agnostic); each nudges the
/// assistant to investigate through the srelens MCP tools and end on a likely
/// cause plus a concrete next step. A user skill with the same `name`
/// overrides the default (see [`list_all_skills`]/[`load_skill_or_builtin`]).
pub fn builtin_skills() -> Vec<Skill> {
    fn skill(name: &str, description: &str, body: &str) -> Skill {
        Skill { name: name.to_string(), description: description.to_string(), body: body.to_string() }
    }
    vec![
        skill(
            "crashloop-triage",
            "Triage a pod stuck in CrashLoopBackOff",
            "When a pod keeps restarting (CrashLoopBackOff):\n\n\
             1. Read the pod's recent events for the restart reason.\n\
             2. Fetch the PREVIOUS container's logs (the crashed instance, not the live one) — that's where the real error is.\n\
             3. Check the container's last exit code and reason (e.g. Error vs OOMKilled).\n\
             4. Compare the container's resource requests/limits against its actual usage.\n\
             5. Check recent rollout history for a bad image or config change.\n\n\
             Report the most likely cause and one concrete next step.",
        ),
        skill(
            "pending-pod",
            "Diagnose a pod stuck in Pending / unschedulable",
            "When a pod stays Pending:\n\n\
             1. Read the pod's events — the scheduler explains why (Insufficient cpu/memory, no nodes match, taints, volume binding).\n\
             2. Check node allocatable vs requested capacity across the cluster.\n\
             3. Check the pod's nodeSelector / affinity / tolerations against the nodes' labels and taints.\n\
             4. If it mounts a PVC, check the PVC is Bound and its StorageClass can provision.\n\n\
             Report which constraint blocks scheduling and how to relieve it.",
        ),
        skill(
            "oomkilled",
            "Investigate a container that was OOMKilled",
            "When a container is being OOMKilled:\n\n\
             1. Confirm the OOMKill from the pod's last-state / events.\n\
             2. Compare the container's memory limit against its working-set usage over time.\n\
             3. Check whether the limit is set too low, or usage is genuinely growing (leak).\n\
             4. Look for a recent image/config change that raised memory use.\n\n\
             Report whether to raise the limit or fix the workload, with a suggested value.",
        ),
        skill(
            "node-pressure",
            "Investigate a node under CPU / memory / disk pressure",
            "When a node is under pressure or NotReady:\n\n\
             1. Check the node's conditions (MemoryPressure, DiskPressure, PIDPressure) and events.\n\
             2. Review the node's allocatable vs the sum of pod requests and actual usage.\n\
             3. Find the top resource-consuming pods on that node.\n\
             4. Check for evicted pods as a symptom.\n\n\
             Report the pressure source and whether to rebalance, scale, or cordon/drain.",
        ),
        skill(
            "service-no-endpoints",
            "Debug a Service that has no ready endpoints",
            "When a Service isn't reachable / has no endpoints:\n\n\
             1. Compare the Service's selector against the target pods' labels — a mismatch yields zero endpoints.\n\
             2. Check the backing pods are Ready (failing readiness probes are excluded from endpoints).\n\
             3. Confirm the Service targetPort matches the container's port.\n\
             4. Inspect the EndpointSlices for the Service.\n\n\
             Report why endpoints are empty and the exact fix.",
        ),
        skill(
            "rollout-stuck",
            "Diagnose a Deployment rollout that isn't progressing",
            "When a Deployment rollout is stuck:\n\n\
             1. Check the Deployment's status conditions (Progressing / Available) and the new ReplicaSet.\n\
             2. Inspect the new pods — ImagePullBackOff, CrashLoopBackOff, or failing readiness all stall a rollout.\n\
             3. Verify replica counts: desired vs updated vs available.\n\
             4. Check events for quota, scheduling, or probe failures.\n\n\
             Report what blocks the rollout and whether to fix forward or roll back.",
        ),
    ]
}

/// Every skill the picker should show: the defaults from [`builtin_skills`]
/// merged with the user's own, where a user file with the same name overrides
/// the default (and is marked `builtin: false`). The `BTreeMap` keeps the
/// result sorted by name. Kept separate from [`list_skills`] so that pure
/// helper stays "user files only" for its existing tests.
fn list_all_skills(dir: &Path) -> Result<Vec<SkillMeta>, String> {
    use std::collections::BTreeMap;
    let mut by_name: BTreeMap<String, SkillMeta> = BTreeMap::new();
    for b in builtin_skills() {
        by_name.insert(
            b.name.clone(),
            SkillMeta { name: b.name, description: b.description, builtin: true },
        );
    }
    for m in list_skills(dir)? {
        by_name.insert(m.name.clone(), SkillMeta { name: m.name, description: m.description, builtin: false });
    }
    Ok(by_name.into_values().collect())
}

/// Load a skill by name: the user's own file if it exists, otherwise the
/// shipped default of that name, otherwise a clear `Err`. A user file that
/// exists but is malformed still surfaces its parse error (it was chosen on
/// purpose) rather than silently falling back to a default.
fn load_skill_or_builtin(dir: &Path, name: &str) -> Result<Skill, String> {
    validate_name(name)?;
    if skill_path(dir, name).exists() {
        return read_skill(dir, name);
    }
    builtin_skills()
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("no skill found named {name:?}"))
}

/// Resolve `<app config dir>/assistant/skills`, creating it if needed — the
/// one bit of app-specific wiring the pure helpers above don't do.
fn resolve_skills_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let dir = skills_dir(&base);
    fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// List skills (name + description only), sorted by name — the shipped
/// defaults plus the user's own, user files overriding a default of the same
/// name.
#[tauri::command]
pub fn skills_list(app: AppHandle) -> Result<Vec<SkillMeta>, String> {
    list_all_skills(&resolve_skills_dir(&app)?)
}

/// Load one full skill (including its body) by name — a user file if present,
/// else the shipped default of that name.
#[tauri::command]
pub fn skill_load(app: AppHandle, name: String) -> Result<Skill, String> {
    load_skill_or_builtin(&resolve_skills_dir(&app)?, &name)
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

    /// The skills folder is user-hand-editable (plain markdown files on
    /// disk), so one broken file must not take the whole list down —
    /// `list_skills` skips it and returns everything that DOES parse, the
    /// same tolerance `srelens_mcp::prompts::load_dir` gives a bad prompt
    /// file. `skill_load` of that same broken file is deliberately NOT
    /// covered by this tolerance — see the next test.
    #[test]
    fn list_skips_a_malformed_file_and_keeps_the_valid_ones() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path());
        write_skill_atomic(&dir, &sample_skill("good")).unwrap();
        fs::write(dir.join("broken.md"), "not front matter at all\n").unwrap();

        let result = list_skills(&dir);
        assert!(result.is_ok(), "one broken file must not fail the whole list, got {result:?}");
        let names: Vec<String> = result.unwrap().into_iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["good"], "the malformed file must be skipped, not surfaced or aborting the list");
    }

    /// The list tolerates a bad file (previous test); loading one specific,
    /// presumably intentionally-selected file by name does not — that path
    /// is unchanged and must still surface a clear `Err`.
    #[test]
    fn load_of_a_malformed_file_still_returns_a_clear_err() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("broken.md"), "not front matter at all\n").unwrap();

        let err = read_skill(&dir, "broken").unwrap_err();
        assert!(err.contains("front matter"), "got: {err}");
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

        for bad in ["../evil", "a/b", "a\\b", "../../etc/passwd"] {
            let save_err = write_skill_atomic(&dir, &sample_skill(bad)).unwrap_err();
            assert!(save_err.contains("invalid skill name"), "save({bad:?}) got: {save_err}");

            let load_err = read_skill(&dir, bad).unwrap_err();
            assert!(load_err.contains("invalid skill name"), "load({bad:?}) got: {load_err}");

            let delete_err = delete_skill(&dir, bad).unwrap_err();
            assert!(delete_err.contains("invalid skill name"), "delete({bad:?}) got: {delete_err}");
        }
    }

    /// `..` alone is charset-legal under `validate_name` (it's made of only
    /// dots, and name validation itself is unchanged — the `.md` suffix is
    /// what neutralizes it), so it must NOT be rejected the way `../evil` is.
    /// What makes it safe is `skill_path` always appending `.md` to the whole
    /// name as one atomic step before it ever becomes a path component: `..`
    /// can only ever produce the literal, inert filename `...md`, never the
    /// parent-directory special path. This is a machine-checked proof of
    /// that, not prose: the resolved path stays inside `dir`, its filename is
    /// exactly the literal string `...md`, and after a save the directory's
    /// only entry is that same literal file — nothing escaped upward.
    #[test]
    fn a_double_dot_name_resolves_to_a_literal_filename_inside_the_dir_not_a_parent_escape() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path());
        fs::create_dir_all(&dir).unwrap();

        let path = skill_path(&dir, "..");
        assert!(path.starts_with(&dir), "skill_path(\"..\") must stay inside dir, got {path:?}");
        assert_eq!(
            path.file_name(),
            Some(std::ffi::OsStr::new("...md")),
            "must resolve to the literal filename `...md`, not a parent-dir escape, got {path:?}"
        );

        let skill = sample_skill("..");
        write_skill_atomic(&dir, &skill).unwrap();
        assert_eq!(read_skill(&dir, "..").unwrap(), skill, "save/load of `..` round-trips like any other name");

        let entries: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            entries,
            vec!["...md".to_string()],
            "the only file written under dir must be the literal `...md`, got {entries:?}"
        );
    }

    #[test]
    fn every_builtin_skill_is_valid_and_round_trips_through_the_on_disk_format() {
        let builtins = builtin_skills();
        assert!(!builtins.is_empty(), "srelens should ship some default skills");
        for s in builtins {
            validate_name(&s.name).unwrap_or_else(|e| panic!("builtin {:?} has an invalid name: {e}", s.name));
            assert!(!s.description.trim().is_empty(), "builtin {:?} needs a description", s.name);
            assert!(!s.body.trim().is_empty(), "builtin {:?} needs a body", s.name);
            assert_eq!(parse_skill(&to_markdown(&s)).unwrap(), s, "builtin {:?} must round-trip", s.name);
        }
    }

    #[test]
    fn list_all_shows_the_builtins_marked_builtin_when_there_are_no_user_skills() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path()); // never created — the user has no skills of their own

        let metas = list_all_skills(&dir).unwrap();
        assert_eq!(metas.len(), builtin_skills().len(), "every default should be listed");
        assert!(metas.iter().all(|m| m.builtin), "with no user files, every entry is a default");
        assert!(metas.iter().any(|m| m.name == "crashloop-triage"), "a known default should be present");
        // sorted by name (BTreeMap ordering)
        let mut sorted = metas.clone();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(metas, sorted);
    }

    #[test]
    fn list_all_shows_a_user_skill_alongside_the_builtins_not_marked_builtin() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path());
        write_skill_atomic(&dir, &sample_skill("my-own")).unwrap();

        let metas = list_all_skills(&dir).unwrap();
        assert_eq!(metas.len(), builtin_skills().len() + 1);
        let mine = metas.iter().find(|m| m.name == "my-own").expect("the user skill should be listed");
        assert!(!mine.builtin, "a user-authored skill is not a default");
    }

    #[test]
    fn a_user_file_overrides_a_builtin_of_the_same_name_without_duplicating_it() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path());
        let override_skill = Skill {
            name: "crashloop-triage".to_string(),
            description: "MY custom crashloop steps".to_string(),
            body: "do it my way\n".to_string(),
        };
        write_skill_atomic(&dir, &override_skill).unwrap();

        let metas = list_all_skills(&dir).unwrap();
        assert_eq!(metas.len(), builtin_skills().len(), "an override replaces the default, it doesn't add a row");
        let hits: Vec<&SkillMeta> = metas.iter().filter(|m| m.name == "crashloop-triage").collect();
        assert_eq!(hits.len(), 1, "the name must appear exactly once");
        assert_eq!(hits[0].description, "MY custom crashloop steps", "the user's description wins");
        assert!(!hits[0].builtin, "an overridden default is now a user skill");

        // and load returns the user's body, not the shipped default's
        assert_eq!(load_skill_or_builtin(&dir, "crashloop-triage").unwrap(), override_skill);
    }

    #[test]
    fn load_falls_back_to_the_shipped_default_when_there_is_no_user_file() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path()); // no user files

        let loaded = load_skill_or_builtin(&dir, "pending-pod").unwrap();
        let default = builtin_skills().into_iter().find(|s| s.name == "pending-pod").unwrap();
        assert_eq!(loaded, default, "with no user file, loading returns the shipped default verbatim");
    }

    #[test]
    fn load_of_an_unknown_name_with_no_matching_default_errs() {
        let tmp = TempDir::new();
        let dir = skills_dir(tmp.path());
        let err = load_skill_or_builtin(&dir, "no-such-skill").unwrap_err();
        assert!(err.contains("no-such-skill"), "error should name the missing skill, got: {err}");
    }
}
