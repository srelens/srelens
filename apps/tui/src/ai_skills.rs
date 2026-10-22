//! AI SRE Playbooks & Slash Commands Registry for srectl.
//!
//! Provides battle-tested Kubernetes diagnostic playbooks (matching GUI assistant_skills.rs)
//! with interactive slash-command autocomplete, zero-argument discovery fallbacks,
//! and contextual resource prompts. Also manages disk-backed user custom skills
//! stored in `~/.config/srelens/assistant/skills/*.md`.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::path::{Path, PathBuf};

/// A full skill: its instructions plus the metadata used to pick it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// Picker metadata only — everything but `body`, matching GUI assistant_skills.rs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub overrides_builtin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDef {
    pub name: Cow<'static, str>,
    pub command: Cow<'static, str>,
    pub aliases: &'static [&'static str],
    pub description: Cow<'static, str>,
    pub target_kind: Option<Cow<'static, str>>,
    pub instructions: Cow<'static, str>,
    pub is_utility: bool,
    pub is_user_defined: bool,
}

impl SkillDef {
    pub fn target_placeholder(&self) -> &str {
        self.target_kind.as_deref().unwrap_or("")
    }
}

const fn builtin_playbook(
    name: &'static str,
    command: &'static str,
    aliases: &'static [&'static str],
    description: &'static str,
    target_kind: Option<&'static str>,
    instructions: &'static str,
    is_utility: bool,
) -> SkillDef {
    let target = match target_kind {
        Some(t) => Some(Cow::Borrowed(t)),
        None => None,
    };
    SkillDef {
        name: Cow::Borrowed(name),
        command: Cow::Borrowed(command),
        aliases,
        description: Cow::Borrowed(description),
        target_kind: target,
        instructions: Cow::Borrowed(instructions),
        is_utility,
        is_user_defined: false,
    }
}

pub const BUILTIN_PLAYBOOKS: &[SkillDef] = &[
    builtin_playbook(
        "crashloop-triage",
        "crashloop",
        &["crash", "crashloop-triage"],
        "Triage pod stuck in CrashLoopBackOff (previous logs, exit code, events)",
        Some("Pod"),
        "When a pod keeps restarting (CrashLoopBackOff):\n\
1. Read the pod's recent events for the restart reason.\n\
2. Fetch the PREVIOUS container's logs (the crashed instance, not the live one) — that's where the real error is.\n\
3. Check the container's last exit code and termination reason (e.g. Error vs OOMKilled).\n\
4. Compare the container's resource requests/limits against its actual usage.\n\
5. Check recent rollout history for a bad image or config change.\n\n\
Report the most likely root cause and one concrete next remediation step.",
        false,
    ),
    builtin_playbook(
        "pending-pod",
        "pending",
        &["pend", "pending-pod", "unschedulable"],
        "Diagnose pod stuck in Pending or unschedulable (capacity, taints, PVCs)",
        Some("Pod"),
        "When a pod stays in Pending status:\n\
1. Read the pod's events — the scheduler explains why (Insufficient cpu/memory, no nodes match, taints, volume binding).\n\
2. Check node allocatable vs requested capacity across the cluster.\n\
3. Check the pod's nodeSelector, affinity, and tolerations against the nodes' labels and taints.\n\
4. If it mounts a PVC, check the PVC is Bound and its StorageClass can provision volume.\n\n\
Report which constraint blocks scheduling and how to relieve it.",
        false,
    ),
    builtin_playbook(
        "oomkilled",
        "oom",
        &["oomkilled", "memory-leak"],
        "Investigate container OOMKilled events and memory limit sizing",
        Some("Pod"),
        "When a container is being OOMKilled:\n\
1. Confirm the OOMKill from the pod's last-state and events.\n\
2. Compare the container's memory limit against its working-set usage over time.\n\
3. Check whether the limit is set too low, or usage is genuinely growing (memory leak).\n\
4. Look for a recent image/config change that raised memory use.\n\n\
Report whether to raise the limit or fix the workload, with a suggested memory value.",
        false,
    ),
    builtin_playbook(
        "node-pressure",
        "nodepressure",
        &["node", "node-pressure", "drain-check"],
        "Investigate node under CPU / Memory / Disk pressure or NotReady",
        Some("Node"),
        "When a node is under pressure or NotReady:\n\
1. Check the node's conditions (MemoryPressure, DiskPressure, PIDPressure) and events.\n\
2. Review the node's allocatable capacity vs the sum of pod requests and actual usage.\n\
3. Find the top resource-consuming pods on that node.\n\
4. Check for evicted pods as a symptom.\n\n\
Report the pressure source and whether to rebalance, scale, or cordon/drain the node.",
        false,
    ),
    builtin_playbook(
        "service-no-endpoints",
        "endpoints",
        &["service-no-endpoints", "no-endpoints", "svc"],
        "Debug Service that has no ready endpoints or connection refused",
        Some("Service"),
        "When a Service is unreachable or has zero endpoints:\n\
1. Compare the Service's selector against the target pods' labels — a mismatch yields zero endpoints.\n\
2. Check the backing pods are Ready (failing readiness probes are excluded from endpoints).\n\
3. Confirm the Service targetPort matches the container's listening port.\n\
4. Inspect the EndpointSlices for the Service.\n\n\
Report why endpoints are empty and provide the exact fix.",
        false,
    ),
    builtin_playbook(
        "rollout-stuck",
        "rollout",
        &["rollout-stuck", "stuck-deploy"],
        "Diagnose Deployment rollout that is stalled or failing progression",
        Some("Deployment"),
        "When a Deployment rollout is stuck:\n\
1. Check the Deployment's status conditions (Progressing, Available) and the active ReplicaSets.\n\
2. Inspect the new pods — ImagePullBackOff, CrashLoopBackOff, or failing readiness all stall a rollout.\n\
3. Verify replica counts: desired vs updated vs available.\n\
4. Check events for quota limits, scheduling blockers, or probe failures.\n\n\
Report what blocks the rollout and whether to fix forward or roll back.",
        false,
    ),
    builtin_playbook(
        "cluster-summary",
        "summarise",
        &["summary", "cluster-summary", "health"],
        "Generate an executive SRE health and incident briefing for the cluster",
        None,
        "Perform an executive SRE health assessment of the active cluster:\n\
1. Query cluster-wide pod counts and identify all pods in non-Running phases (CrashLoop, Pending, Error, Evicted).\n\
2. Inspect nodes for any NotReady states or pressure conditions (Memory/Disk pressure).\n\
3. Review cluster warning events from the last 30 minutes to identify emerging incident patterns.\n\
4. Check for stalled Deployments or StatefulSets with unmet replica requirements.\n\n\
Synthesize the findings into an executive briefing with high-priority risks and suggested remediation actions.",
        false,
    ),
    builtin_playbook(
        "argo-triage",
        "argo",
        &["argo-triage", "argocd", "progressing"],
        "Diagnose ArgoCD application stuck in Progressing, Degraded, or OutOfSync",
        Some("Application"),
        "When an ArgoCD application is stuck in Progressing, Degraded, or failing to sync:\n\
1. Check the Application's sync status, health status, and health message.\n\
2. Inspect the operation phase and error message of the last sync.\n\
3. Identify which specific child resources are Degraded, Progressing, or Missing.\n\
4. Check for drift between live cluster state and the target Git revision.\n\
5. Inspect SyncHook jobs or PreSync/PostSync failures.\n\
6. Check the target namespace and cluster destination connectivity.\n\n\
Report the root cause why the application is stuck or degraded and provide concrete remediation steps (sync flags, manifest fix, or controller action).",
        false,
    ),
    builtin_playbook(
        "clear-chat",
        "clear",
        &["cls", "reset"],
        "Clear conversation history and reset assistant state",
        None,
        "",
        true,
    ),
    builtin_playbook(
        "save-chat",
        "save",
        &["export"],
        "Export current conversation to a markdown file",
        None,
        "",
        true,
    ),
    builtin_playbook(
        "ai-settings",
        "settings",
        &["config", "model"],
        "Open AI model, API key, and provider configuration",
        None,
        "",
        true,
    ),
    builtin_playbook(
        "caveman",
        "caveman",
        &["cave", "terse"],
        "Set ultra-compressed caveman mode to cut token usage ~75% (lite|full|ultra|wenyan-*|off)",
        Some("lite|full|ultra|wenyan|off"),
        "",
        true,
    ),
];

// ============================================================================
// Disk-backed User Skills Management (GUI Parity)
// ============================================================================

/// Reject any name that isn't `^[A-Za-z0-9._-]+$` — in particular, one
/// containing `/` or `\`, so a skill can never resolve outside `skills_dir`
/// via a crafted name like `../evil` or `a/b`.
pub fn validate_name(name: &str) -> Result<(), String> {
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

/// Parses a skill from markdown text. Supports YAML frontmatter:
/// ```text
/// ---
/// name: my-skill
/// description: Quick description
/// ---
/// <body>
/// ```
/// If frontmatter is missing or fields are empty, falls back to `fallback_name` and `fallback_desc`.
pub fn parse_skill_content(
    text: &str,
    fallback_name: Option<&str>,
    fallback_desc: Option<&str>,
) -> Result<Skill, String> {
    let normalized = text.replace("\r\n", "\n");

    let (mut name, mut description, body) = if let Some(rest) = normalized.strip_prefix("---\n") {
        if let Some((header, body_content)) = rest.split_once("\n---\n") {
            let mut parsed_name = None;
            let mut parsed_desc = None;
            for line in header.lines() {
                if let Some(val) = line.strip_prefix("name:") {
                    parsed_name = Some(val.trim().to_string());
                } else if let Some(val) = line.strip_prefix("description:") {
                    parsed_desc = Some(val.trim().to_string());
                }
            }
            (parsed_name, parsed_desc, body_content.to_string())
        } else {
            (None, None, normalized.clone())
        }
    } else {
        (None, None, normalized.clone())
    };

    if name.as_deref().unwrap_or("").is_empty() {
        if let Some(fb) = fallback_name {
            name = Some(fb.to_string());
        }
    }

    if description.as_deref().unwrap_or("").is_empty() {
        if let Some(fb) = fallback_desc {
            description = Some(fb.to_string());
        }
    }

    let final_name = name.ok_or_else(|| {
        "skill name is required (specify frontmatter `name:` or pass [NAME] on the command line)"
            .to_string()
    })?;

    validate_name(&final_name)?;

    Ok(Skill {
        name: final_name,
        description: description.unwrap_or_default(),
        body,
    })
}

/// Strict parser for files already on disk: requires valid frontmatter header.
pub fn parse_skill(text: &str) -> Result<Skill, String> {
    let normalized = text.replace("\r\n", "\n");
    let rest = normalized
        .strip_prefix("---\n")
        .ok_or("missing front matter: the file must start with `---`")?;
    let (header, body) = rest
        .split_once("\n---\n")
        .ok_or("unterminated front matter: no closing `---` line")?;

    let mut name = None;
    let mut description = None;
    for line in header.lines() {
        if let Some(val) = line.strip_prefix("name:") {
            name = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("description:") {
            description = Some(val.trim().to_string());
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

/// Renders a skill back to its on-disk markdown format with frontmatter.
pub fn to_markdown(skill: &Skill) -> String {
    format!(
        "---\nname: {}\ndescription: {}\n---\n{}",
        skill.name, skill.description, skill.body
    )
}

pub fn skill_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.md"))
}

pub fn skill_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        })
        .collect();
    files.sort();
    files
}

pub fn user_skill_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let named = skill_path(dir, name);
    if named.exists() {
        if let Ok(raw) = std::fs::read_to_string(&named) {
            if let Ok(skill) = parse_skill(&raw) {
                if skill.name == name {
                    return Some(named);
                }
            }
        }
    }
    skill_files(dir).into_iter().find(|path| {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| parse_skill(&raw).ok())
            .is_some_and(|skill| skill.name == name)
    })
}

pub fn write_skill_atomic(dir: &Path, skill: &Skill) -> Result<PathBuf, String> {
    validate_name(&skill.name)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let path = skill_path(dir, &skill.name);
    let tmp = dir.join(format!("{}.md.tmp", skill.name));
    std::fs::write(&tmp, to_markdown(skill))
        .map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("could not finalize {}: {e}", path.display()))?;
    Ok(path)
}

pub fn read_skill(dir: &Path, name: &str) -> Result<Skill, String> {
    validate_name(name)?;
    let path =
        user_skill_file(dir, name).ok_or_else(|| format!("no skill found named {name:?}"))?;
    let raw =
        std::fs::read_to_string(&path).map_err(|_| format!("no skill found named {name:?}"))?;
    parse_skill(&raw)
}

pub fn delete_skill(dir: &Path, name: &str) -> Result<bool, String> {
    validate_name(name)?;
    let Some(path) = user_skill_file(dir, name) else {
        return Ok(false);
    };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("could not delete {}: {e}", path.display())),
    }
}

pub fn list_skills(dir: &Path) -> Result<Vec<SkillMeta>, String> {
    match std::fs::read_dir(dir) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("could not read {}: {e}", dir.display())),
    }

    let mut metas = Vec::new();
    for path in skill_files(dir) {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(skill) = parse_skill(&raw) else {
            continue;
        };
        metas.push(SkillMeta {
            name: skill.name,
            description: skill.description,
            builtin: false,
            path: Some(path.display().to_string()),
            overrides_builtin: false,
        });
    }
    metas.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(metas)
}

/// Shipped default diagnostic skills (matching GUI assistant_skills.rs).
pub fn builtin_skills() -> Vec<Skill> {
    BUILTIN_PLAYBOOKS
        .iter()
        .filter(|s| !s.is_utility)
        .map(|s| Skill {
            name: s.name.to_string(),
            description: s.description.to_string(),
            body: s.instructions.to_string(),
        })
        .collect()
}

/// List all skills: default built-ins merged with user files.
pub fn list_all_skills(dir: &Path) -> Result<Vec<SkillMeta>, String> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut by_name: BTreeMap<String, SkillMeta> = BTreeMap::new();
    for b in builtin_skills() {
        by_name.insert(
            b.name.clone(),
            SkillMeta {
                name: b.name,
                description: b.description,
                builtin: true,
                path: None,
                overrides_builtin: false,
            },
        );
    }
    let shipped: BTreeSet<String> = by_name.keys().cloned().collect();
    let mut from_user: BTreeSet<String> = BTreeSet::new();
    for m in list_skills(dir)? {
        if !from_user.insert(m.name.clone()) {
            let named = skill_path(dir, &m.name).display().to_string();
            if m.path.as_deref() != Some(named.as_str()) {
                continue;
            }
        }
        let overrides_builtin = shipped.contains(&m.name);
        by_name.insert(
            m.name.clone(),
            SkillMeta {
                overrides_builtin,
                ..m
            },
        );
    }
    Ok(by_name.into_values().collect())
}

pub fn load_skill_or_builtin(dir: &Path, name: &str) -> Result<Skill, String> {
    validate_name(name)?;
    if user_skill_file(dir, name).is_some() {
        return read_skill(dir, name);
    }
    builtin_skills()
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("no skill found named {name:?}"))
}

pub fn read_user_skills(dir: &Path) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")))
        .collect();
    paths.sort();

    let mut by_name: std::collections::BTreeMap<String, Skill> = std::collections::BTreeMap::new();
    for path in paths {
        let is_canonical_file = path.file_stem().and_then(|s| s.to_str());
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(skill) = parse_skill(&raw) {
                // If a skill with this name was already encountered, the canonical file (<name>.md) wins
                if by_name.contains_key(&skill.name) {
                    if is_canonical_file == Some(&skill.name) {
                        by_name.insert(skill.name.clone(), skill);
                    }
                } else {
                    by_name.insert(skill.name.clone(), skill);
                }
            }
        }
    }
    by_name.into_values().collect()
}

/// Scans `~/.config/srelens/assistant/skills/*.md` for user-defined skills.
pub fn load_user_skills_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SRELENS_SKILLS_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    dirs::config_dir()
        .map(|d| d.join("srelens").join("assistant").join("skills"))
        .unwrap_or_else(|| PathBuf::from(".srelens/skills"))
}

// ============================================================================
// Remote Ingestion & GitHub Translation
// ============================================================================

/// Translates GitHub web blob URLs into direct raw URLs.
/// Example: `https://github.com/owner/repo/blob/main/skills/triage.md`
/// -> `https://raw.githubusercontent.com/owner/repo/main/skills/triage.md`
pub fn resolve_fetch_url(url: &str) -> String {
    let trimmed = url.trim();
    if let Some(rest) = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
    {
        let (path_part, query_part) = match rest.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (rest, None),
        };
        let parts: Vec<&str> = path_part.split('/').collect();
        // parts: [owner, repo, "blob", git_ref, ...rest_path]
        if parts.len() >= 5 && parts[2] == "blob" {
            let owner = parts[0];
            let repo = parts[1];
            let git_ref = parts[3];
            let subpath = parts[4..].join("/");
            let mut resolved =
                format!("https://raw.githubusercontent.com/{owner}/{repo}/{git_ref}/{subpath}");
            if let Some(q) = query_part {
                resolved.push('?');
                resolved.push_str(q);
            }
            return resolved;
        }
    }
    trimmed.to_string()
}

/// Derives a fallback skill name from a URL's filename stem.
pub fn fallback_name_from_url(url: &str) -> Option<String> {
    let path = url.split('?').next().unwrap_or(url);
    let path = path.split('#').next().unwrap_or(path);
    let last_segment = path.trim_end_matches('/').rsplit('/').next()?;
    let stem = last_segment.strip_suffix(".md").unwrap_or(last_segment);
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_string())
    }
}

/// Returns true only if `url_str` is an HTTPS URL targeting an approved GitHub host,
/// ensuring secret GitHub tokens are never leaked to arbitrary domains or plaintext HTTP.
pub fn is_trusted_github_url(url_str: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url_str) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    match parsed.host_str() {
        Some(host) => {
            let host = host.to_ascii_lowercase();
            host == "github.com"
                || host == "api.github.com"
                || host == "raw.githubusercontent.com"
                || host == "gist.githubusercontent.com"
                || host.ends_with(".github.com")
                || host.ends_with(".githubusercontent.com")
        }
        None => false,
    }
}

/// Fetches skill markdown from an HTTP/HTTPS URL, translating GitHub blob URLs
/// and passing `GITHUB_TOKEN` / `GH_TOKEN` authentication only to trusted GitHub HTTPS hosts.
pub fn fetch_skill_url(url: &str) -> Result<String, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let resolved_url = resolve_fetch_url(url);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(concat!("srectl/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("could not initialize HTTP client: {e}"))?;

    let mut req = client.get(&resolved_url);

    if is_trusted_github_url(&resolved_url) {
        if let Ok(token) = std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GH_TOKEN")) {
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                req = req.header("Authorization", format!("Bearer {trimmed}"));
            }
        }
    }

    let res = req
        .send()
        .map_err(|e| format!("failed to fetch from {resolved_url}: {e}"))?;
    if !res.status().is_success() {
        return Err(format!(
            "failed to fetch skill from {resolved_url}: HTTP {}",
            res.status()
        ));
    }

    res.text()
        .map_err(|e| format!("failed to read response body from {resolved_url}: {e}"))
}

// ============================================================================
// Interactive TUI Slash Command Resolution & Dynamic Autocomplete
// ============================================================================

/// Checks if `name` collides with a reserved utility or built-in command/alias.
pub fn is_reserved_command_or_alias(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower == "clear" || lower == "help" {
        return true;
    }
    for pb in BUILTIN_PLAYBOOKS {
        if pb.name.eq_ignore_ascii_case(&lower) {
            return pb.is_utility;
        }
        if pb.command.eq_ignore_ascii_case(&lower)
            || pb.aliases.iter().any(|a| a.eq_ignore_ascii_case(&lower))
        {
            return true;
        }
    }
    false
}

/// Validates that a custom skill name matches naming rules and does not collide
/// with built-in commands or aliases (while allowing exact overrides of built-in playbooks).
pub fn validate_custom_skill_name(name: &str) -> Result<(), String> {
    validate_name(name)?;
    let lower = name.to_ascii_lowercase();
    if lower == "clear" || lower == "help" {
        return Err(format!(
            "cannot use reserved command name '{name}' as a skill name"
        ));
    }
    for pb in BUILTIN_PLAYBOOKS {
        if pb.name.eq_ignore_ascii_case(name) {
            if pb.is_utility {
                return Err(format!(
                    "cannot use reserved utility name '{name}' as a skill name"
                ));
            }
            return Ok(());
        }
        if pb.command.eq_ignore_ascii_case(name) {
            if pb.is_utility {
                return Err(format!(
                    "skill name '{name}' collides with reserved utility command '/{}'",
                    pb.command
                ));
            }
            return Err(format!(
                "skill name '{name}' collides with built-in command '/{}'. Use '{}' to override this playbook.",
                pb.command, pb.name
            ));
        }
        for alias in pb.aliases {
            if alias.eq_ignore_ascii_case(name) {
                if pb.is_utility {
                    return Err(format!(
                        "skill name '{name}' collides with reserved utility alias '/{}'",
                        alias
                    ));
                }
                return Err(format!(
                    "skill name '{name}' collides with built-in alias '/{}'. Use '{}' to override this playbook.",
                    alias, pb.name
                ));
            }
        }
    }
    Ok(())
}

/// Loads all available playbooks, merging built-in playbooks with user-defined
/// skills from `dir`. User playbooks override built-in playbooks of the same name.
pub fn load_all_playbooks_from(dir: &Path) -> Vec<SkillDef> {
    let mut playbooks: Vec<SkillDef> = BUILTIN_PLAYBOOKS.to_vec();

    let user_skills = read_user_skills(dir);
    for skill in user_skills {
        if let Some(existing) = playbooks
            .iter_mut()
            .find(|p| p.name == skill.name && !p.is_utility)
        {
            existing.description = Cow::Owned(skill.description);
            existing.instructions = Cow::Owned(skill.body);
            existing.is_user_defined = true;
        } else if !is_reserved_command_or_alias(&skill.name) {
            playbooks.push(SkillDef {
                name: Cow::Owned(skill.name.clone()),
                command: Cow::Owned(skill.name.clone()),
                aliases: &[],
                description: Cow::Owned(skill.description),
                target_kind: None,
                instructions: Cow::Owned(skill.body),
                is_utility: false,
                is_user_defined: true,
            });
        }
    }

    playbooks
}

pub fn load_all_playbooks() -> Vec<SkillDef> {
    load_all_playbooks_from(&load_user_skills_dir())
}

/// Finds all slash commands matching the user's current input, dynamically
/// discovering both built-in playbooks and custom user skills.
pub fn match_slash_commands(input: &str) -> Vec<SkillDef> {
    match_slash_commands_from(input, &load_user_skills_dir())
}

pub fn match_slash_commands_from(input: &str, dir: &Path) -> Vec<SkillDef> {
    let clean = input.trim_start_matches('/');
    let query = clean.split_whitespace().next().unwrap_or("").to_lowercase();

    load_all_playbooks_from(dir)
        .into_iter()
        .filter(|s| {
            if query.is_empty() {
                return true;
            }
            s.command.to_lowercase().starts_with(&query)
                || s.aliases
                    .iter()
                    .any(|a| a.to_lowercase().starts_with(&query))
                || s.name.to_lowercase().starts_with(&query)
        })
        .collect()
}

/// Expands a slash command into an enriched, context-aware SRE prompt for the agent.
pub fn expand_slash_command(
    command_name: &str,
    target_arg: Option<&str>,
    active_context: &str,
    active_namespace: &str,
) -> Option<String> {
    expand_slash_command_from(
        command_name,
        target_arg,
        active_context,
        active_namespace,
        &load_user_skills_dir(),
    )
}

pub fn expand_slash_command_from(
    command_name: &str,
    target_arg: Option<&str>,
    active_context: &str,
    active_namespace: &str,
    dir: &Path,
) -> Option<String> {
    let playbooks = load_all_playbooks_from(dir);
    let skill = playbooks.iter().find(|s| {
        s.command == command_name
            || s.aliases.iter().any(|a| *a == command_name)
            || s.name == command_name
    })?;

    if skill.is_utility {
        return None;
    }

    let ns_desc = if active_namespace.is_empty() {
        "all namespaces".to_string()
    } else {
        format!("namespace '{}'", active_namespace)
    };

    let target_statement = match target_arg {
        Some(target) => {
            let placeholder = skill.target_placeholder();
            let kind = if placeholder.is_empty() { "resource" } else { placeholder };
            format!("Focus on {} '{}' in {}.", kind, target, ns_desc)
        }
        None => match skill.target_kind.as_deref() {
            Some("Pod") => format!("Scan {} for any pods experiencing this issue and triage them.", ns_desc),
            Some("Node") => "Scan the cluster for any nodes experiencing pressure or unreadiness and triage them.".to_string(),
            Some("Deployment") => format!("Scan {} for any stalled or degraded rollouts and triage them.", ns_desc),
            Some("Service") => format!("Scan {} for any services with zero endpoints or connection issues and triage them.", ns_desc),
            Some("Application") => format!("Scan {} for any ArgoCD applications stuck in Progressing, Degraded, or OutOfSync and triage them.", ns_desc),
            _ => format!("Investigate cluster '{}' in {}.", active_context, ns_desc),
        },
    };

    let prompt = format!(
        "Playbook: {}\nContext: Cluster '{}', {}\n{}\n\nGuidelines:\n{}",
        skill.description, active_context, ns_desc, target_statement, skill.instructions
    );

    Some(prompt)
}

// ============================================================================
// Caveman Mode
// ============================================================================

/// Supported caveman compression levels matching SKILL.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CavemanLevel {
    Lite,
    Full,
    Ultra,
    WenyanLite,
    WenyanFull,
    WenyanUltra,
}

impl CavemanLevel {
    pub fn display_name(&self) -> &'static str {
        match self {
            CavemanLevel::Lite => "lite",
            CavemanLevel::Full => "full",
            CavemanLevel::Ultra => "ultra",
            CavemanLevel::WenyanLite => "wenyan-lite",
            CavemanLevel::WenyanFull => "wenyan-full",
            CavemanLevel::WenyanUltra => "wenyan-ultra",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let trimmed = s.trim().to_lowercase();
        match trimmed.as_str() {
            "lite" => Some(CavemanLevel::Lite),
            "full" | "" => Some(CavemanLevel::Full),
            "ultra" => Some(CavemanLevel::Ultra),
            "wenyan-lite" | "wenyan_lite" | "wenyanlite" => Some(CavemanLevel::WenyanLite),
            "wenyan-full" | "wenyan_full" | "wenyanfull" | "wenyan" => {
                Some(CavemanLevel::WenyanFull)
            }
            "wenyan-ultra" | "wenyan_ultra" | "wenyanultra" => Some(CavemanLevel::WenyanUltra),
            _ => None,
        }
    }

    pub fn prompt_instructions(&self) -> &'static str {
        match self {
            CavemanLevel::Lite => "\
[INSTRUCTION: CAVEMAN MODE (LITE)]
Respond terse and direct. Cut token usage drastically.
Rules:
- No filler (just/really/basically/actually/simply) or pleasantries (sure/certainly/happy to/hello) or hedging.
- Keep articles and complete grammatical sentences, but keep them concise and strictly professional.
- Technical terms, code blocks, yaml, and errors must remain exact.
- Drop caveman for security warnings or destructive confirmations; resume after.
- Stop immediately once the answer is delivered.",

            CavemanLevel::Full => "\
[INSTRUCTION: CAVEMAN MODE (FULL)]
Respond terse like smart caveman. Cut token usage ~75%. All technical substance stays, only fluff dies.
Rules:
- Drop articles (a/an/the), filler (just/really/basically/actually/simply), pleasantries, and hedging.
- Fragments OK. Short synonyms (e.g. fix, not 'implement a solution for').
- Technical terms, error messages, yaml, and code blocks MUST remain exact and uncompressed.
- Pattern: [thing] [action] [reason]. [next step].
- Example: 'Bug in auth middleware. Token expiry check use < not <=. Fix: ...'
- Drop caveman for security warnings or destructive confirmations; resume after.
- Stop immediately once the answer is delivered.",

            CavemanLevel::Ultra => "\
[INSTRUCTION: CAVEMAN MODE (ULTRA)]
Ultra-compressed caveman. Extreme token reduction.
Rules:
- Drop all articles, conjunctions, pleasantries, filler, and hedging.
- Abbreviate common terms (DB/auth/config/req/res/fn/impl/svc/ns/dep).
- Use arrows for causality (X → Y).
- One word when one word enough.
- Code blocks, commands, and error snippets stay exact.
- Example: 'Inline obj prop → new ref → re-render. Wrap in useMemo.'
- Drop caveman for security warnings or destructive confirmations; resume after.
- Stop immediately once the answer is delivered.",

            CavemanLevel::WenyanLite => "\
[INSTRUCTION: CAVEMAN MODE (WENYAN-LITE)]
Semi-classical Chinese conciseness (文言文簡略). Cut token usage drastically.
Rules:
- Drop filler and hedging, retain grammar structure in concise classical register.
- Technical terms, error messages, and code blocks remain exact.
- Drop caveman for security warnings or destructive confirmations; resume after.
- Stop immediately once the answer is delivered.",

            CavemanLevel::WenyanFull => "\
[INSTRUCTION: CAVEMAN MODE (WENYAN-FULL)]
Maximum classical terseness (文言文極簡). 80-90% character reduction.
Rules:
- Fully 文言文 phrasing. Verbs precede objects, subjects often omitted, use classical particles (之/乃/為/其).
- Technical terms, error messages, and code blocks remain exact.
- Example: '池reuse open connection。不每req新開。skip handshake overhead。'
- Drop caveman for security warnings or destructive confirmations; resume after.
- Stop immediately once the answer is delivered.",

            CavemanLevel::WenyanUltra => "\
[INSTRUCTION: CAVEMAN MODE (WENYAN-ULTRA)]
Extreme abbreviation while keeping classical Chinese feel. Maximum compression, ultra terse.
Rules:
- Maximum compression, ultra terse classical phrasing.
- Technical terms, error messages, and code blocks remain exact.
- Example: '池reuse conn。skip handshake → fast。'
- Drop caveman for security warnings or destructive confirmations; resume after.
- Stop immediately once the answer is delivered.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CavemanCommandAction {
    /// Show current status / prompt help
    Status,
    /// Disable caveman mode
    Disable,
    /// Set caveman level with an optional query following the command
    SetLevel {
        level: CavemanLevel,
        remainder_query: Option<String>,
    },
}

pub fn parse_caveman_command(args: &str) -> CavemanCommandAction {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return CavemanCommandAction::Status;
    }

    let mut parts = trimmed.split_whitespace();
    let first = parts.next().unwrap_or("");
    let remainder: String = parts.collect::<Vec<_>>().join(" ");
    let remainder_opt = if remainder.trim().is_empty() {
        None
    } else {
        Some(remainder.trim().to_string())
    };

    match first.to_lowercase().as_str() {
        "off" | "stop" | "disable" | "none" | "normal" => CavemanCommandAction::Disable,
        "lite" => CavemanCommandAction::SetLevel {
            level: CavemanLevel::Lite,
            remainder_query: remainder_opt,
        },
        "full" => CavemanCommandAction::SetLevel {
            level: CavemanLevel::Full,
            remainder_query: remainder_opt,
        },
        "ultra" => CavemanCommandAction::SetLevel {
            level: CavemanLevel::Ultra,
            remainder_query: remainder_opt,
        },
        "wenyan-lite" | "wenyan_lite" | "wenyanlite" => CavemanCommandAction::SetLevel {
            level: CavemanLevel::WenyanLite,
            remainder_query: remainder_opt,
        },
        "wenyan-full" | "wenyan_full" | "wenyanfull" | "wenyan" => CavemanCommandAction::SetLevel {
            level: CavemanLevel::WenyanFull,
            remainder_query: remainder_opt,
        },
        "wenyan-ultra" | "wenyan_ultra" | "wenyanultra" => CavemanCommandAction::SetLevel {
            level: CavemanLevel::WenyanUltra,
            remainder_query: remainder_opt,
        },
        _ => {
            // First word is not a keyword, so treated as full level with entire input as question
            CavemanCommandAction::SetLevel {
                level: CavemanLevel::Full,
                remainder_query: Some(trimmed.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_slash_commands_all_on_slash() {
        let all = match_slash_commands("/");
        assert!(all.len() >= 8);
    }

    #[test]
    fn test_match_slash_commands_prefix() {
        let cr = match_slash_commands("/cr");
        assert_eq!(cr.len(), 1);
        assert_eq!(cr[0].command, "crashloop");

        let p = match_slash_commands("/pen");
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].command, "pending");

        let sum = match_slash_commands("/sum");
        assert_eq!(sum.len(), 1);
        assert_eq!(sum[0].command, "summarise");

        let cave = match_slash_commands("/cave");
        assert_eq!(cave.len(), 1);
        assert_eq!(cave[0].command, "caveman");
    }

    #[test]
    fn test_expand_slash_command_targeted() {
        let expanded =
            expand_slash_command("crashloop", Some("auth-service-xyz"), "prod", "default").unwrap();
        assert!(expanded.contains("auth-service-xyz"));
        assert!(expanded.contains("default"));
        assert!(expanded.contains("crashed instance"));
    }

    #[test]
    fn test_expand_slash_command_discover() {
        let expanded = expand_slash_command("pending", None, "prod", "kube-system").unwrap();
        assert!(expanded.contains("kube-system"));
        assert!(expanded.contains("Scan namespace 'kube-system' for any pods"));
        assert!(expanded.contains("taints"));
    }

    #[test]
    fn test_expand_utility_commands_return_none() {
        assert!(expand_slash_command("clear", None, "prod", "default").is_none());
        assert!(expand_slash_command("save", None, "prod", "default").is_none());
        assert!(expand_slash_command("settings", None, "prod", "default").is_none());
        assert!(expand_slash_command("caveman", None, "prod", "default").is_none());
    }

    #[test]
    fn test_caveman_levels_and_parsing() {
        assert_eq!(CavemanLevel::parse("lite"), Some(CavemanLevel::Lite));
        assert_eq!(CavemanLevel::parse("full"), Some(CavemanLevel::Full));
        assert_eq!(CavemanLevel::parse("ultra"), Some(CavemanLevel::Ultra));
        assert_eq!(
            CavemanLevel::parse("wenyan-lite"),
            Some(CavemanLevel::WenyanLite)
        );
        assert_eq!(
            CavemanLevel::parse("wenyan-full"),
            Some(CavemanLevel::WenyanFull)
        );
        assert_eq!(
            CavemanLevel::parse("wenyan"),
            Some(CavemanLevel::WenyanFull)
        );
        assert_eq!(
            CavemanLevel::parse("wenyan-ultra"),
            Some(CavemanLevel::WenyanUltra)
        );
        assert_eq!(CavemanLevel::parse(""), Some(CavemanLevel::Full));
        assert_eq!(CavemanLevel::parse("unknown"), None);
    }

    #[test]
    fn test_parse_caveman_command() {
        assert_eq!(parse_caveman_command(""), CavemanCommandAction::Status);
        assert_eq!(parse_caveman_command("   "), CavemanCommandAction::Status);
        assert_eq!(parse_caveman_command("off"), CavemanCommandAction::Disable);
        assert_eq!(parse_caveman_command("stop"), CavemanCommandAction::Disable);
        assert_eq!(
            parse_caveman_command("normal"),
            CavemanCommandAction::Disable
        );

        assert_eq!(
            parse_caveman_command("ultra"),
            CavemanCommandAction::SetLevel {
                level: CavemanLevel::Ultra,
                remainder_query: None,
            }
        );

        assert_eq!(
            parse_caveman_command("lite why is pod pending?"),
            CavemanCommandAction::SetLevel {
                level: CavemanLevel::Lite,
                remainder_query: Some("why is pod pending?".to_string()),
            }
        );

        assert_eq!(
            parse_caveman_command("why is auth-service in crashloop?"),
            CavemanCommandAction::SetLevel {
                level: CavemanLevel::Full,
                remainder_query: Some("why is auth-service in crashloop?".to_string()),
            }
        );
    }

    #[test]
    fn test_caveman_prompt_instructions() {
        for level in [
            CavemanLevel::Lite,
            CavemanLevel::Full,
            CavemanLevel::Ultra,
            CavemanLevel::WenyanLite,
            CavemanLevel::WenyanFull,
            CavemanLevel::WenyanUltra,
        ] {
            let prompt = level.prompt_instructions();
            assert!(!prompt.is_empty());
            assert!(prompt.contains("INSTRUCTION: CAVEMAN MODE"));
        }
    }

    #[test]
    fn test_is_trusted_github_url() {
        assert!(is_trusted_github_url(
            "https://github.com/org/repo/raw/main/skill.md"
        ));
        assert!(is_trusted_github_url(
            "https://raw.githubusercontent.com/org/repo/main/skill.md"
        ));
        assert!(is_trusted_github_url(
            "https://gist.githubusercontent.com/user/id/raw/skill.md"
        ));
        assert!(is_trusted_github_url(
            "https://api.github.com/repos/org/repo"
        ));

        // Reject plaintext HTTP
        assert!(!is_trusted_github_url(
            "http://github.com/org/repo/raw/main/skill.md"
        ));

        // Reject attacker domains that mention github in query, path, or subdomain
        assert!(!is_trusted_github_url(
            "https://attacker.example/skill.md?source=github.com"
        ));
        assert!(!is_trusted_github_url(
            "https://attacker.example/github.com/skill.md"
        ));
        assert!(!is_trusted_github_url(
            "https://github.com.attacker.example/skill.md"
        ));
    }

    #[test]
    fn test_validate_custom_skill_name() {
        // Allowed custom names
        assert!(validate_custom_skill_name("my-skill").is_ok());
        assert!(validate_custom_skill_name("redis_triage").is_ok());

        // Allowed exact override of built-in diagnostic playbook
        assert!(validate_custom_skill_name("crashloop-triage").is_ok());
        assert!(validate_custom_skill_name("pending-pod").is_ok());

        // Disallowed reserved commands/utilities
        assert!(validate_custom_skill_name("clear").is_err());
        assert!(validate_custom_skill_name("help").is_err());
        assert!(validate_custom_skill_name("clear-chat").is_err());
        assert!(validate_custom_skill_name("save-chat").is_err());
        assert!(validate_custom_skill_name("ai-settings").is_err());
        assert!(validate_custom_skill_name("caveman").is_err());
        assert!(validate_custom_skill_name("save").is_err());
        assert!(validate_custom_skill_name("settings").is_err());
        assert!(validate_custom_skill_name("cave").is_err());
        assert!(validate_custom_skill_name("terse").is_err());
        assert!(validate_custom_skill_name("cls").is_err());
        assert!(validate_custom_skill_name("reset").is_err());
        assert!(validate_custom_skill_name("export").is_err());
        assert!(validate_custom_skill_name("config").is_err());
        assert!(validate_custom_skill_name("model").is_err());

        // Disallowed collision with built-in diagnostic command/alias
        assert!(validate_custom_skill_name("crashloop").is_err());
        assert!(validate_custom_skill_name("pend").is_err());
    }

    #[test]
    fn test_reserved_command_and_utility_collisions() {
        assert!(is_reserved_command_or_alias("clear"));
        assert!(is_reserved_command_or_alias("help"));
        assert!(is_reserved_command_or_alias("clear-chat"));
        assert!(is_reserved_command_or_alias("save-chat"));
        assert!(is_reserved_command_or_alias("ai-settings"));
        assert!(is_reserved_command_or_alias("caveman"));
        assert!(is_reserved_command_or_alias("save"));
        assert!(is_reserved_command_or_alias("export"));
        assert!(is_reserved_command_or_alias("settings"));
        assert!(is_reserved_command_or_alias("config"));
        assert!(is_reserved_command_or_alias("model"));
        assert!(is_reserved_command_or_alias("cave"));
        assert!(is_reserved_command_or_alias("terse"));
        assert!(is_reserved_command_or_alias("cls"));
        assert!(is_reserved_command_or_alias("reset"));
        assert!(is_reserved_command_or_alias("crashloop"));
        assert!(is_reserved_command_or_alias("pend"));

        // Allowed overrides and new names
        assert!(!is_reserved_command_or_alias("crashloop-triage"));
        assert!(!is_reserved_command_or_alias("pending-pod"));
        assert!(!is_reserved_command_or_alias("my-skill"));
    }

    #[test]
    fn test_load_all_playbooks_ignores_utility_overrides() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();

        std::fs::write(
            dir.path().join("caveman.md"),
            "---\nname: caveman\ndescription: Custom caveman\n---\nCustom instructions\n",
        )
        .unwrap();

        std::fs::write(
            dir.path().join("clear-chat.md"),
            "---\nname: clear-chat\ndescription: Custom clear\n---\nCustom instructions\n",
        )
        .unwrap();

        let playbooks = load_all_playbooks_from(dir.path());
        let caveman = playbooks.iter().find(|p| p.name == "caveman").unwrap();
        assert!(caveman.is_utility);
        assert_eq!(caveman.instructions, "");

        let clear = playbooks.iter().find(|p| p.name == "clear-chat").unwrap();
        assert!(clear.is_utility);
        assert_eq!(clear.instructions, "");
    }

    #[test]
    fn test_user_skill_file_checks_declared_name() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();

        // Write a file named "foo.md" that actually declares "name: bar"
        std::fs::write(
            dir.path().join("foo.md"),
            "---\nname: bar\ndescription: Bar\n---\nBody\n",
        )
        .unwrap();

        // Looking for "foo" should NOT return foo.md since its declared name is "bar"
        assert_eq!(user_skill_file(dir.path(), "foo"), None);

        // Looking for "bar" should find foo.md
        assert_eq!(
            user_skill_file(dir.path(), "bar"),
            Some(dir.path().join("foo.md"))
        );
    }

    #[test]
    fn test_match_slash_commands_case_insensitive() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();

        std::fs::write(
            dir.path().join("MySkill.md"),
            "---\nname: MySkill\ndescription: CamelCase skill\n---\nBody\n",
        )
        .unwrap();

        let matches = match_slash_commands_from("/myskill", dir.path());
        assert!(matches.iter().any(|s| s.name == "MySkill"));

        let matches_upper = match_slash_commands_from("/MYSKILL", dir.path());
        assert!(matches_upper.iter().any(|s| s.name == "MySkill"));
    }
}
