//! AI SRE Playbooks & Slash Commands Registry for SRElens TUI.
//!
//! Provides battle-tested Kubernetes diagnostic playbooks (matching GUI assistant_skills.rs)
//! with interactive slash-command autocomplete, zero-argument discovery fallbacks,
//! and contextual resource prompts.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDef {
    pub name: &'static str,
    pub command: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub target_kind: Option<&'static str>,
    pub instructions: &'static str,
    pub is_utility: bool,
}

impl SkillDef {
    pub fn target_placeholder(&self) -> &'static str {
        self.target_kind.unwrap_or("")
    }
}

pub const BUILTIN_PLAYBOOKS: &[SkillDef] = &[
    SkillDef {
        name: "crashloop-triage",
        command: "crashloop",
        aliases: &["crash", "crashloop-triage"],
        description: "Triage pod stuck in CrashLoopBackOff (previous logs, exit code, events)",
        target_kind: Some("Pod"),
        instructions: "When a pod keeps restarting (CrashLoopBackOff):\n\
1. Read the pod's recent events for the restart reason.\n\
2. Fetch the PREVIOUS container's logs (the crashed instance, not the live one) — that's where the real error is.\n\
3. Check the container's last exit code and termination reason (e.g. Error vs OOMKilled).\n\
4. Compare the container's resource requests/limits against its actual usage.\n\
5. Check recent rollout history for a bad image or config change.\n\n\
Report the most likely root cause and one concrete next remediation step.",
        is_utility: false,
    },
    SkillDef {
        name: "pending-pod",
        command: "pending",
        aliases: &["pend", "pending-pod", "unschedulable"],
        description: "Diagnose pod stuck in Pending or unschedulable (capacity, taints, PVCs)",
        target_kind: Some("Pod"),
        instructions: "When a pod stays in Pending status:\n\
1. Read the pod's events — the scheduler explains why (Insufficient cpu/memory, no nodes match, taints, volume binding).\n\
2. Check node allocatable vs requested capacity across the cluster.\n\
3. Check the pod's nodeSelector, affinity, and tolerations against the nodes' labels and taints.\n\
4. If it mounts a PVC, check the PVC is Bound and its StorageClass can provision volume.\n\n\
Report which constraint blocks scheduling and how to relieve it.",
        is_utility: false,
    },
    SkillDef {
        name: "oomkilled",
        command: "oom",
        aliases: &["oomkilled", "memory-leak"],
        description: "Investigate container OOMKilled events and memory limit sizing",
        target_kind: Some("Pod"),
        instructions: "When a container is being OOMKilled:\n\
1. Confirm the OOMKill from the pod's last-state and events.\n\
2. Compare the container's memory limit against its working-set usage over time.\n\
3. Check whether the limit is set too low, or usage is genuinely growing (memory leak).\n\
4. Look for a recent image/config change that raised memory use.\n\n\
Report whether to raise the limit or fix the workload, with a suggested memory value.",
        is_utility: false,
    },
    SkillDef {
        name: "node-pressure",
        command: "nodepressure",
        aliases: &["node", "node-pressure", "drain-check"],
        description: "Investigate node under CPU / Memory / Disk pressure or NotReady",
        target_kind: Some("Node"),
        instructions: "When a node is under pressure or NotReady:\n\
1. Check the node's conditions (MemoryPressure, DiskPressure, PIDPressure) and events.\n\
2. Review the node's allocatable capacity vs the sum of pod requests and actual usage.\n\
3. Find the top resource-consuming pods on that node.\n\
4. Check for evicted pods as a symptom.\n\n\
Report the pressure source and whether to rebalance, scale, or cordon/drain the node.",
        is_utility: false,
    },
    SkillDef {
        name: "service-no-endpoints",
        command: "endpoints",
        aliases: &["service-no-endpoints", "no-endpoints", "svc"],
        description: "Debug Service that has no ready endpoints or connection refused",
        target_kind: Some("Service"),
        instructions: "When a Service is unreachable or has zero endpoints:\n\
1. Compare the Service's selector against the target pods' labels — a mismatch yields zero endpoints.\n\
2. Check the backing pods are Ready (failing readiness probes are excluded from endpoints).\n\
3. Confirm the Service targetPort matches the container's listening port.\n\
4. Inspect the EndpointSlices for the Service.\n\n\
Report why endpoints are empty and provide the exact fix.",
        is_utility: false,
    },
    SkillDef {
        name: "rollout-stuck",
        command: "rollout",
        aliases: &["rollout-stuck", "stuck-deploy"],
        description: "Diagnose Deployment rollout that is stalled or failing progression",
        target_kind: Some("Deployment"),
        instructions: "When a Deployment rollout is stuck:\n\
1. Check the Deployment's status conditions (Progressing, Available) and the active ReplicaSets.\n\
2. Inspect the new pods — ImagePullBackOff, CrashLoopBackOff, or failing readiness all stall a rollout.\n\
3. Verify replica counts: desired vs updated vs available.\n\
4. Check events for quota limits, scheduling blockers, or probe failures.\n\n\
Report what blocks the rollout and whether to fix forward or roll back.",
        is_utility: false,
    },
    SkillDef {
        name: "cluster-summary",
        command: "summarise",
        aliases: &["summary", "cluster-summary", "health"],
        description: "Generate an executive SRE health and incident briefing for the cluster",
        target_kind: None,
        instructions: "Perform an executive SRE health assessment of the active cluster:\n\
1. Query cluster-wide pod counts and identify all pods in non-Running phases (CrashLoop, Pending, Error, Evicted).\n\
2. Inspect nodes for any NotReady states or pressure conditions (Memory/Disk pressure).\n\
3. Review cluster warning events from the last 30 minutes to identify emerging incident patterns.\n\
4. Check for stalled Deployments or StatefulSets with unmet replica requirements.\n\n\
Synthesize the findings into an executive briefing with high-priority risks and suggested remediation actions.",
        is_utility: false,
    },
    SkillDef {
        name: "clear-chat",
        command: "clear",
        aliases: &["cls", "reset"],
        description: "Clear conversation history and reset assistant state",
        target_kind: None,
        instructions: "",
        is_utility: true,
    },
    SkillDef {
        name: "save-chat",
        command: "save",
        aliases: &["export"],
        description: "Export current conversation to a markdown file",
        target_kind: None,
        instructions: "",
        is_utility: true,
    },
    SkillDef {
        name: "ai-settings",
        command: "settings",
        aliases: &["config", "model"],
        description: "Open AI model, API key, and provider configuration",
        target_kind: None,
        instructions: "",
        is_utility: true,
    },
];

/// Finds all slash commands matching the user's current input.
/// Input starts with `/`.
pub fn match_slash_commands(input: &str) -> Vec<&'static SkillDef> {
    let clean = input.trim_start_matches('/');
    let query = clean.split_whitespace().next().unwrap_or("").to_lowercase();

    BUILTIN_PLAYBOOKS
        .iter()
        .filter(|s| {
            if query.is_empty() {
                return true;
            }
            s.command.starts_with(&query)
                || s.aliases.iter().any(|a| a.starts_with(&query))
                || s.name.starts_with(&query)
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
    let skill = BUILTIN_PLAYBOOKS.iter().find(|s| {
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
            let kind = skill.target_kind.unwrap_or("resource");
            format!("Focus on {} '{}' in {}.", kind, target, ns_desc)
        }
        None => match skill.target_kind {
            Some("Pod") => format!("Scan {} for any pods experiencing this issue and triage them.", ns_desc),
            Some("Node") => "Scan the cluster for any nodes experiencing pressure or unreadiness and triage them.".to_string(),
            Some("Deployment") => format!("Scan {} for any stalled or degraded rollouts and triage them.", ns_desc),
            Some("Service") => format!("Scan {} for any services with zero endpoints or connection issues and triage them.", ns_desc),
            _ => format!("Investigate cluster '{}' in {}.", active_context, ns_desc),
        },
    };

    let prompt = format!(
        "Playbook: {}\nContext: Cluster '{}', {}\n{}\n\nGuidelines:\n{}",
        skill.description,
        active_context,
        ns_desc,
        target_statement,
        skill.instructions
    );

    Some(prompt)
}

/// Scans `~/.config/srelens/assistant/skills/*.md` for user-defined skills.
pub fn load_user_skills_dir() -> PathBuf {
    dirs::config_dir()
        .map(|d| d.join("srelens").join("assistant").join("skills"))
        .unwrap_or_else(|| PathBuf::from(".srelens/skills"))
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
    }

    #[test]
    fn test_expand_slash_command_targeted() {
        let expanded = expand_slash_command("crashloop", Some("auth-service-xyz"), "prod", "default").unwrap();
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
    }
}
