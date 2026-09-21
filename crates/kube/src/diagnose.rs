use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticVerdict {
    Healthy,
    OOMKilled,
    CrashLoopBackOff,
    ConfigError,
    ImagePullFailed,
    SchedulingFailed,
    Evicted,
    Failed,
    NodeLost,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticSignal {
    pub severity: SignalSeverity,
    pub title: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticReport {
    pub verdict: DiagnosticVerdict,
    pub summary: String,
    pub remediation: String,
    pub signals: Vec<DiagnosticSignal>,
}

fn extract_quoted_string(s: &str) -> Option<&str> {
    let start = s.find('"')?;
    let end = s[start + 1..].find('"')?;
    Some(&s[start + 1..start + 1 + end])
}

pub fn make_one_line_gist(category: &str, raw_detail: &str, max_len: usize) -> String {
    let raw = raw_detail.trim();
    if raw.is_empty() {
        return category.to_string();
    }

    // 1. Semantic pattern extractors

    // Pattern A: Device allocation failure (GPU, KVM, SRIOV, device plugin)
    if (raw.contains("Allocate failed")
        || raw.contains("cannot allocate")
        || raw.contains("healthy devices"))
        && (raw.contains('/') || raw.contains("device"))
    {
        let device = raw.split_whitespace().find_map(|w| {
            let clean =
                w.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '.' && c != '-');
            if clean.contains('/')
                && (clean.ends_with("/gpu")
                    || clean.contains(".io/")
                    || clean.contains(".com/")
                    || clean.contains(".org/"))
            {
                Some(clean)
            } else {
                None
            }
        });

        if let Some(dev) = device {
            if raw.contains("no healthy devices") || raw.contains("unhealthy") {
                let s = format!("Allocate failed: No healthy {} devices", dev);
                if s.chars().count() <= max_len {
                    return s;
                }
            } else {
                let s = format!("Allocate failed for {}", dev);
                if s.chars().count() <= max_len {
                    return s;
                }
            }
        }
    }

    // Pattern B: Volume / PVC mount failure
    if raw.contains("MountVolume") || raw.contains("failed to mount") || category == "FailedMount" {
        let pvc = extract_quoted_string(raw);
        if raw.contains("not found") {
            if let Some(name) = pvc {
                let s = format!("Mount failed: PVC \"{}\" not found", name);
                if s.chars().count() <= max_len {
                    return s;
                }
            } else {
                return "Mount failed: Volume or PVC not found".to_string();
            }
        }
    }

    // Pattern C: Network / CNI / Sandbox setup failure
    if (raw.contains("setup network")
        || raw.contains("sandbox")
        || category == "FailedCreatePodSandBox")
        && (raw.contains("IP addresses") || raw.contains("network"))
    {
        if raw.contains("no IP addresses") || raw.contains("IPAM") {
            let net_name = raw
                .split("network:")
                .nth(1)
                .or_else(|| raw.split("network ").nth(1))
                .map(|s| {
                    s.trim()
                        .trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
                });
            if let Some(net) = net_name.filter(|s| !s.is_empty()) {
                let s = format!("Network setup failed: No IP addresses in {}", net);
                if s.chars().count() <= max_len {
                    return s;
                }
            } else {
                return "Network setup failed: No IP addresses available".to_string();
            }
        }
    }

    // Pattern D: Image pull failure
    if (category == "ImagePullBackOff" || category == "ErrImagePull" || raw.contains("pull"))
        && (raw.contains("not found")
            || raw.contains("manifest unknown")
            || raw.contains("unauthorized")
            || raw.contains("denied"))
    {
        let image = extract_quoted_string(raw);
        if raw.contains("unauthorized") || raw.contains("denied") {
            if let Some(img) = image {
                let s = format!("Image pull failed: Auth error for \"{}\"", img);
                if s.chars().count() <= max_len {
                    return s;
                }
            }
        } else if let Some(img) = image {
            let s = format!(
                "{}: \"{}\" not found",
                if !category.is_empty() {
                    category
                } else {
                    "ImagePullFailed"
                },
                img
            );
            if s.chars().count() <= max_len {
                return s;
            }
        }
    }

    // Pattern E: DeadlineExceeded
    if category == "DeadlineExceeded" || raw.contains("deadline") {
        if let Some(secs) = raw.split("deadline").nth(1).and_then(|s| {
            s.split_whitespace()
                .find(|w| w.chars().all(|c| c.is_ascii_digit()))
        }) {
            let s = format!("DeadlineExceeded: Active longer than {}s", secs);
            if s.chars().count() <= max_len {
                return s;
            }
        }
    }

    // Pattern F: Missing Secret / ConfigMap
    if raw.contains("not found") && (raw.contains("secret") || raw.contains("configmap")) {
        let name = extract_quoted_string(raw);
        let kind = if raw.contains("secret") {
            "Secret"
        } else {
            "ConfigMap"
        };
        if let Some(n) = name {
            let s = if !category.is_empty() && category != "Failed" && category != "Error" {
                format!("{}: {} \"{}\" not found", category, kind, n)
            } else {
                format!("{}: \"{}\" not found", kind, n)
            };
            if s.chars().count() <= max_len {
                return s;
            }
        }
    }

    // Pattern G: Permission denied / OCI runtime error
    if raw.contains("permission denied") {
        let s = if !category.is_empty() && category != "Failed" && category != "Error" {
            format!("{}: Permission denied", category)
        } else {
            "Permission denied starting container process".to_string()
        };
        if s.chars().count() <= max_len {
            return s;
        }
    }

    // General Fallback
    // 1. Strip redundant boilerplates
    let stripped = raw
        .strip_prefix("Pod was rejected: ")
        .or_else(|| raw.strip_prefix("Pod was rejected:"))
        .or_else(|| raw.strip_prefix("Error syncing pod: "))
        .or_else(|| raw.strip_prefix("Error: "))
        .or_else(|| raw.strip_prefix("failed to "))
        .or_else(|| raw.strip_prefix("Failed to "))
        .or_else(|| raw.strip_prefix("The node was low on resource: "))
        .unwrap_or(raw)
        .trim();

    // 2. If it has RPC / CRI error "desc = ...", extract the description
    let content = if let Some(idx) = stripped.find("desc = ") {
        stripped[idx + "desc = ".len()..].trim().trim_matches('"')
    } else {
        stripped
    };

    // 3. Extract the primary clause before delimiters (;, \n)
    let clause = content.split([';', '\n']).next().unwrap_or(content).trim();

    // 4. Format headline: prepend category if not already present
    let headline = if !category.is_empty()
        && category != "Failed"
        && category != "Error"
        && !clause.to_lowercase().starts_with(&category.to_lowercase())
    {
        format!("{}: {}", category, clause)
    } else {
        clause.to_string()
    };

    // 5. Budget cap at max_len with whole-word ellipsis
    if headline.chars().count() <= max_len {
        headline
    } else {
        let mut truncated = String::new();
        let mut cur_len = 0;
        let target = max_len.saturating_sub(1);
        for word in headline.split_whitespace() {
            let word_len = word.chars().count();
            if cur_len == 0 {
                truncated.push_str(word);
                cur_len += word_len;
            } else if cur_len + 1 + word_len <= target {
                truncated.push(' ');
                truncated.push_str(word);
                cur_len += 1 + word_len;
            } else {
                break;
            }
        }
        if truncated.is_empty() {
            let s: String = headline.chars().take(target).collect();
            format!("{}…", s)
        } else {
            format!("{}…", truncated)
        }
    }
}

fn parse_scheduling_failure(raw: &str) -> (String, Vec<DiagnosticSignal>) {
    let mut signals = Vec::new();

    // 1. Separate main node availability from preemption suffix
    let parts: Vec<&str> = raw.split(", preemption:").collect();
    let main_part = parts[0].trim();
    let preemption_part = parts.get(1).map(|s| s.trim());

    // Look for "0/N nodes are available:" or "0/N node(s) were available:"
    let (node_prefix, reasons_str) = if let Some(idx) = main_part.find("nodes are available:") {
        let prefix = main_part[..idx + "nodes are available:".len()].trim();
        let rest = main_part[idx + "nodes are available:".len()..].trim();
        (prefix, rest)
    } else if let Some(idx) = main_part.find("node(s) were available:") {
        let prefix = main_part[..idx + "node(s) were available:".len()].trim();
        let rest = main_part[idx + "node(s) were available:".len()..].trim();
        (prefix, rest)
    } else {
        ("Scheduling failed:", main_part)
    };

    // Parse reasons (separated by comma)
    let raw_reasons: Vec<&str> = reasons_str
        .split(',')
        .map(|r| r.trim().trim_end_matches('.'))
        .filter(|r| !r.is_empty())
        .collect();

    let mut gist_reasons = Vec::new();

    for reason in &raw_reasons {
        let lower = reason.to_lowercase();
        if lower.contains("insufficient") {
            let res_name = reason
                .split_whitespace()
                .filter(|w| {
                    !w.chars().all(|c| c.is_numeric()) && !w.eq_ignore_ascii_case("insufficient")
                })
                .collect::<Vec<_>>()
                .join(" ");
            let clean_res = if !res_name.is_empty() {
                res_name
            } else {
                "resources".to_string()
            };
            gist_reasons.push(format!("Insufficient {}", clean_res));

            signals.push(DiagnosticSignal {
                severity: SignalSeverity::Error,
                title: format!("Insufficient Node Resources ({})", clean_res),
                detail: Some(reason.to_string()),
            });
        } else if lower.contains("too many pods") {
            gist_reasons.push("Pod limit reached".to_string());
            signals.push(DiagnosticSignal {
                severity: SignalSeverity::Error,
                title: "Node Pod Capacity Reached".to_string(),
                detail: Some(reason.to_string()),
            });
        } else if lower.contains("taint") || lower.contains("tolerat") {
            gist_reasons.push("Untolerated taint".to_string());
            signals.push(DiagnosticSignal {
                severity: SignalSeverity::Warning,
                title: "Untolerated Node Taint".to_string(),
                detail: Some(reason.to_string()),
            });
        } else if lower.contains("affinity")
            || lower.contains("selector")
            || lower.contains("match")
        {
            gist_reasons.push("Node affinity mismatch".to_string());
            signals.push(DiagnosticSignal {
                severity: SignalSeverity::Warning,
                title: "Node Affinity / Selector Mismatch".to_string(),
                detail: Some(reason.to_string()),
            });
        } else {
            gist_reasons.push(reason.to_string());
            signals.push(DiagnosticSignal {
                severity: SignalSeverity::Warning,
                title: "Scheduling Constraint".to_string(),
                detail: Some(reason.to_string()),
            });
        }
    }

    if let Some(prem) = preemption_part {
        signals.push(DiagnosticSignal {
            severity: SignalSeverity::Info,
            title: "Preemption Check".to_string(),
            detail: Some(format!("Preemption: {}", prem)),
        });
    }

    if signals.is_empty() {
        signals.push(DiagnosticSignal {
            severity: SignalSeverity::Warning,
            title: "Pod is Unschedulable".to_string(),
            detail: Some(raw.to_string()),
        });
    }

    // Build concise 1-line gist
    let prefix_short = if let Some(n) = node_prefix.strip_suffix(" are available:") {
        format!("{} nodes available", n.trim_end_matches(" nodes"))
    } else {
        node_prefix.trim_end_matches(':').to_string()
    };

    let summary = if !gist_reasons.is_empty() {
        let reasons_joined = gist_reasons.join(", ");
        if prefix_short.len() + reasons_joined.len() + 3 <= 75 {
            format!("{} ({})", prefix_short, reasons_joined)
        } else {
            let first_reasons = gist_reasons[..gist_reasons.len().min(2)].join(", ");
            format!("{} ({})", prefix_short, first_reasons)
        }
    } else {
        format!("{}: pod unschedulable", prefix_short)
    };

    (summary, signals)
}

pub fn analyze_pod_health(
    pod: &Value,
    events: &[Value],
    previous_logs: Option<&str>,
) -> DiagnosticReport {
    let mut signals = Vec::new();

    // Check container statuses (init, regular, and ephemeral)
    let empty_statuses = Vec::new();
    let regular_statuses = pod
        .pointer("/status/containerStatuses")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty_statuses);
    let init_statuses = pod
        .pointer("/status/initContainerStatuses")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty_statuses);
    let ephemeral_statuses = pod
        .pointer("/status/ephemeralContainerStatuses")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty_statuses);

    for cs in init_statuses
        .iter()
        .chain(regular_statuses.iter())
        .chain(ephemeral_statuses.iter())
    {
        let name = cs.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
        let terminated = cs
            .pointer("/lastState/terminated")
            .or_else(|| cs.pointer("/state/terminated"));

        if let Some(term) = terminated {
            let reason = term.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            let exit_code = term.get("exitCode").and_then(|v| v.as_i64()).unwrap_or(0);

            if reason == "OOMKilled" || exit_code == 137 {
                // Look up container spec for limits in both initContainers and containers
                let memory_limit = pod
                    .pointer("/spec/containers")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                    .chain(
                        pod.pointer("/spec/initContainers")
                            .and_then(|v| v.as_array())
                            .into_iter()
                            .flatten(),
                    )
                    .find(|c| c.get("name").and_then(|n| n.as_str()) == Some(name))
                    .and_then(|c| c.pointer("/resources/limits/memory"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("none");

                signals.push(DiagnosticSignal {
                    severity: SignalSeverity::Error,
                    title: format!("Container '{}' was OOMKilled", name),
                    detail: Some(format!(
                        "Exit code 137. Last terminated reason: OOMKilled. Memory limit: {}",
                        memory_limit
                    )),
                });

                let summary = make_one_line_gist(
                    "OOMKilled",
                    &format!("OOMKilled: container '{}' exceeded memory limit", name),
                    65,
                );

                return DiagnosticReport {
                    verdict: DiagnosticVerdict::OOMKilled,
                    summary,
                    remediation: format!("Container '{}' exceeded memory limit ({}). Increase memory limit in pod spec.", name, memory_limit),
                    signals,
                };
            }
        }

        // Check for ConfigError (e.g. missing Secret / ConfigMap)
        let waiting_reason = cs.pointer("/state/waiting/reason").and_then(|v| v.as_str());
        let waiting_msg = cs
            .pointer("/state/waiting/message")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if let Some(reason) = waiting_reason {
            if reason == "CreateContainerConfigError" || reason == "CreateContainerError" {
                signals.push(DiagnosticSignal {
                    severity: SignalSeverity::Error,
                    title: format!("Config error in container '{}'", name),
                    detail: Some(waiting_msg.to_string()),
                });
                let fallback = format!("container '{}' config error", name);
                let detail = if !waiting_msg.is_empty() {
                    waiting_msg
                } else {
                    &fallback
                };
                let summary = make_one_line_gist(reason, detail, 65);
                return DiagnosticReport {
                    verdict: DiagnosticVerdict::ConfigError,
                    summary,
                    remediation: format!(
                        "Ensure referenced config or secret exists: {}",
                        waiting_msg
                    ),
                    signals,
                };
            }

            if reason == "ImagePullBackOff"
                || reason == "ErrImagePull"
                || reason == "InvalidImageName"
            {
                signals.push(DiagnosticSignal {
                    severity: SignalSeverity::Error,
                    title: format!("Image pull failed for container '{}'", name),
                    detail: Some(waiting_msg.to_string()),
                });
                let fallback = format!("container '{}' image pull failed", name);
                let detail = if !waiting_msg.is_empty() {
                    waiting_msg
                } else {
                    &fallback
                };
                let summary = make_one_line_gist(reason, detail, 65);
                return DiagnosticReport {
                    verdict: DiagnosticVerdict::ImagePullFailed,
                    summary,
                    remediation: format!(
                        "Check image name, tag, and imagePullSecrets: {}",
                        waiting_msg
                    ),
                    signals,
                };
            }
        }

        // Check for CrashLoopBackOff or non-zero exit crashes
        let is_waiting_crashloop = waiting_reason
            .map(|r| r == "CrashLoopBackOff")
            .unwrap_or(false);

        let last_terminated = cs.pointer("/lastState/terminated");
        let last_exit_code = last_terminated
            .and_then(|t| t.get("exitCode"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let restart_count = cs.get("restartCount").and_then(|v| v.as_i64()).unwrap_or(0);

        if is_waiting_crashloop || (last_exit_code != 0 && restart_count > 0) {
            let mut panic_detail = None;
            if let Some(logs) = previous_logs {
                // Look for panic or fatal lines in the logs
                for line in logs.lines() {
                    let lower = line.to_lowercase();
                    if lower.contains("panic")
                        || lower.contains("fatal")
                        || lower.contains("exception")
                    {
                        panic_detail = Some(line.trim().to_string());
                        break;
                    }
                }
            }

            signals.push(DiagnosticSignal {
                severity: SignalSeverity::Error,
                title: format!("CrashLoopBackOff in container '{}'", name),
                detail: Some(format!(
                    "Restarts: {}. Last exit code: {}",
                    restart_count, last_exit_code
                )),
            });

            if let Some(ref detail) = panic_detail {
                signals.push(DiagnosticSignal {
                    severity: SignalSeverity::Error,
                    title: format!("Panic / Crash detected in logs for '{}'", name),
                    detail: Some(detail.clone()),
                });
            }

            let remediation = if let Some(ref p) = panic_detail {
                format!(
                    "Application crashed: \"{}\". Fix application error or configuration.",
                    p
                )
            } else {
                format!(
                    "Container '{}' failed with exit code {}. Check previous logs with [l].",
                    name, last_exit_code
                )
            };

            let summary = if let Some(ref p) = panic_detail {
                let p_clean = p.split("stack backtrace").next().unwrap_or(p).trim();
                make_one_line_gist(
                    "CrashLoopBackOff",
                    &format!("CrashLoopBackOff: container '{}' - {}", name, p_clean),
                    65,
                )
            } else {
                make_one_line_gist(
                    "CrashLoopBackOff",
                    &format!(
                        "CrashLoopBackOff: container '{}' (restarts: {})",
                        name, restart_count
                    ),
                    65,
                )
            };

            return DiagnosticReport {
                verdict: DiagnosticVerdict::CrashLoopBackOff,
                summary,
                remediation,
                signals,
            };
        }

        // Generic catch for any abnormal waiting container state (e.g. RunContainerError, PreStopHookError)
        if let Some(reason) = waiting_reason {
            if reason != "ContainerCreating" && reason != "PodInitializing" {
                signals.push(DiagnosticSignal {
                    severity: SignalSeverity::Error,
                    title: format!("Container '{}' waiting ({})", name, reason),
                    detail: if !waiting_msg.is_empty() {
                        Some(waiting_msg.to_string())
                    } else {
                        None
                    },
                });
                let fallback = format!("container '{}' waiting ({})", name, reason);
                let detail = if !waiting_msg.is_empty() {
                    waiting_msg
                } else {
                    &fallback
                };
                let summary = make_one_line_gist(reason, detail, 65);
                return DiagnosticReport {
                    verdict: DiagnosticVerdict::Failed,
                    summary,
                    remediation: format!(
                        "Inspect container '{}' configuration and kubelet logs.",
                        name
                    ),
                    signals,
                };
            }
        }
    }

    let phase = pod
        .pointer("/status/phase")
        .or_else(|| pod.get("phase"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let status_reason = pod
        .pointer("/status/reason")
        .or_else(|| pod.get("reason"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let status_message = pod
        .pointer("/status/message")
        .or_else(|| pod.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let pod_name = pod
        .pointer("/metadata/name")
        .or_else(|| pod.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("pod");

    // Check for Pod terminating (deletionTimestamp set)
    if pod.pointer("/metadata/deletionTimestamp").is_some() {
        signals.push(DiagnosticSignal {
            severity: SignalSeverity::Warning,
            title: "Pod Terminating".to_string(),
            detail: Some(format!(
                "Pod '{}' has deletionTimestamp set and is waiting for termination/finalizers",
                pod_name
            )),
        });
        return DiagnosticReport {
            verdict: DiagnosticVerdict::Failed,
            summary: make_one_line_gist(
                "Terminating",
                &format!("Terminating: pod '{}' (waiting for finalizers)", pod_name),
                65,
            ),
            remediation: "Check terminating containers, finalizers, or unmounting volumes."
                .to_string(),
            signals,
        };
    }

    // 1. Check for Evicted pod (ephemeral-storage exhaustion, node memory/disk pressure)
    if status_reason.eq_ignore_ascii_case("Evicted")
        || status_message.to_lowercase().contains("evicted")
        || (phase.eq_ignore_ascii_case("Failed")
            && status_message.to_lowercase().contains("ephemeral-storage"))
    {
        let cause = if status_message.to_lowercase().contains("ephemeral-storage") {
            "node ephemeral-storage exhaustion"
        } else if status_message.to_lowercase().contains("memory") {
            "node memory pressure"
        } else if status_message.to_lowercase().contains("disk") {
            "node disk pressure"
        } else if status_message.to_lowercase().contains("pid") {
            "node PID pressure"
        } else {
            "node resource pressure"
        };

        signals.push(DiagnosticSignal {
            severity: SignalSeverity::Error,
            title: format!("Kubelet Eviction ({})", cause),
            detail: if !status_message.is_empty() {
                Some(status_message.to_string())
            } else {
                Some(format!("Pod was evicted by kubelet due to {}", cause))
            },
        });

        // Check if container was terminated with exit code during eviction
        for cs in regular_statuses {
            let c_name = cs
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("container");
            let exit_code = cs
                .pointer("/lastState/terminated/exitCode")
                .or_else(|| cs.pointer("/state/terminated/exitCode"))
                .and_then(|v| v.as_i64());
            if let Some(code) = exit_code {
                let desc = if code == 143 {
                    "Terminated with SIGTERM (exit code 143) by kubelet during eviction".to_string()
                } else {
                    format!("Container '{}' exited with code {}", c_name, code)
                };
                signals.push(DiagnosticSignal {
                    severity: SignalSeverity::Info,
                    title: format!("Container '{}' Termination", c_name),
                    detail: Some(desc),
                });
            }
        }

        let remediation = if cause.contains("ephemeral-storage") {
            "Pod evicted due to node ephemeral-storage exhaustion. Increase ephemeral-storage limits or clean up disk.".to_string()
        } else if cause.contains("memory") {
            "Pod evicted due to node memory pressure. Check node allocatable memory and pod memory limits.".to_string()
        } else {
            "Pod was evicted due to node resource pressure. Adjust resource limits or clean up node.".to_string()
        };

        return DiagnosticReport {
            verdict: DiagnosticVerdict::Evicted,
            summary: make_one_line_gist("Evicted", cause, 65),
            remediation,
            signals,
        };
    }

    // 2. Check for Unknown phase (node lost / kubelet unresponsive)
    if phase.eq_ignore_ascii_case("Unknown") {
        signals.push(DiagnosticSignal {
            severity: SignalSeverity::Warning,
            title: "Node Lost / Kubelet Unresponsive".to_string(),
            detail: Some("Pod is in Unknown phase. Kubelet stopped posting status.".to_string()),
        });
        return DiagnosticReport {
            verdict: DiagnosticVerdict::NodeLost,
            summary: make_one_line_gist(
                "NodeLost",
                &format!("Pod '{}' in Unknown phase", pod_name),
                65,
            ),
            remediation: "Check node health, network connectivity, or kubelet service.".to_string(),
            signals,
        };
    }

    // Check top-level waitingReason (e.g. from PodSummary or synthetic views)
    let top_waiting_reason = pod
        .get("waitingReason")
        .or_else(|| pod.get("waiting_reason"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

    if let Some(reason) = top_waiting_reason {
        let name = pod.get("name").and_then(|v| v.as_str()).unwrap_or("pod");
        let event_msg = events.iter().find_map(|e| {
            let r = e.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            if r == "Failed" || r == "FailedMount" || r == reason {
                e.get("message").and_then(|v| v.as_str())
            } else {
                None
            }
        });

        if reason == "CreateContainerConfigError" || reason == "CreateContainerError" {
            let detail = event_msg.map(|s| s.to_string());
            let remediation = if let Some(ref d) = detail {
                format!("Ensure referenced config or secret exists: {}", d)
            } else {
                "Ensure referenced ConfigMap or Secret exists and is accessible in the namespace."
                    .to_string()
            };
            signals.push(DiagnosticSignal {
                severity: SignalSeverity::Error,
                title: format!("Container config error ({})", reason),
                detail,
            });
            let fallback = format!("{}: pod '{}' config error", reason, name);
            let summary = make_one_line_gist(reason, event_msg.unwrap_or(&fallback), 65);
            return DiagnosticReport {
                verdict: DiagnosticVerdict::ConfigError,
                summary,
                remediation,
                signals,
            };
        }

        if reason == "ImagePullBackOff" || reason == "ErrImagePull" || reason == "InvalidImageName"
        {
            let detail = event_msg.map(|s| s.to_string());
            let remediation = if let Some(ref d) = detail {
                format!("Check image repository, tag, and imagePullSecrets: {}", d)
            } else {
                "Check image repository, tag, and imagePullSecrets.".to_string()
            };
            signals.push(DiagnosticSignal {
                severity: SignalSeverity::Error,
                title: format!("Image pull failure ({})", reason),
                detail,
            });
            let fallback = format!("{}: pod '{}' image pull failed", reason, name);
            let summary = make_one_line_gist(reason, event_msg.unwrap_or(&fallback), 65);
            return DiagnosticReport {
                verdict: DiagnosticVerdict::ImagePullFailed,
                summary,
                remediation,
                signals,
            };
        }

        if reason == "CrashLoopBackOff" {
            signals.push(DiagnosticSignal {
                severity: SignalSeverity::Error,
                title: format!("CrashLoopBackOff ({})", reason),
                detail: event_msg.map(|s| s.to_string()),
            });
            let fallback = format!("{}: pod '{}' crashing", reason, name);
            let summary = make_one_line_gist(reason, event_msg.unwrap_or(&fallback), 65);
            return DiagnosticReport {
                verdict: DiagnosticVerdict::CrashLoopBackOff,
                summary,
                remediation:
                    "Check previous container logs with [l] or events for failure details."
                        .to_string(),
                signals,
            };
        }
    }

    // Check for Scheduling failures in conditions or events
    let unschedulable_cond = pod
        .pointer("/status/conditions")
        .and_then(|v| v.as_array())
        .and_then(|conds| {
            conds.iter().find(|c| {
                let c_type = c.get("type").and_then(|v| v.as_str());
                let status = c.get("status").and_then(|v| v.as_str());
                let reason = c.get("reason").and_then(|v| v.as_str());
                c_type == Some("PodScheduled")
                    && (status == Some("False") || reason == Some("Unschedulable"))
            })
        });

    let failed_scheduling_event = events
        .iter()
        .find(|e| e.get("reason").and_then(|v| v.as_str()) == Some("FailedScheduling"));

    if let Some(cond) = unschedulable_cond {
        let msg = cond
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Pod is unschedulable");
        let (summary, sched_signals) = parse_scheduling_failure(msg);
        signals.extend(sched_signals);
        return DiagnosticReport {
            verdict: DiagnosticVerdict::SchedulingFailed,
            summary,
            remediation: "Check node capacity or adjust resource requests / tolerations.".into(),
            signals,
        };
    } else if let Some(evt) = failed_scheduling_event {
        let msg = evt
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("FailedScheduling event recorded");
        let (summary, sched_signals) = parse_scheduling_failure(msg);
        signals.extend(sched_signals);
        return DiagnosticReport {
            verdict: DiagnosticVerdict::SchedulingFailed,
            summary,
            remediation: "Check node capacity or adjust resource requests / tolerations.".into(),
            signals,
        };
    }

    // Check events for config errors or image errors
    for evt in events {
        let reason = evt.get("reason").and_then(|v| v.as_str()).unwrap_or("");
        let msg = evt.get("message").and_then(|v| v.as_str()).unwrap_or("");
        let evt_type = evt.get("type").and_then(|v| v.as_str()).unwrap_or("");

        if reason == "FailedMount"
            || (reason == "Failed"
                && (msg.to_lowercase().contains("secret")
                    || msg.to_lowercase().contains("configmap")))
        {
            signals.push(DiagnosticSignal {
                severity: SignalSeverity::Error,
                title: format!("Event: {}", reason),
                detail: Some(msg.to_string()),
            });
            let summary = make_one_line_gist(reason, msg, 65);
            return DiagnosticReport {
                verdict: DiagnosticVerdict::ConfigError,
                summary,
                remediation: format!(
                    "Ensure referenced volume, Secret, or ConfigMap exists: {}",
                    msg
                ),
                signals,
            };
        }

        if reason == "FailedCreatePodSandBox" || reason == "NetworkNotReady" {
            signals.push(DiagnosticSignal {
                severity: SignalSeverity::Error,
                title: format!("Sandbox / Network Failure ({})", reason),
                detail: Some(msg.to_string()),
            });
            let summary = make_one_line_gist(reason, msg, 65);
            return DiagnosticReport {
                verdict: DiagnosticVerdict::Failed,
                summary,
                remediation:
                    "Check CNI network plugin, node IPAM subnet address capacity, or kubelet logs."
                        .to_string(),
                signals,
            };
        }

        if reason == "Failed"
            && (msg.contains("pull") || msg.contains("image") || msg.contains("ErrImagePull"))
        {
            signals.push(DiagnosticSignal {
                severity: SignalSeverity::Error,
                title: format!("Event: {}", reason),
                detail: Some(msg.to_string()),
            });
            let summary = make_one_line_gist("ImagePullFailed", msg, 65);
            return DiagnosticReport {
                verdict: DiagnosticVerdict::ImagePullFailed,
                summary,
                remediation: format!("Verify image name, tag, and imagePullSecrets: {}", msg),
                signals,
            };
        }

        if evt_type == "Warning" && reason != "FailedScheduling" {
            signals.push(DiagnosticSignal {
                severity: SignalSeverity::Warning,
                title: format!("Event: {}", reason),
                detail: Some(msg.to_string()),
            });
        }
    }

    if phase.eq_ignore_ascii_case("Pending") {
        if let Some(warn) = events
            .iter()
            .find(|e| e.get("type").and_then(|v| v.as_str()) == Some("Warning"))
        {
            let reason = warn
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("Warning");
            let msg = warn.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let summary = make_one_line_gist(reason, msg, 65);
            return DiagnosticReport {
                verdict: DiagnosticVerdict::Failed,
                summary,
                remediation: "Inspect correlated warning events and node/pod status.".to_string(),
                signals,
            };
        }
        return DiagnosticReport {
            verdict: DiagnosticVerdict::SchedulingFailed,
            summary: make_one_line_gist(
                "Pending",
                &format!("Pod '{}' waiting for scheduling/admission", pod_name),
                65,
            ),
            remediation: "Inspect node capacity, taints, tolerations, or cluster scheduler logs."
                .to_string(),
            signals,
        };
    }

    if phase.eq_ignore_ascii_case("Failed") {
        let exit_code = regular_statuses.iter().find_map(|cs| {
            cs.pointer("/lastState/terminated/exitCode")
                .or_else(|| cs.pointer("/state/terminated/exitCode"))
                .and_then(|v| v.as_i64())
        });

        let term_reason = regular_statuses
            .iter()
            .find_map(|cs| {
                cs.pointer("/lastState/terminated/reason")
                    .or_else(|| cs.pointer("/state/terminated/reason"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or(status_reason);

        let cat = if !status_reason.is_empty() {
            status_reason
        } else if !term_reason.is_empty() {
            term_reason
        } else {
            "Failed"
        };

        let summary = if !status_message.is_empty() {
            make_one_line_gist(cat, status_message, 65)
        } else if let Some(code) = exit_code {
            if code == 143 {
                "Failed: terminated with SIGTERM (exit code 143)".to_string()
            } else {
                format!(
                    "Failed: container exited with code {} (exit code {})",
                    code, code
                )
            }
        } else {
            make_one_line_gist(
                cat,
                &format!("Pod '{}' terminated in Failed phase", pod_name),
                65,
            )
        };

        signals.push(DiagnosticSignal {
            severity: SignalSeverity::Error,
            title: format!(
                "Pod Failed ({})",
                if !term_reason.is_empty() {
                    term_reason
                } else {
                    "Error"
                }
            ),
            detail: if !status_message.is_empty() {
                Some(status_message.to_string())
            } else {
                exit_code.map(|c| format!("Exit code: {}", c))
            },
        });

        return DiagnosticReport {
            verdict: DiagnosticVerdict::Failed,
            summary,
            remediation: "Check application logs, exit code, or pod restartPolicy.".to_string(),
            signals,
        };
    }

    if phase.eq_ignore_ascii_case("Running") {
        DiagnosticReport {
            verdict: DiagnosticVerdict::Healthy,
            summary: "Pod is running and healthy".into(),
            remediation: String::new(),
            signals,
        }
    } else if phase.eq_ignore_ascii_case("Succeeded") {
        DiagnosticReport {
            verdict: DiagnosticVerdict::Healthy,
            summary: format!("Pod '{}' completed successfully", pod_name),
            remediation: String::new(),
            signals,
        }
    } else {
        DiagnosticReport {
            verdict: DiagnosticVerdict::Failed,
            summary: make_one_line_gist(
                if phase.is_empty() { "Unknown" } else { phase },
                &format!("Pod '{}' in non-running phase", pod_name),
                65,
            ),
            remediation: "Inspect pod status, events, and kubelet logs.".to_string(),
            signals,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_diagnose_oom_killed_pod() {
        let pod = json!({
            "metadata": {
                "name": "payment-service-5f87bc-k29x",
                "namespace": "prod",
            },
            "spec": {
                "containers": [{
                    "name": "payment-api",
                    "resources": {
                        "limits": { "memory": "256Mi" },
                        "requests": { "memory": "128Mi" }
                    }
                }]
            },
            "status": {
                "phase": "Running",
                "containerStatuses": [{
                    "name": "payment-api",
                    "restartCount": 4,
                    "ready": false,
                    "lastState": {
                        "terminated": {
                            "exitCode": 137,
                            "reason": "OOMKilled",
                            "finishedAt": "2026-09-20T12:00:00Z"
                        }
                    }
                }]
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_eq!(report.verdict, DiagnosticVerdict::OOMKilled);
        assert!(report.summary.contains("OOMKilled"));
        assert!(report.summary.contains("payment-api"));
        assert!(report.remediation.contains("memory limit"));
        assert!(report.remediation.contains("256Mi"));
        assert!(!report.signals.is_empty());
    }

    #[test]
    fn test_diagnose_crashloop_panic() {
        let pod = json!({
            "metadata": {
                "name": "auth-svc-789-xyz",
                "namespace": "prod",
            },
            "spec": {
                "containers": [{
                    "name": "auth",
                }]
            },
            "status": {
                "containerStatuses": [{
                    "name": "auth",
                    "restartCount": 5,
                    "ready": false,
                    "state": {
                        "waiting": {
                            "reason": "CrashLoopBackOff",
                            "message": "back-off 5m0s restarting failed container"
                        }
                    },
                    "lastState": {
                        "terminated": {
                            "exitCode": 1,
                            "reason": "Error"
                        }
                    }
                }]
            }
        });

        let logs = "2026-09-20T12:00:01Z [INFO] Server starting\nthread 'main' panicked at 'DatabaseConnectionRefused(\"tcp://db:5432\")', src/main.rs:42\nstack backtrace:\n";

        let report = analyze_pod_health(&pod, &[], Some(logs));
        assert_eq!(report.verdict, DiagnosticVerdict::CrashLoopBackOff);
        assert!(report.summary.contains("CrashLoopBackOff"));
        assert!(report.summary.contains("auth"));
        // Check that panic line was captured in signals
        let panic_signal = report.signals.iter().find(|s| s.title.contains("Panic"));
        assert!(panic_signal.is_some());
        let sig = panic_signal.unwrap();
        assert!(sig
            .detail
            .as_ref()
            .unwrap()
            .contains("DatabaseConnectionRefused"));
    }

    #[test]
    fn test_diagnose_config_error() {
        let pod = json!({
            "metadata": { "name": "web-123", "namespace": "prod" },
            "status": {
                "containerStatuses": [{
                    "name": "web",
                    "ready": false,
                    "state": {
                        "waiting": {
                            "reason": "CreateContainerConfigError",
                            "message": "secret \"db-creds\" not found"
                        }
                    }
                }]
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_eq!(report.verdict, DiagnosticVerdict::ConfigError);
        assert!(report.summary.contains("CreateContainerConfigError"));
        assert!(report.remediation.contains("db-creds"));
    }

    #[test]
    fn test_diagnose_image_pull_error() {
        let pod = json!({
            "metadata": { "name": "worker-1", "namespace": "prod" },
            "status": {
                "containerStatuses": [{
                    "name": "worker",
                    "ready": false,
                    "state": {
                        "waiting": {
                            "reason": "ImagePullBackOff",
                            "message": "Back-off pulling image \"myregistry.io/app:v2.0\""
                        }
                    }
                }]
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_eq!(report.verdict, DiagnosticVerdict::ImagePullFailed);
        assert!(report.summary.contains("ImagePullBackOff"));
        assert!(report.remediation.contains("myregistry.io/app:v2.0"));
    }

    #[test]
    fn test_diagnose_scheduling_failed() {
        let pod = json!({
            "metadata": { "name": "heavy-job", "namespace": "default" },
            "status": {
                "phase": "Pending",
                "conditions": [{
                    "type": "PodScheduled",
                    "status": "False",
                    "reason": "Unschedulable",
                    "message": "0/3 nodes are available: 3 Insufficient memory."
                }]
            }
        });

        let events = vec![json!({
            "type": "Warning",
            "reason": "FailedScheduling",
            "message": "0/3 nodes are available: 3 Insufficient memory."
        })];

        let report = analyze_pod_health(&pod, &events, None);
        assert_eq!(report.verdict, DiagnosticVerdict::SchedulingFailed);
        assert!(
            report.summary.contains("Insufficient memory")
                || report.summary.contains("Unschedulable")
        );
        assert!(
            report.remediation.contains("node capacity") || report.remediation.contains("requests")
        );
    }

    #[test]
    fn test_diagnose_pod_summary_shape() {
        let summary = json!({
            "name": "missing-secret-demo-8548b65c5-bmx2r",
            "namespace": "default",
            "phase": "Pending",
            "ready": "0/1",
            "restarts": 0,
            "waitingReason": "CreateContainerConfigError"
        });

        let report = analyze_pod_health(&summary, &[], None);
        assert_eq!(report.verdict, DiagnosticVerdict::ConfigError);
        assert!(report.summary.contains("CreateContainerConfigError"));
        assert!(!report.signals.is_empty());
    }

    #[test]
    fn test_diagnose_pod_summary_with_missing_secret_event() {
        let summary = json!({
            "name": "missing-secret-demo-8548b65c5-bmx2r",
            "namespace": "default",
            "phase": "Pending",
            "ready": "0/1",
            "restarts": 0,
            "waitingReason": "CreateContainerConfigError"
        });

        let events = vec![json!({
            "type": "Warning",
            "reason": "Failed",
            "message": "Error: secret \"intentionally-missing-secret\" not found"
        })];

        let report = analyze_pod_health(&summary, &events, None);
        assert_eq!(report.verdict, DiagnosticVerdict::ConfigError);
        assert!(report.remediation.contains("intentionally-missing-secret"));
        assert_eq!(report.signals.len(), 1);
        assert_eq!(
            report.signals[0].detail.as_deref(),
            Some("Error: secret \"intentionally-missing-secret\" not found")
        );
    }

    #[test]
    fn test_diagnose_evicted_pod() {
        let pod = json!({
            "metadata": { "name": "matchbox-app-preview-1014-6596698bd6-4tv8p", "namespace": "matching" },
            "status": {
                "phase": "Failed",
                "reason": "Evicted",
                "message": "The node was low on resource: ephemeral-storage. Container matchbox was using 524288000B.",
                "containerStatuses": [{
                    "name": "matchbox",
                    "ready": false,
                    "lastState": {
                        "terminated": {
                            "exitCode": 143,
                            "reason": "ContainerCannotRun"
                        }
                    }
                }]
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_eq!(report.verdict, DiagnosticVerdict::Evicted);
        assert!(report.summary.contains("Evicted"));
        assert!(
            report.remediation.contains("ephemeral-storage")
                || report.remediation.contains("evicted")
        );
        assert!(!report.signals.is_empty());
    }

    #[test]
    fn test_diagnose_failed_pod_with_exit_code() {
        let pod = json!({
            "metadata": { "name": "batch-job-fail-x9", "namespace": "default" },
            "status": {
                "phase": "Failed",
                "containerStatuses": [{
                    "name": "task",
                    "ready": false,
                    "lastState": {
                        "terminated": {
                            "exitCode": 1,
                            "reason": "Error"
                        }
                    }
                }]
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_eq!(report.verdict, DiagnosticVerdict::Failed);
        assert!(report.summary.contains("Failed") || report.summary.contains("exit code 1"));
    }

    #[test]
    fn test_diagnose_unknown_phase() {
        let pod = json!({
            "metadata": { "name": "orphan-pod", "namespace": "default" },
            "status": {
                "phase": "Unknown"
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_eq!(report.verdict, DiagnosticVerdict::NodeLost);
    }

    #[test]
    fn test_diagnose_scheduling_gist_and_breakdown() {
        let pod = json!({
            "metadata": { "name": "virt-launcher-testvm-xb72f", "namespace": "kubevirt" },
            "status": {
                "phase": "Pending",
                "conditions": [{
                    "type": "PodScheduled",
                    "status": "False",
                    "reason": "Unschedulable",
                    "message": "0/1 nodes are available: 1 Insufficient devices.kubevirt.io/kvm, 1 Too many pods. no new claims to deallocate, preemption: 0/1 nodes are available: 1 Preemption is not helpful for scheduling."
                }]
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_eq!(report.verdict, DiagnosticVerdict::SchedulingFailed);
        assert!(report.summary.len() < 80);
        assert!(report.summary.contains("0/1 nodes"));
        assert!(report.signals.len() >= 2);
        assert!(report
            .signals
            .iter()
            .any(|s| s.title.contains("Insufficient")
                || s.detail
                    .as_deref()
                    .unwrap_or("")
                    .contains("devices.kubevirt.io/kvm")));
        assert!(report
            .signals
            .iter()
            .any(|s| s.title.contains("Pod Capacity")
                || s.detail.as_deref().unwrap_or("").contains("Too many pods")));
    }

    #[test]
    fn test_diagnose_eviction_gist_and_breakdown() {
        let pod = json!({
            "metadata": { "name": "matchbox-app-preview-1014-6596698bd6-4tv8p", "namespace": "matching" },
            "status": {
                "phase": "Failed",
                "reason": "Evicted",
                "message": "The node was low on resource: ephemeral-storage. Container matchbox was using 524288000B.",
                "containerStatuses": [{
                    "name": "matchbox",
                    "ready": false,
                    "lastState": {
                        "terminated": {
                            "exitCode": 143,
                            "reason": "ContainerCannotRun"
                        }
                    }
                }]
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_eq!(report.verdict, DiagnosticVerdict::Evicted);
        assert!(report.summary.len() < 80);
        assert_eq!(report.summary, "Evicted: node ephemeral-storage exhaustion");
        assert!(report.signals.iter().any(|s| s
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("524288000B")));
        assert!(report
            .signals
            .iter()
            .any(|s| s.title.contains("Termination")
                || s.detail.as_deref().unwrap_or("").contains("143")));
    }

    #[test]
    fn test_diagnose_unexpected_admission_error_gpu() {
        let pod = json!({
            "metadata": { "name": "gpu-pod", "namespace": "gpu-operator" },
            "status": {
                "phase": "Failed",
                "reason": "UnexpectedAdmissionError",
                "message": "Pod was rejected: Allocate failed due to no healthy devices present; cannot allocate unhealthy devices nvidia.com/gpu, which is unexpected"
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_eq!(report.verdict, DiagnosticVerdict::Failed);
        assert_eq!(
            report.summary,
            "Allocate failed: No healthy nvidia.com/gpu devices"
        );
        assert!(!report.summary.ends_with('…'));
        assert!(!report.summary.ends_with("..."));
        // Full raw detail is preserved in signals
        assert_eq!(report.signals.len(), 1);
        assert!(report.signals[0]
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("cannot allocate unhealthy devices nvidia.com/gpu"));
    }

    #[test]
    fn test_diagnose_cni_sandbox_failure() {
        let pod = json!({
            "metadata": { "name": "web-worker", "namespace": "prod" },
            "status": {
                "phase": "Pending"
            }
        });
        let events = vec![json!({
            "type": "Warning",
            "reason": "FailedCreatePodSandBox",
            "message": "Failed to create pod sandbox: rpc error: code = Unknown desc = failed to setup network for sandbox: no IP addresses available in network: podnet"
        })];

        let report = analyze_pod_health(&pod, &events, None);
        assert_eq!(
            report.summary,
            "Network setup failed: No IP addresses in podnet"
        );
        assert!(!report.summary.ends_with('…'));
        assert!(report.signals.iter().any(|s| s
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("no IP addresses available")));
    }

    #[test]
    fn test_diagnose_failed_mount_storage() {
        let pod = json!({
            "metadata": { "name": "db-follower", "namespace": "prod" },
            "status": {
                "phase": "Pending"
            }
        });
        let events = vec![json!({
            "type": "Warning",
            "reason": "FailedMount",
            "message": "MountVolume.SetUp failed for volume \"pvc-data\": persistentvolumeclaim \"pvc-data\" not found"
        })];

        let report = analyze_pod_health(&pod, &events, None);
        assert_eq!(report.verdict, DiagnosticVerdict::ConfigError);
        assert_eq!(report.summary, "Mount failed: PVC \"pvc-data\" not found");
        assert!(!report.summary.ends_with('…'));
        assert!(report.signals.iter().any(|s| s
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("persistentvolumeclaim \"pvc-data\" not found")));
    }

    #[test]
    fn test_diagnose_deadline_exceeded() {
        let pod = json!({
            "metadata": { "name": "batch-processor-job-4k9", "namespace": "batch" },
            "status": {
                "phase": "Failed",
                "reason": "DeadlineExceeded",
                "message": "Pod was active on the node longer than the specified deadline 3600 seconds"
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_eq!(report.verdict, DiagnosticVerdict::Failed);
        assert!(
            report.summary.len() <= 75,
            "Summary too long: {} chars",
            report.summary.len()
        );
        assert!(report.summary.contains("DeadlineExceeded"));
        assert!(report.signals.iter().any(|s| s
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("3600 seconds")));
    }

    #[test]
    fn test_diagnose_init_container_failure() {
        let pod = json!({
            "metadata": { "name": "app-with-init", "namespace": "default" },
            "status": {
                "phase": "Pending",
                "initContainerStatuses": [{
                    "name": "db-migrate",
                    "ready": false,
                    "restartCount": 3,
                    "state": {
                        "waiting": {
                            "reason": "CrashLoopBackOff",
                            "message": "back-off 10s restarting failed container"
                        }
                    },
                    "lastState": {
                        "terminated": {
                            "exitCode": 2,
                            "reason": "Error"
                        }
                    }
                }],
                "containerStatuses": [{
                    "name": "main-app",
                    "ready": false,
                    "state": {
                        "waiting": {
                            "reason": "PodInitializing"
                        }
                    }
                }]
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_eq!(report.verdict, DiagnosticVerdict::CrashLoopBackOff);
        assert!(report.summary.len() <= 75);
        assert!(report.summary.contains("db-migrate"));
    }

    #[test]
    fn test_diagnose_terminating_pod() {
        let pod = json!({
            "metadata": {
                "name": "stuck-terminating-pod",
                "namespace": "default",
                "deletionTimestamp": "2026-09-20T12:00:00Z"
            },
            "status": {
                "phase": "Running"
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_eq!(report.verdict, DiagnosticVerdict::Failed);
        assert!(report.summary.len() <= 75);
        assert!(report.summary.contains("Terminating"));
    }

    #[test]
    fn test_diagnose_pending_without_events_never_healthy() {
        let pod = json!({
            "metadata": { "name": "fresh-pending-pod", "namespace": "default" },
            "status": {
                "phase": "Pending"
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_ne!(report.verdict, DiagnosticVerdict::Healthy);
        assert_eq!(report.verdict, DiagnosticVerdict::SchedulingFailed);
        assert!(report.summary.len() <= 75);
        assert!(report.summary.contains("Pending"));
    }

    #[test]
    fn test_diagnose_succeeded_pod_healthy() {
        let pod = json!({
            "metadata": { "name": "backup-cronjob-2894-xx9", "namespace": "default" },
            "status": {
                "phase": "Succeeded"
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_eq!(report.verdict, DiagnosticVerdict::Healthy);
        assert!(report.summary.contains("completed successfully"));
    }

    #[test]
    fn test_diagnose_custom_phase_never_healthy() {
        let pod = json!({
            "metadata": { "name": "custom-state-pod", "namespace": "default" },
            "status": {
                "phase": "CustomErrorPhase"
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_ne!(report.verdict, DiagnosticVerdict::Healthy);
        assert_eq!(report.verdict, DiagnosticVerdict::Failed);
        assert!(report.summary.len() <= 75);
    }

    #[test]
    fn test_diagnose_generic_run_container_error() {
        let pod = json!({
            "metadata": { "name": "bad-caps-pod", "namespace": "default" },
            "status": {
                "phase": "Running",
                "containerStatuses": [{
                    "name": "worker",
                    "ready": false,
                    "state": {
                        "waiting": {
                            "reason": "RunContainerError",
                            "message": "failed to create containerd task: failed to create shim task: OCI runtime create failed: container_linux.go:380: starting container process caused: permission denied"
                        }
                    }
                }]
            }
        });

        let report = analyze_pod_health(&pod, &[], None);
        assert_eq!(report.verdict, DiagnosticVerdict::Failed);
        assert!(
            report.summary.len() <= 75,
            "Summary length {} > 75",
            report.summary.len()
        );
        assert!(
            report.summary.contains("RunContainerError")
                || report.summary.contains("permission denied")
        );
        assert!(report.signals.iter().any(|s| s
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("OCI runtime create failed")));
    }
}
