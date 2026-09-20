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

pub fn analyze_pod_health(
    pod: &Value,
    events: &[Value],
    previous_logs: Option<&str>,
) -> DiagnosticReport {
    let mut signals = Vec::new();

    // Check container statuses for OOMKilled
    let container_statuses = pod
        .pointer("/status/containerStatuses")
        .and_then(|v| v.as_array());

    if let Some(statuses) = container_statuses {
        for cs in statuses {
            let name = cs.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
            let terminated = cs
                .pointer("/lastState/terminated")
                .or_else(|| cs.pointer("/state/terminated"));

            if let Some(term) = terminated {
                let reason = term.get("reason").and_then(|v| v.as_str()).unwrap_or("");
                let exit_code = term.get("exitCode").and_then(|v| v.as_i64()).unwrap_or(0);

                if reason == "OOMKilled" || exit_code == 137 {
                    // Look up container spec for limits
                    let memory_limit = pod
                        .pointer("/spec/containers")
                        .and_then(|v| v.as_array())
                        .and_then(|containers| {
                            containers
                                .iter()
                                .find(|c| c.get("name").and_then(|n| n.as_str()) == Some(name))
                        })
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

                    return DiagnosticReport {
                        verdict: DiagnosticVerdict::OOMKilled,
                        summary: format!("Container '{}' was OOMKilled (exit code 137)", name),
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
                    return DiagnosticReport {
                        verdict: DiagnosticVerdict::ConfigError,
                        summary: format!("Container '{}' failed with {}", name, reason),
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
                    return DiagnosticReport {
                        verdict: DiagnosticVerdict::ImagePullFailed,
                        summary: format!("Container '{}' failed with {}", name, reason),
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

                return DiagnosticReport {
                    verdict: DiagnosticVerdict::CrashLoopBackOff,
                    summary: format!(
                        "Container '{}' is in CrashLoopBackOff (restarts: {})",
                        name, restart_count
                    ),
                    remediation,
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

    // 1. Check for Evicted pod (ephemeral-storage exhaustion, node memory/disk pressure)
    if status_reason.eq_ignore_ascii_case("Evicted")
        || status_message.to_lowercase().contains("evicted")
        || (phase.eq_ignore_ascii_case("Failed")
            && status_message.to_lowercase().contains("ephemeral-storage"))
    {
        signals.push(DiagnosticSignal {
            severity: SignalSeverity::Error,
            title: "Pod Evicted by kubelet".to_string(),
            detail: if !status_message.is_empty() {
                Some(status_message.to_string())
            } else {
                None
            },
        });

        let remediation = if status_message.to_lowercase().contains("ephemeral-storage") {
            "Pod evicted due to node ephemeral-storage exhaustion. Increase ephemeral-storage limits or clean up disk.".to_string()
        } else if status_message.to_lowercase().contains("memory") {
            "Pod evicted due to node memory pressure. Check node allocatable memory and pod memory limits.".to_string()
        } else {
            "Pod was evicted due to node resource pressure. Adjust resource limits or clean up node.".to_string()
        };

        return DiagnosticReport {
            verdict: DiagnosticVerdict::Evicted,
            summary: if !status_message.is_empty() {
                format!("Pod Evicted: {}", status_message)
            } else {
                format!("Pod '{}' was Evicted by kubelet", pod_name)
            },
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
            summary: format!("Pod '{}' is in Unknown phase", pod_name),
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
            return DiagnosticReport {
                verdict: DiagnosticVerdict::ConfigError,
                summary: format!("Pod '{}' failed with {}", name, reason),
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
            return DiagnosticReport {
                verdict: DiagnosticVerdict::ImagePullFailed,
                summary: format!("Pod '{}' failed with {}", name, reason),
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
            return DiagnosticReport {
                verdict: DiagnosticVerdict::CrashLoopBackOff,
                summary: format!("Pod '{}' is in CrashLoopBackOff", name),
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
        signals.push(DiagnosticSignal {
            severity: SignalSeverity::Warning,
            title: "Pod is Unschedulable".into(),
            detail: Some(msg.to_string()),
        });
        return DiagnosticReport {
            verdict: DiagnosticVerdict::SchedulingFailed,
            summary: format!("Scheduling failed: {}", msg),
            remediation: "Check node capacity or adjust resource requests / tolerations.".into(),
            signals,
        };
    } else if let Some(evt) = failed_scheduling_event {
        let msg = evt
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("FailedScheduling event recorded");
        signals.push(DiagnosticSignal {
            severity: SignalSeverity::Warning,
            title: "FailedScheduling event".into(),
            detail: Some(msg.to_string()),
        });
        return DiagnosticReport {
            verdict: DiagnosticVerdict::SchedulingFailed,
            summary: format!("Scheduling failed: {}", msg),
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
            return DiagnosticReport {
                verdict: DiagnosticVerdict::ConfigError,
                summary: format!("Configuration / volume mount error: {}", msg),
                remediation: format!(
                    "Ensure referenced volume, Secret, or ConfigMap exists: {}",
                    msg
                ),
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
            return DiagnosticReport {
                verdict: DiagnosticVerdict::ImagePullFailed,
                summary: format!("Image error: {}", msg),
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

    if phase.eq_ignore_ascii_case("Failed") {
        let exit_code = container_statuses.and_then(|statuses| {
            statuses.iter().find_map(|cs| {
                cs.pointer("/lastState/terminated/exitCode")
                    .or_else(|| cs.pointer("/state/terminated/exitCode"))
                    .and_then(|v| v.as_i64())
            })
        });

        let term_reason = container_statuses
            .and_then(|statuses| {
                statuses.iter().find_map(|cs| {
                    cs.pointer("/lastState/terminated/reason")
                        .or_else(|| cs.pointer("/state/terminated/reason"))
                        .and_then(|v| v.as_str())
                })
            })
            .unwrap_or(status_reason);

        let summary = if let Some(code) = exit_code {
            if code == 143 {
                "Pod Failed: terminated with SIGTERM (exit code 143)".to_string()
            } else {
                format!(
                    "Pod Failed: container exited with code {} (exit code {})",
                    code, code
                )
            }
        } else if !status_message.is_empty() {
            format!("Pod Failed: {}", status_message)
        } else {
            format!("Pod '{}' terminated in Failed phase", pod_name)
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

    DiagnosticReport {
        verdict: DiagnosticVerdict::Healthy,
        summary: "Pod is healthy or has no detectable failures".into(),
        remediation: String::new(),
        signals,
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
}
