//! The `k8s.whyNotReady` capability.
//!
//! Assembles structured root-cause evidence for unready, crash-looping, or failing pods
//! (container states, exit codes, failing probes, warning events, and capped log tail).

use std::collections::BTreeSet;
use std::sync::Arc;

use k8s_openapi::api::core::v1::{ContainerStatus, Event, Pod};
use kube::api::{ListParams, LogParams};
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;
use crate::events::{event_first_timestamp, event_last_timestamp};

const MAX_LOG_TAIL_BYTES: usize = 4 * 1024; // 4 KiB max
const DEFAULT_TAIL_LINES: i64 = 50;
const MAX_TAIL_LINES: i64 = 200;

/// Input arguments for `k8s.whyNotReady`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WhyNotReadyIn {
    pub context: String,
    pub namespace: String,
    pub pod: String,
    #[serde(rename = "containerName", default)]
    pub container_name: Option<String>,
    #[serde(rename = "tailLines", default)]
    pub tail_lines: Option<i64>,
}

/// Output evidence for `k8s.whyNotReady`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WhyNotReadyOut {
    pub pod: String,
    pub namespace: String,
    pub phase: String,
    pub init_containers: Vec<ContainerEvidence>,
    pub app_containers: Vec<ContainerEvidence>,
    pub previous_logs_tail: Option<String>,
    pub current_logs_tail: Option<String>,
    pub recent_warning_events: Vec<CompactEventSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContainerEvidence {
    pub name: String,
    pub restart_count: i32,
    pub current_state: ContainerStateSummary,
    pub last_terminated_state: Option<TerminatedStateSummary>,
    pub failing_probes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContainerStateSummary {
    pub state: String, // "waiting" | "running" | "terminated" | "unknown"
    pub reason: Option<String>,
    pub message: Option<String>,
    pub exit_code: Option<i32>,
    pub started_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TerminatedStateSummary {
    pub exit_code: i32,
    pub signal: Option<i32>,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub oom_killed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CompactEventSummary {
    #[serde(rename = "type")]
    pub type_: String,
    pub reason: String,
    pub message: String,
    pub count: i32,
    pub first_seen: Option<String>,
    pub last_seen: Option<String>,
}

/// Cap log tail to `max_bytes`, keeping valid UTF-8 boundaries and preferring a newline start.
pub fn cap_log_tail(logs: &str, max_bytes: usize) -> String {
    if logs.len() <= max_bytes {
        return logs.to_string();
    }
    let cut = logs.len() - max_bytes;
    let mut boundary = cut;
    while boundary < logs.len() && !logs.is_char_boundary(boundary) {
        boundary += 1;
    }
    // If boundary lands in the middle of a line, advance to the next newline to avoid a chopped line
    if boundary > 0 && logs.as_bytes().get(boundary - 1) != Some(&b'\n') {
        if let Some(nl) = logs[boundary..].find('\n') {
            return logs[boundary + nl + 1..].to_string();
        }
    }
    logs[boundary..].to_string()
}

fn extract_container_evidence(
    name: &str,
    status: Option<&ContainerStatus>,
    has_readiness_probe: bool,
    events: &[Event],
) -> ContainerEvidence {
    let restart_count = status.map(|s| s.restart_count).unwrap_or(0);

    let current_state = if let Some(s) = status.and_then(|s| s.state.as_ref()) {
        if let Some(w) = &s.waiting {
            ContainerStateSummary {
                state: "waiting".to_string(),
                reason: w.reason.clone(),
                message: w.message.clone(),
                exit_code: None,
                started_at: None,
            }
        } else if let Some(r) = &s.running {
            ContainerStateSummary {
                state: "running".to_string(),
                reason: None,
                message: None,
                exit_code: None,
                started_at: r.started_at.as_ref().map(|t| t.0.to_string()),
            }
        } else if let Some(t) = &s.terminated {
            ContainerStateSummary {
                state: "terminated".to_string(),
                reason: t.reason.clone(),
                message: t.message.clone(),
                exit_code: Some(t.exit_code),
                started_at: t.started_at.as_ref().map(|t| t.0.to_string()),
            }
        } else {
            ContainerStateSummary {
                state: "unknown".to_string(),
                reason: None,
                message: None,
                exit_code: None,
                started_at: None,
            }
        }
    } else {
        ContainerStateSummary {
            state: "unknown".to_string(),
            reason: None,
            message: None,
            exit_code: None,
            started_at: None,
        }
    };

    let last_terminated_state = status
        .and_then(|s| s.last_state.as_ref())
        .and_then(|ls| ls.terminated.as_ref())
        .map(|t| {
            let oom_killed = t.reason.as_deref() == Some("OOMKilled");
            TerminatedStateSummary {
                exit_code: t.exit_code,
                signal: t.signal,
                reason: t.reason.clone(),
                message: t.message.clone(),
                started_at: t.started_at.as_ref().map(|ts| ts.0.to_string()),
                finished_at: t.finished_at.as_ref().map(|ts| ts.0.to_string()),
                oom_killed,
            }
        });

    // Detect failing probes
    let mut failing_probes_set = BTreeSet::new();

    // 1. If container is running or started, but not ready, and spec declares a readiness probe
    let is_ready = status.map(|s| s.ready).unwrap_or(false);
    let is_running_or_started = current_state.state == "running"
        || status
            .and_then(|s| s.started)
            .unwrap_or(current_state.state == "running");
    if has_readiness_probe && is_running_or_started && !is_ready {
        failing_probes_set.insert("Readiness".to_string());
    }

    // 2. Scan events for probe failure indicators targeting this container
    for ev in events {
        if ev.reason.as_deref() == Some("Unhealthy") {
            let msg = ev.message.as_deref().unwrap_or_default();
            // Check involved_object.field_path (e.g. spec.containers{web})
            let matched_container = ev.involved_object.field_path.as_deref().and_then(|fp| {
                if let Some(start) = fp.find('{') {
                    if let Some(end) = fp.find('}') {
                        if start < end {
                            return Some(&fp[start + 1..end]);
                        }
                    }
                }
                None
            });

            let applies_to_container = match matched_container {
                Some(c_name) => c_name == name,
                None => {
                    msg.contains(&format!("container {}", name))
                        || msg.contains(&format!("container \"{}\"", name))
                }
            };

            if applies_to_container {
                if msg.contains("Readiness probe failed") {
                    failing_probes_set.insert("Readiness".to_string());
                }
                if msg.contains("Liveness probe failed") {
                    failing_probes_set.insert("Liveness".to_string());
                }
                if msg.contains("Startup probe failed") {
                    failing_probes_set.insert("Startup".to_string());
                }
            }
        }
    }

    ContainerEvidence {
        name: name.to_string(),
        restart_count,
        current_state,
        last_terminated_state,
        failing_probes: failing_probes_set.into_iter().collect(),
    }
}

/// Pure analysis function evaluating pod unready evidence from Pod resource and Events.
pub fn analyze_pod_readiness(
    pod: &Pod,
    events: &[Event],
    previous_logs_tail: Option<String>,
    current_logs_tail: Option<String>,
) -> WhyNotReadyOut {
    let name = pod.metadata.name.clone().unwrap_or_default();
    let namespace = pod.metadata.namespace.clone().unwrap_or_default();
    let phase = pod
        .status
        .as_ref()
        .and_then(|s| s.phase.clone())
        .unwrap_or_else(|| "Unknown".to_string());

    let spec = pod.spec.as_ref();
    let status = pod.status.as_ref();

    let init_statuses = status.and_then(|s| s.init_container_statuses.as_ref());
    let app_statuses = status.and_then(|s| s.container_statuses.as_ref());

    let mut init_containers = Vec::new();
    if let Some(spec) = spec {
        if let Some(inits) = &spec.init_containers {
            for c in inits {
                let c_status = init_statuses.and_then(|cs| cs.iter().find(|s| s.name == c.name));
                let has_readiness = c.readiness_probe.is_some();
                init_containers.push(extract_container_evidence(
                    &c.name,
                    c_status,
                    has_readiness,
                    events,
                ));
            }
        }
    }

    let mut app_containers = Vec::new();
    if let Some(spec) = spec {
        for c in &spec.containers {
            let c_status = app_statuses.and_then(|cs| cs.iter().find(|s| s.name == c.name));
            let has_readiness = c.readiness_probe.is_some();
            app_containers.push(extract_container_evidence(
                &c.name,
                c_status,
                has_readiness,
                events,
            ));
        }
    }

    // Summarize warning events
    let mut recent_warning_events = Vec::new();
    for ev in events {
        let is_warning = ev.type_.as_deref() == Some("Warning")
            || ev.reason.as_deref() == Some("Failed")
            || ev.reason.as_deref() == Some("Unhealthy")
            || ev.reason.as_deref() == Some("BackOff");

        if is_warning {
            let first_seen = event_first_timestamp(ev).map(|t| t.to_string());
            let last_seen = event_last_timestamp(ev).map(|t| t.to_string());

            recent_warning_events.push(CompactEventSummary {
                type_: ev.type_.clone().unwrap_or_else(|| "Warning".to_string()),
                reason: ev.reason.clone().unwrap_or_default(),
                message: ev.message.clone().unwrap_or_default(),
                count: ev.count.unwrap_or(1),
                first_seen,
                last_seen,
            });
        }
    }

    // Sort warning events by last_seen descending
    recent_warning_events.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));

    WhyNotReadyOut {
        pod: name,
        namespace,
        phase,
        init_containers,
        app_containers,
        previous_logs_tail,
        current_logs_tail,
        recent_warning_events,
    }
}

/// Returns ordered candidate container names to check for crash logs.
/// Ranks failing containers with execution/crash history first, then other failing containers,
/// unready containers, containers with restart history, and finally defaults to the first spec container.
pub fn candidate_containers_for_logs(pod: &Pod, explicit: Option<&str>) -> Vec<String> {
    if let Some(name) = explicit {
        return vec![name.to_string()];
    }

    let status = pod.status.as_ref();
    let spec = pod.spec.as_ref();
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();

    let mut add = |name: &str| {
        if seen.insert(name.to_string()) {
            candidates.push(name.to_string());
        }
    };

    // 1. App containers with execution/crash history (CrashLoopBackOff, terminated non-zero, or Error with restarts)
    if let Some(cs) = status.and_then(|s| s.container_statuses.as_ref()) {
        for c in cs {
            let has_crash_history = c
                .state
                .as_ref()
                .and_then(|st| st.waiting.as_ref())
                .is_some_and(|w| {
                    w.reason.as_deref() == Some("CrashLoopBackOff")
                        || (w.reason.as_deref() == Some("Error") && c.restart_count > 0)
                })
                || c.state
                    .as_ref()
                    .and_then(|st| st.terminated.as_ref())
                    .is_some_and(|t| t.exit_code != 0)
                || c.last_state
                    .as_ref()
                    .and_then(|st| st.terminated.as_ref())
                    .is_some_and(|t| t.exit_code != 0);
            if has_crash_history {
                add(&c.name);
            }
        }
    }

    // 2. Init containers with execution/crash history
    if let Some(ics) = status.and_then(|s| s.init_container_statuses.as_ref()) {
        for c in ics {
            let has_crash_history = c
                .state
                .as_ref()
                .and_then(|st| st.waiting.as_ref())
                .is_some_and(|w| {
                    w.reason.as_deref() == Some("CrashLoopBackOff")
                        || (w.reason.as_deref() == Some("Error") && c.restart_count > 0)
                })
                || c.state
                    .as_ref()
                    .and_then(|st| st.terminated.as_ref())
                    .is_some_and(|t| t.exit_code != 0)
                || c.last_state
                    .as_ref()
                    .and_then(|st| st.terminated.as_ref())
                    .is_some_and(|t| t.exit_code != 0);
            if has_crash_history {
                add(&c.name);
            }
        }
    }

    // 3. Other waiting failing containers (ImagePullBackOff, ErrImagePull, etc.)
    if let Some(cs) = status.and_then(|s| s.container_statuses.as_ref()) {
        for c in cs {
            if c.state
                .as_ref()
                .and_then(|st| st.waiting.as_ref())
                .is_some_and(|w| {
                    w.reason.as_deref() == Some("ImagePullBackOff")
                        || w.reason.as_deref() == Some("ErrImagePull")
                        || w.reason.as_deref() == Some("Error")
                })
            {
                add(&c.name);
            }
        }
    }
    if let Some(ics) = status.and_then(|s| s.init_container_statuses.as_ref()) {
        for c in ics {
            if c.state
                .as_ref()
                .and_then(|st| st.waiting.as_ref())
                .is_some_and(|w| {
                    w.reason.as_deref() == Some("ImagePullBackOff")
                        || w.reason.as_deref() == Some("ErrImagePull")
                        || w.reason.as_deref() == Some("Error")
                })
            {
                add(&c.name);
            }
        }
    }

    // 4. Unready app containers
    if let Some(cs) = status.and_then(|s| s.container_statuses.as_ref()) {
        for c in cs {
            if !c.ready {
                add(&c.name);
            }
        }
    }

    // 5. Containers with restart history
    if let Some(cs) = status.and_then(|s| s.container_statuses.as_ref()) {
        for c in cs {
            if c.restart_count > 0 {
                add(&c.name);
            }
        }
    }

    // 6. Default to first container in spec
    if let Some(s) = spec {
        if let Some(first) = s.containers.first() {
            add(&first.name);
        }
    }

    candidates
}

/// Helper selecting candidate container for previous logs.
/// Ranks currently failing containers ahead of historical restarts.
pub fn select_target_container(pod: &Pod, explicit: Option<&str>) -> Option<String> {
    candidate_containers_for_logs(pod, explicit)
        .into_iter()
        .next()
}

/// `k8s.whyNotReady` capability definition.
pub fn why_not_ready_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<WhyNotReadyIn, WhyNotReadyOut, _, _>(
        "k8s.whyNotReady",
        "assembles structured root-cause evidence for unready or crashing pods (container states, exit codes, failing probes, warning events, and capped log tail)",
        Annotations::READ_ONLY,
        move |input: WhyNotReadyIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;

                let pod_api: Api<Pod> = Api::namespaced(client.clone(), &input.namespace);
                let pod = tokio::time::timeout(request_timeout(), pod_api.get(&input.pod))
                    .await
                    .map_err(|_| CapabilityError::Handler("fetch pod timed out".into()))?
                    .map_err(|e| match e {
                        kube::Error::Api(ref ae) if ae.code == 404 => CapabilityError::Handler(format!(
                            "Pod {}/{} not found",
                            input.namespace, input.pod
                        )),
                        other => CapabilityError::Handler(format!(
                            "Failed to fetch Pod {}/{}: {}",
                            input.namespace, input.pod, other
                        )),
                    })?;

                // Fetch events for this pod, bounded and UID-filtered
                let event_api: Api<Event> = Api::namespaced(client.clone(), &input.namespace);
                let pod_uid = pod.metadata.uid.as_deref().unwrap_or_default();
                let event_params = ListParams::default().fields(&format!(
                    "involvedObject.name={},involvedObject.kind=Pod",
                    input.pod
                ));
                let (raw_events, events_trunc) = crate::list_cap::list_capped(&event_api, event_params)
                    .await
                    .map_err(|e| e.into_capability_error("list events"))?;
                if events_trunc {
                    return Err(CapabilityError::Handler(format!(
                        "Events for pod '{}/{}' were truncated by list limit",
                        input.namespace, input.pod
                    )));
                }

                let events: Vec<_> = raw_events
                    .into_iter()
                    .filter(|ev| {
                        if !pod_uid.is_empty() {
                            if let Some(ev_uid) = &ev.involved_object.uid {
                                return ev_uid == pod_uid;
                            }
                        }
                        true
                    })
                    .collect();

                // Fetch previous or recent logs tail across candidate containers
                let candidate_containers =
                    candidate_containers_for_logs(&pod, input.container_name.as_deref());
                let tail_lines = input
                    .tail_lines
                    .unwrap_or(DEFAULT_TAIL_LINES)
                    .clamp(1, MAX_TAIL_LINES);

                let mut previous_logs_tail = None;
                let mut current_logs_tail = None;

                for c_name in candidate_containers {
                    let log_params_prev = LogParams {
                        container: Some(c_name.clone()),
                        tail_lines: Some(tail_lines),
                        limit_bytes: Some(MAX_LOG_TAIL_BYTES as i64),
                        previous: true,
                        ..Default::default()
                    };
                    let prev_res = tokio::time::timeout(
                        request_timeout(),
                        pod_api.logs(&input.pod, &log_params_prev),
                    )
                    .await;

                    match prev_res {
                        Ok(Ok(text)) if !text.is_empty() => {
                            previous_logs_tail = Some(cap_log_tail(&text, MAX_LOG_TAIL_BYTES));
                            break;
                        }
                        _ => {
                            // If previous logs not found, try current logs
                            let log_params_curr = LogParams {
                                container: Some(c_name),
                                tail_lines: Some(tail_lines),
                                limit_bytes: Some(MAX_LOG_TAIL_BYTES as i64),
                                previous: false,
                                ..Default::default()
                            };
                            let curr = tokio::time::timeout(
                                request_timeout(),
                                pod_api.logs(&input.pod, &log_params_curr),
                            )
                            .await
                            .ok()
                            .and_then(|r| r.ok())
                            .filter(|text| !text.is_empty())
                            .map(|raw| cap_log_tail(&raw, MAX_LOG_TAIL_BYTES));

                            if curr.is_some() {
                                current_logs_tail = curr;
                                break;
                            }
                        }
                    }
                }

                Ok(analyze_pod_readiness(&pod, &events, previous_logs_tail, current_logs_tail))
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        Container, ContainerState, ContainerStateTerminated, ContainerStateWaiting,
        ObjectReference, PodSpec, PodStatus, Probe,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};
    use k8s_openapi::jiff::Timestamp;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = why_not_ready_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.whyNotReady");
    }

    #[test]
    fn deserializes_caller_payload_and_denies_unknown_fields() {
        let valid = serde_json::json!({
            "context": "kind-srelens",
            "namespace": "default",
            "pod": "web-746765799-abcde",
            "containerName": "web",
            "tailLines": 100
        });
        let parsed: Result<WhyNotReadyIn, _> = serde_json::from_value(valid);
        assert!(parsed.is_ok());
        let input = parsed.unwrap();
        assert_eq!(input.context, "kind-srelens");
        assert_eq!(input.container_name.as_deref(), Some("web"));
        assert_eq!(input.tail_lines, Some(100));

        // Unknown field rejected
        let invalid = serde_json::json!({
            "context": "kind-srelens",
            "namespace": "default",
            "pod": "web-746765799-abcde",
            "unexpected": "foo"
        });
        assert!(serde_json::from_value::<WhyNotReadyIn>(invalid).is_err());
    }

    #[test]
    fn separates_init_and_app_containers_and_detects_oom() {
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("crash-pod".to_string()),
                namespace: Some("prod".to_string()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                init_containers: Some(vec![Container {
                    name: "init-db".to_string(),
                    ..Default::default()
                }]),
                containers: vec![Container {
                    name: "api-server".to_string(),
                    readiness_probe: Some(Probe::default()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some("Running".to_string()),
                init_container_statuses: Some(vec![ContainerStatus {
                    name: "init-db".to_string(),
                    ready: true,
                    restart_count: 0,
                    state: Some(ContainerState {
                        terminated: Some(ContainerStateTerminated {
                            exit_code: 0,
                            reason: Some("Completed".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                container_statuses: Some(vec![ContainerStatus {
                    name: "api-server".to_string(),
                    ready: false,
                    restart_count: 5,
                    state: Some(ContainerState {
                        waiting: Some(ContainerStateWaiting {
                            reason: Some("CrashLoopBackOff".to_string()),
                            message: Some("back-off 5m0s restarting failed container".to_string()),
                        }),
                        ..Default::default()
                    }),
                    last_state: Some(ContainerState {
                        terminated: Some(ContainerStateTerminated {
                            exit_code: 137, // OOM kill exit code
                            reason: Some("OOMKilled".to_string()),
                            message: Some("container exceeded memory limit".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
        };

        let now = Timestamp::now();
        let events = vec![Event {
            metadata: ObjectMeta {
                name: Some("ev1".to_string()),
                ..Default::default()
            },
            type_: Some("Warning".to_string()),
            reason: Some("BackOff".to_string()),
            message: Some("Back-off restarting failed container api-server".to_string()),
            count: Some(12),
            involved_object: ObjectReference {
                kind: Some("Pod".to_string()),
                name: Some("crash-pod".to_string()),
                ..Default::default()
            },
            last_timestamp: Some(Time(now)),
            ..Default::default()
        }];

        let out = analyze_pod_readiness(
            &pod,
            &events,
            Some("Fatal error: out of memory\nKilled".to_string()),
            None,
        );

        assert_eq!(out.pod, "crash-pod");
        assert_eq!(out.namespace, "prod");
        assert_eq!(out.phase, "Running");

        // Init container completed successfully
        assert_eq!(out.init_containers.len(), 1);
        assert_eq!(out.init_containers[0].name, "init-db");
        assert_eq!(out.init_containers[0].current_state.state, "terminated");
        assert_eq!(out.init_containers[0].current_state.exit_code, Some(0));

        // App container in CrashLoopBackOff with OOMKilled in last_state
        assert_eq!(out.app_containers.len(), 1);
        let app = &out.app_containers[0];
        assert_eq!(app.name, "api-server");
        assert_eq!(app.restart_count, 5);
        assert_eq!(app.current_state.state, "waiting");
        assert_eq!(
            app.current_state.reason.as_deref(),
            Some("CrashLoopBackOff")
        );

        let last_term = app.last_terminated_state.as_ref().unwrap();
        assert_eq!(last_term.exit_code, 137);
        assert_eq!(last_term.reason.as_deref(), Some("OOMKilled"));
        assert!(last_term.oom_killed);

        // Events
        assert_eq!(out.recent_warning_events.len(), 1);
        assert_eq!(out.recent_warning_events[0].reason, "BackOff");
        assert_eq!(out.recent_warning_events[0].count, 12);

        // Logs tail
        assert_eq!(
            out.previous_logs_tail.as_deref(),
            Some("Fatal error: out of memory\nKilled")
        );
        assert!(out.current_logs_tail.is_none());
    }

    #[test]
    fn forced_sigkill_without_oom_reason_is_not_flagged_as_oom() {
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("killed-pod".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "worker".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some("Running".to_string()),
                container_statuses: Some(vec![ContainerStatus {
                    name: "worker".to_string(),
                    ready: true,
                    restart_count: 1,
                    last_state: Some(ContainerState {
                        terminated: Some(ContainerStateTerminated {
                            exit_code: 137,
                            reason: Some("Error".to_string()), // Forced SIGKILL, not OOMKilled
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
        };

        let out = analyze_pod_readiness(&pod, &[], None, None);
        let term = out.app_containers[0]
            .last_terminated_state
            .as_ref()
            .unwrap();
        assert_eq!(term.exit_code, 137);
        assert!(
            !term.oom_killed,
            "forced SIGKILL without OOMKilled reason must not be oom_killed"
        );
    }

    #[test]
    fn probe_failure_targets_only_named_container_via_field_path() {
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("multi-pod".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![
                    Container {
                        name: "app".to_string(),
                        ..Default::default()
                    },
                    Container {
                        name: "sidecar".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some("Running".to_string()),
                container_statuses: Some(vec![
                    ContainerStatus {
                        name: "app".to_string(),
                        ready: false,
                        restart_count: 0,
                        ..Default::default()
                    },
                    ContainerStatus {
                        name: "sidecar".to_string(),
                        ready: true,
                        restart_count: 0,
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }),
        };

        let events = vec![Event {
            metadata: ObjectMeta {
                name: Some("unhealthy-app".to_string()),
                ..Default::default()
            },
            type_: Some("Warning".to_string()),
            reason: Some("Unhealthy".to_string()),
            message: Some("Readiness probe failed: HTTP 500".to_string()),
            count: Some(3),
            involved_object: ObjectReference {
                kind: Some("Pod".to_string()),
                name: Some("multi-pod".to_string()),
                field_path: Some("spec.containers{app}".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }];

        let out = analyze_pod_readiness(&pod, &events, None, None);
        assert_eq!(out.app_containers[0].name, "app");
        assert_eq!(
            out.app_containers[0].failing_probes,
            vec!["Readiness".to_string()]
        );

        // Sidecar must NOT inherit app probe failure
        assert_eq!(out.app_containers[1].name, "sidecar");
        assert!(out.app_containers[1].failing_probes.is_empty());
    }

    #[test]
    fn cap_log_tail_preserves_short_and_truncates_long() {
        let short = "Line 1\nLine 2\nLine 3";
        assert_eq!(cap_log_tail(short, 100), short);

        let long = "aaaa\nbbbb\ncccc\ndddd\n";
        // Capping to 10 bytes:
        let capped = cap_log_tail(long, 10);
        assert!(capped.len() <= 10);
        assert_eq!(capped, "cccc\ndddd\n");
    }

    #[test]
    fn ranks_crashing_containers_ahead_of_image_pull_backoff() {
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("test-pod".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![
                    Container {
                        name: "pull-fail".to_string(),
                        ..Default::default()
                    },
                    Container {
                        name: "crash-loop".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some("Running".to_string()),
                container_statuses: Some(vec![
                    ContainerStatus {
                        name: "pull-fail".to_string(),
                        ready: false,
                        restart_count: 0,
                        state: Some(ContainerState {
                            waiting: Some(ContainerStateWaiting {
                                reason: Some("ImagePullBackOff".to_string()),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    ContainerStatus {
                        name: "crash-loop".to_string(),
                        ready: false,
                        restart_count: 3,
                        state: Some(ContainerState {
                            waiting: Some(ContainerStateWaiting {
                                reason: Some("CrashLoopBackOff".to_string()),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }),
        };

        let candidates = candidate_containers_for_logs(&pod, None);
        assert_eq!(
            candidates,
            vec!["crash-loop".to_string(), "pull-fail".to_string()],
            "Container with crash history must be ranked ahead of ImagePullBackOff container"
        );
    }
}
