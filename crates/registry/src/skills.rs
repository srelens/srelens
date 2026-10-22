//! SRE playbooks and diagnostic skills capabilities for the MCP server.
//!
//! Exposes:
//! - `skills.get`: Retrieve an SRE playbook or diagnostic skill by name.
//!
//! Provides built-in battle-tested Kubernetes diagnostic playbooks and discovers
//! custom user skills stored in `~/.config/srelens/assistant/skills/*.md`
//! (and GUI config directory `app.srelens.desktop/assistant/skills/*.md`).

use std::fs;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

/// Input for retrieving an SRE playbook or diagnostic skill by name.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SkillGetIn {
    /// The unique name of the skill or playbook (e.g. "crashloop-triage", "pending-pod", "oomkilled", "node-pressure", "service-no-endpoints", "rollout-stuck", "cluster-summary", "argo-triage")
    pub name: String,
}

/// Output containing the full skill definition, metadata, and instructions.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillGetOut {
    pub name: String,
    pub description: String,
    pub body: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default, rename = "overridesBuiltin")]
    pub overrides_builtin: bool,
}

/// Reject any name that isn't `^[A-Za-z0-9._-]+$` — in particular, one
/// containing `/` or `\`, so a skill can never resolve outside the skills directory
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

/// Parses a skill file's text with optional YAML front-matter:
/// ```text
/// ---
/// name: my-skill
/// description: Triage memory leaks
/// ---
/// <markdown instructions>
/// ```
/// Falls back to `fallback_name` if `name:` is not present in front-matter.
pub fn parse_skill(
    text: &str,
    fallback_name: Option<&str>,
) -> Result<(String, String, String), String> {
    let normalized = text.replace("\r\n", "\n");
    let (name, desc, body) = if let Some(rest) = normalized.strip_prefix("---\n") {
        if let Some((header, body_content)) = rest.split_once("\n---\n") {
            let mut name = None;
            let mut desc = None;
            for line in header.lines() {
                if let Some(val) = line.strip_prefix("name:") {
                    name = Some(val.trim().to_string());
                } else if let Some(val) = line.strip_prefix("description:") {
                    desc = Some(val.trim().to_string());
                }
            }
            (name, desc, body_content.to_string())
        } else {
            (None, None, normalized.clone())
        }
    } else {
        (None, None, normalized.clone())
    };

    let final_name = name
        .or_else(|| fallback_name.map(|s| s.to_string()))
        .ok_or_else(|| "skill name is required (missing in frontmatter)".to_string())?;

    validate_name(&final_name)?;

    Ok((final_name, desc.unwrap_or_default(), body))
}

/// Resolve candidate directories where user skills may be stored.
/// If an explicit directory is provided, it is returned exclusively.
pub fn candidate_skills_dirs(explicit: Option<&Path>) -> Vec<PathBuf> {
    if let Some(dir) = explicit {
        return vec![dir.to_path_buf()];
    }
    let mut dirs = Vec::new();
    if let Ok(env_dir) = std::env::var("SRELENS_SKILLS_DIR") {
        let trimmed = env_dir.trim();
        if !trimmed.is_empty() {
            dirs.push(PathBuf::from(trimmed));
        }
    }
    if let Some(config) = dirs::config_dir() {
        let srelens_dir = config.join("srelens").join("assistant").join("skills");
        if !dirs.contains(&srelens_dir) {
            dirs.push(srelens_dir);
        }
        let desktop_dir = config
            .join("app.srelens.desktop")
            .join("assistant")
            .join("skills");
        if !dirs.contains(&desktop_dir) {
            dirs.push(desktop_dir);
        }
    }
    dirs
}

/// Shipped default diagnostic skills and playbooks.
pub fn builtin_skills() -> Vec<SkillGetOut> {
    vec![
        SkillGetOut {
            name: "crashloop-triage".into(),
            description: "Triage pod stuck in CrashLoopBackOff (previous logs, exit code, events)"
                .into(),
            body: "When a pod keeps restarting (CrashLoopBackOff):\n\
1. Read the pod's recent events for the restart reason.\n\
2. Fetch the PREVIOUS container's logs (the crashed instance, not the live one) — that's where the real error is.\n\
3. Check the container's last exit code and termination reason (e.g. Error vs OOMKilled).\n\
4. Compare the container's resource requests/limits against its actual usage.\n\
5. Check recent rollout history for a bad image or config change.\n\n\
Report the most likely root cause and one concrete next remediation step."
                .into(),
            builtin: true,
            path: None,
            overrides_builtin: false,
        },
        SkillGetOut {
            name: "pending-pod".into(),
            description: "Diagnose pod stuck in Pending or unschedulable (capacity, taints, PVCs)"
                .into(),
            body: "When a pod stays in Pending status:\n\
1. Read the pod's events — the scheduler explains why (Insufficient cpu/memory, no nodes match, taints, volume binding).\n\
2. Check node allocatable vs requested capacity across the cluster.\n\
3. Check the pod's nodeSelector, affinity, and tolerations against the nodes' labels and taints.\n\
4. If it mounts a PVC, check the PVC is Bound and its StorageClass can provision volume.\n\n\
Report which constraint blocks scheduling and how to relieve it."
                .into(),
            builtin: true,
            path: None,
            overrides_builtin: false,
        },
        SkillGetOut {
            name: "oomkilled".into(),
            description: "Investigate container OOMKilled events and memory limit sizing".into(),
            body: "When a container is being OOMKilled:\n\
1. Confirm the OOMKill from the pod's last-state and events.\n\
2. Compare the container's memory limit against its working-set usage over time.\n\
3. Check whether the limit is set too low, or usage is genuinely growing (memory leak).\n\
4. Look for a recent image/config change that raised memory use.\n\n\
Report whether to raise the limit or fix the workload, with a suggested memory value."
                .into(),
            builtin: true,
            path: None,
            overrides_builtin: false,
        },
        SkillGetOut {
            name: "node-pressure".into(),
            description: "Investigate node under CPU / Memory / Disk pressure or NotReady".into(),
            body: "When a node is under pressure or NotReady:\n\
1. Check the node's conditions (MemoryPressure, DiskPressure, PIDPressure) and events.\n\
2. Review the node's allocatable capacity vs the sum of pod requests and actual usage.\n\
3. Find the top resource-consuming pods on that node.\n\
4. Check for evicted pods as a symptom.\n\n\
Report the pressure source and whether to rebalance, scale, or cordon/drain the node."
                .into(),
            builtin: true,
            path: None,
            overrides_builtin: false,
        },
        SkillGetOut {
            name: "service-no-endpoints".into(),
            description: "Debug Service that has no ready endpoints or connection refused".into(),
            body: "When a Service is unreachable or has zero endpoints:\n\
1. Compare the Service's selector against the target pods' labels — a mismatch yields zero endpoints.\n\
2. Check the backing pods are Ready (failing readiness probes are excluded from endpoints).\n\
3. Confirm the Service targetPort matches the container's listening port.\n\
4. Inspect the EndpointSlices for the Service.\n\n\
Report why endpoints are empty and provide the exact fix."
                .into(),
            builtin: true,
            path: None,
            overrides_builtin: false,
        },
        SkillGetOut {
            name: "rollout-stuck".into(),
            description: "Diagnose Deployment rollout that is stalled or failing progression"
                .into(),
            body: "When a Deployment rollout is stuck:\n\
1. Check the Deployment's status conditions (Progressing, Available) and the active ReplicaSets.\n\
2. Inspect the new pods — ImagePullBackOff, CrashLoopBackOff, or failing readiness all stall a rollout.\n\
3. Verify replica counts: desired vs updated vs available.\n\
4. Check events for quota limits, scheduling blockers, or probe failures.\n\n\
Report what blocks the rollout and whether to fix forward or roll back."
                .into(),
            builtin: true,
            path: None,
            overrides_builtin: false,
        },
        SkillGetOut {
            name: "cluster-summary".into(),
            description:
                "Generate an executive SRE health and incident briefing for the cluster"
                    .into(),
            body: "Perform an executive SRE health assessment of the active cluster:\n\
1. Query cluster-wide pod counts and identify all pods in non-Running phases (CrashLoop, Pending, Error, Evicted).\n\
2. Inspect nodes for any NotReady states or pressure conditions (Memory/Disk pressure).\n\
3. Review cluster warning events from the last 30 minutes to identify emerging incident patterns.\n\
4. Check for stalled Deployments or StatefulSets with unmet replica requirements.\n\n\
Synthesize the findings into an executive briefing with high-priority risks and suggested remediation actions."
                .into(),
            builtin: true,
            path: None,
            overrides_builtin: false,
        },
        SkillGetOut {
            name: "argo-triage".into(),
            description:
                "Diagnose ArgoCD application stuck in Progressing, Degraded, or OutOfSync"
                    .into(),
            body: "When an ArgoCD application is stuck in Progressing, Degraded, or failing to sync:\n\
1. Check the Application's sync status, health status, and health message.\n\
2. Inspect the operation phase and error message of the last sync.\n\
3. Identify which specific child resources are Degraded, Progressing, or Missing.\n\
4. Check for drift between live cluster state and the target Git revision.\n\
5. Inspect SyncHook jobs or PreSync/PostSync failures.\n\
6. Check the target namespace and cluster destination connectivity.\n\n\
Report the root cause why the application is stuck or degraded and provide concrete remediation steps (sync flags, manifest fix, or controller action)."
                .into(),
            builtin: true,
            path: None,
            overrides_builtin: false,
        },
    ]
}

/// Load a skill by name: first searches candidate user directories, then falls back to built-ins.
pub fn load_skill_or_builtin(dirs: &[PathBuf], name: &str) -> Result<SkillGetOut, CapabilityError> {
    validate_name(name).map_err(CapabilityError::Handler)?;

    let is_shipped = builtin_skills().into_iter().find(|s| s.name == name);

    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let exact = dir.join(format!("{name}.md"));
        if exact.is_file() {
            if let Ok(raw) = fs::read_to_string(&exact) {
                if let Ok((parsed_name, desc, body)) = parse_skill(&raw, Some(name)) {
                    if parsed_name == name {
                        return Ok(SkillGetOut {
                            name: parsed_name,
                            description: desc,
                            body,
                            builtin: false,
                            path: Some(exact.display().to_string()),
                            overrides_builtin: is_shipped.is_some(),
                        });
                    }
                }
            }
        }
        if let Ok(entries) = fs::read_dir(dir) {
            let mut paths: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
                })
                .collect();
            paths.sort();
            for path in paths {
                if let Ok(raw) = fs::read_to_string(&path) {
                    let fallback_stem = path.file_stem().and_then(|s| s.to_str());
                    if let Ok((parsed_name, desc, body)) = parse_skill(&raw, fallback_stem) {
                        if parsed_name == name {
                            return Ok(SkillGetOut {
                                name: parsed_name,
                                description: desc,
                                body,
                                builtin: false,
                                path: Some(path.display().to_string()),
                                overrides_builtin: is_shipped.is_some(),
                            });
                        }
                    }
                }
            }
        }
    }

    if let Some(builtin) = is_shipped {
        return Ok(builtin);
    }

    Err(CapabilityError::Handler(format!(
        "no diagnostic playbook or skill found named {name:?}"
    )))
}

/// Build the `skills.get` capability.
pub fn skills_get_capability(skills_dir: Option<PathBuf>) -> Capability {
    Capability::typed::<SkillGetIn, SkillGetOut, _, _>(
        "skills.get",
        "retrieve an SRE playbook or diagnostic skill by name (e.g. crashloop-triage, pending-pod, oomkilled, node-pressure, service-no-endpoints, rollout-stuck, cluster-summary, argo-triage)",
        Annotations::READ_ONLY,
        move |input: SkillGetIn| {
            let skills_dir = skills_dir.clone();
            async move {
                let dirs = candidate_skills_dirs(skills_dir.as_deref());
                load_skill_or_builtin(&dirs, &input.name)
            }
        },
    )
}

/// Build the `skills.get` capability serving strictly built-in playbooks without disk access
/// (used for multi-tenant web sessions to prevent reading host process directories or leaking paths).
pub fn skills_get_builtin_only_capability() -> Capability {
    Capability::typed::<SkillGetIn, SkillGetOut, _, _>(
        "skills.get",
        "retrieve an SRE playbook or diagnostic skill by name (e.g. crashloop-triage, pending-pod, oomkilled, node-pressure, service-no-endpoints, rollout-stuck, cluster-summary, argo-triage)",
        Annotations::READ_ONLY,
        |input: SkillGetIn| async move { load_skill_or_builtin(&[], &input.name) },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_builtin_skills_present_and_have_instructions() {
        let builtins = builtin_skills();
        assert_eq!(builtins.len(), 8);
        for b in &builtins {
            assert!(!b.name.is_empty());
            assert!(!b.description.is_empty());
            assert!(!b.body.is_empty());
            assert!(b.builtin);
            assert!(b.path.is_none());
            assert!(!b.overrides_builtin);
        }
        let names: Vec<&str> = builtins.iter().map(|b| b.name.as_str()).collect();
        assert!(names.contains(&"crashloop-triage"));
        assert!(names.contains(&"pending-pod"));
        assert!(names.contains(&"oomkilled"));
        assert!(names.contains(&"node-pressure"));
        assert!(names.contains(&"service-no-endpoints"));
        assert!(names.contains(&"rollout-stuck"));
        assert!(names.contains(&"cluster-summary"));
        assert!(names.contains(&"argo-triage"));
    }

    #[tokio::test]
    async fn test_skills_get_builtin() {
        let cap = skills_get_capability(None);
        let resp = cap.handler.as_ref()(serde_json::json!({ "name": "crashloop-triage" }))
            .await
            .expect("crashloop-triage capability succeeds");

        let out: SkillGetOut = serde_json::from_value(resp).expect("valid SkillGetOut");
        assert_eq!(out.name, "crashloop-triage");
        assert!(out.builtin);
        assert!(!out.overrides_builtin);
        assert!(out.body.contains("CrashLoopBackOff"));
        assert!(out.body.contains("PREVIOUS"));
    }

    #[tokio::test]
    async fn test_skills_get_not_found() {
        let cap = skills_get_capability(None);
        let err = cap.handler.as_ref()(serde_json::json!({ "name": "non-existent-skill" }))
            .await
            .expect_err("should return not found error");

        assert!(err
            .to_string()
            .contains("no diagnostic playbook or skill found"));
    }

    #[tokio::test]
    async fn test_skills_get_invalid_name() {
        let cap = skills_get_capability(None);
        let err = cap.handler.as_ref()(serde_json::json!({ "name": "../sneaky" }))
            .await
            .expect_err("should reject invalid name");

        assert!(err.to_string().contains("invalid skill name"));
    }

    #[tokio::test]
    async fn test_skills_get_user_custom_and_override() {
        let dir = tempdir().unwrap();
        let custom_file = dir.path().join("my-custom.md");
        fs::write(
            &custom_file,
            "---\nname: my-custom\ndescription: Custom playbook\n---\nStep 1: Check DNS\n",
        )
        .unwrap();

        let override_file = dir.path().join("pending-pod.md");
        fs::write(
            &override_file,
            "---\nname: pending-pod\ndescription: Overridden pending playbook\n---\nCustom pending checks\n",
        )
        .unwrap();

        let cap = skills_get_capability(Some(dir.path().to_path_buf()));

        // Check custom skill
        let resp_custom = cap.handler.as_ref()(serde_json::json!({ "name": "my-custom" }))
            .await
            .unwrap();
        let custom_out: SkillGetOut = serde_json::from_value(resp_custom).unwrap();
        assert_eq!(custom_out.name, "my-custom");
        assert_eq!(custom_out.description, "Custom playbook");
        assert_eq!(custom_out.body, "Step 1: Check DNS\n");
        assert!(!custom_out.builtin);
        assert!(!custom_out.overrides_builtin);
        assert_eq!(custom_out.path, Some(custom_file.display().to_string()));

        // Check overridden built-in
        let resp_override = cap.handler.as_ref()(serde_json::json!({ "name": "pending-pod" }))
            .await
            .unwrap();
        let override_out: SkillGetOut = serde_json::from_value(resp_override).unwrap();
        assert_eq!(override_out.name, "pending-pod");
        assert_eq!(override_out.description, "Overridden pending playbook");
        assert_eq!(override_out.body, "Custom pending checks\n");
        assert!(!override_out.builtin);
        assert!(override_out.overrides_builtin);
        assert_eq!(override_out.path, Some(override_file.display().to_string()));
    }

    #[test]
    fn test_wire_serialization_camel_case() {
        let out = SkillGetOut {
            name: "test-skill".into(),
            description: "desc".into(),
            body: "echo hello".into(),
            builtin: true,
            path: None,
            overrides_builtin: false,
        };
        let val_out = serde_json::to_value(&out).unwrap();
        assert_eq!(val_out["overridesBuiltin"], false);
        assert!(val_out.get("overrides_builtin").is_none());

        let out_override = SkillGetOut {
            name: "override-skill".into(),
            description: "desc".into(),
            body: "echo override".into(),
            builtin: false,
            path: Some("/path/to/skill.md".into()),
            overrides_builtin: true,
        };
        let val_override = serde_json::to_value(&out_override).unwrap();
        assert_eq!(val_override["overridesBuiltin"], true);
        assert!(val_override.get("overrides_builtin").is_none());
    }

    #[tokio::test]
    async fn test_skills_get_builtin_only_capability() {
        let cap = skills_get_builtin_only_capability();

        // Built-in succeeds with no path
        let resp = cap.handler.as_ref()(serde_json::json!({ "name": "crashloop-triage" }))
            .await
            .unwrap();
        let out: SkillGetOut = serde_json::from_value(resp).unwrap();
        assert_eq!(out.name, "crashloop-triage");
        assert!(out.builtin);
        assert_eq!(out.path, None);
        assert!(!out.overrides_builtin);

        // Custom skill is not found and cannot search host disk
        let err = cap.handler.as_ref()(serde_json::json!({ "name": "custom-disk-skill" }))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("no diagnostic playbook or skill found"));
    }
}
