//! AI SRE Playbooks & Slash Commands Registry for SRElens TUI.
//!
//! Provides battle-tested Kubernetes diagnostic playbooks (matching GUI assistant_skills.rs)
//! with interactive slash-command autocomplete, zero-argument discovery fallbacks,
//! and contextual resource prompts.

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

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
    SkillDef {
        name: "caveman",
        command: "caveman",
        aliases: &["cave", "terse"],
        description: "Set ultra-compressed caveman mode to cut token usage ~75% (lite|full|ultra|wenyan-*|off)",
        target_kind: Some("lite|full|ultra|wenyan|off"),
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
            "wenyan-full" | "wenyan_full" | "wenyanfull" | "wenyan" => Some(CavemanLevel::WenyanFull),
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

        let cave = match_slash_commands("/cave");
        assert_eq!(cave.len(), 1);
        assert_eq!(cave[0].command, "caveman");
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
        assert!(expand_slash_command("caveman", None, "prod", "default").is_none());
    }

    #[test]
    fn test_caveman_levels_and_parsing() {
        assert_eq!(CavemanLevel::parse("lite"), Some(CavemanLevel::Lite));
        assert_eq!(CavemanLevel::parse("full"), Some(CavemanLevel::Full));
        assert_eq!(CavemanLevel::parse("ultra"), Some(CavemanLevel::Ultra));
        assert_eq!(CavemanLevel::parse("wenyan-lite"), Some(CavemanLevel::WenyanLite));
        assert_eq!(CavemanLevel::parse("wenyan-full"), Some(CavemanLevel::WenyanFull));
        assert_eq!(CavemanLevel::parse("wenyan"), Some(CavemanLevel::WenyanFull));
        assert_eq!(CavemanLevel::parse("wenyan-ultra"), Some(CavemanLevel::WenyanUltra));
        assert_eq!(CavemanLevel::parse(""), Some(CavemanLevel::Full));
        assert_eq!(CavemanLevel::parse("unknown"), None);
    }

    #[test]
    fn test_parse_caveman_command() {
        assert_eq!(parse_caveman_command(""), CavemanCommandAction::Status);
        assert_eq!(parse_caveman_command("   "), CavemanCommandAction::Status);
        assert_eq!(parse_caveman_command("off"), CavemanCommandAction::Disable);
        assert_eq!(parse_caveman_command("stop"), CavemanCommandAction::Disable);
        assert_eq!(parse_caveman_command("normal"), CavemanCommandAction::Disable);

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
}
