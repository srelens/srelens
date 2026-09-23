//! Changed & Rollout Triage engine.
//!
//! Provides a holistic triage view answering:
//! 1. Did someone recently deploy a new version of an app?
//! 2. What changed (revision, image tag diff)?
//! 3. Is the new deployment breaking (CrashLoopBackOff, OOM exit 137, probe failures, stalled rollout)?
//! 4. Paging verdict: Should an on-call engineer get a page? (🚨 Page, ⚠️ In Progress, 🟢 Healthy).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::api::apps::v1::{Deployment, ReplicaSet};
use k8s_openapi::api::core::v1::{Event, Pod};
use k8s_openapi::jiff::Timestamp;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;
use crate::events::{event_last_timestamp, EventSummary};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PagingVerdict {
    #[serde(rename = "page")]
    Page,
    #[serde(rename = "inProgress")]
    InProgress,
    #[serde(rename = "healthy")]
    Healthy,
    #[serde(rename = "none")]
    None,
}

impl PagingVerdict {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Page => "PAGE",
            Self::InProgress => "IN PROGRESS",
            Self::Healthy => "HEALTHY",
            Self::None => "NONE",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Page => "🚨",
            Self::InProgress => "⚠️",
            Self::Healthy => "🟢",
            Self::None => "⚪",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RolloutStatus {
    #[serde(rename = "complete")]
    Complete,
    #[serde(rename = "progressing")]
    Progressing,
    #[serde(rename = "stalled")]
    Stalled,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "scaledDown")]
    ScaledDown,
    #[serde(rename = "unknown")]
    Unknown,
}

impl RolloutStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Complete => "Complete",
            Self::Progressing => "Progressing",
            Self::Stalled => "Stalled",
            Self::Failed => "Failed",
            Self::ScaledDown => "Scaled Down",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AppDeploymentChange {
    #[serde(rename = "appName")]
    pub app_name: String,
    pub kind: String,
    pub namespace: String,
    pub verdict: PagingVerdict,
    #[serde(rename = "verdictReason")]
    pub verdict_reason: String,
    #[serde(rename = "deployedAt")]
    pub deployed_at: Option<String>,
    #[serde(rename = "deployedAge")]
    pub deployed_age: String,
    #[serde(rename = "currentRevision")]
    pub current_revision: String,
    #[serde(rename = "previousRevision")]
    pub previous_revision: Option<String>,
    #[serde(rename = "currentImages")]
    pub current_images: Vec<String>,
    #[serde(rename = "previousImages")]
    pub previous_images: Vec<String>,
    #[serde(rename = "imageDiff")]
    pub image_diff: String,
    #[serde(rename = "desiredReplicas")]
    pub desired_replicas: i32,
    #[serde(rename = "updatedReplicas")]
    pub updated_replicas: i32,
    #[serde(rename = "readyReplicas")]
    pub ready_replicas: i32,
    #[serde(rename = "availableReplicas")]
    pub available_replicas: i32,
    #[serde(rename = "rolloutStatus")]
    pub rollout_status: RolloutStatus,
    #[serde(rename = "failingPodsCount")]
    pub failing_pods_count: usize,
    #[serde(rename = "crashLoopCount")]
    pub crash_loop_count: usize,
    #[serde(rename = "oomKilledCount")]
    pub oom_killed_count: usize,
    #[serde(rename = "probeFailureCount")]
    pub probe_failure_count: usize,
    #[serde(rename = "restartCount")]
    pub restart_count: i32,
    #[serde(rename = "primarySymptoms")]
    pub primary_symptoms: Vec<String>,
    #[serde(rename = "failingPodNames")]
    pub failing_pod_names: Vec<String>,
    #[serde(rename = "topEvents")]
    pub top_events: Vec<EventSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InfraChangeItem {
    pub age: String,
    #[serde(rename = "lastTs")]
    pub last_ts: Option<String>,
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub reason: String,
    pub message: String,
    pub count: i32,
    #[serde(rename = "isWarning")]
    pub is_warning: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TriageSummary {
    #[serde(rename = "totalDeployments")]
    pub total_deployments: usize,
    #[serde(rename = "pagingAlerts")]
    pub paging_alerts: usize,
    #[serde(rename = "inProgress")]
    pub in_progress: usize,
    pub healthy: usize,
    #[serde(rename = "overallVerdict")]
    pub overall_verdict: PagingVerdict,
    #[serde(rename = "headlineMessage")]
    pub headline_message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChangedTriageReport {
    #[serde(rename = "windowSeconds")]
    pub window_seconds: u64,
    #[serde(rename = "windowLabel")]
    pub window_label: String,
    pub namespace: Option<String>,
    pub summary: TriageSummary,
    pub deployments: Vec<AppDeploymentChange>,
    #[serde(rename = "infraChanges")]
    pub infra_changes: Vec<InfraChangeItem>,
}

pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(stripped) = s.strip_suffix('m') {
        let mins: u64 = stripped
            .parse()
            .map_err(|_| format!("Invalid minutes in duration: {s}"))?;
        Ok(Duration::from_secs(mins * 60))
    } else if let Some(stripped) = s.strip_suffix('h') {
        let hrs: u64 = stripped
            .parse()
            .map_err(|_| format!("Invalid hours in duration: {s}"))?;
        Ok(Duration::from_secs(hrs * 3600))
    } else if let Some(stripped) = s.strip_suffix('d') {
        let days: u64 = stripped
            .parse()
            .map_err(|_| format!("Invalid days in duration: {s}"))?;
        Ok(Duration::from_secs(days * 86400))
    } else if let Some(stripped) = s.strip_suffix('s') {
        let secs: u64 = stripped
            .parse()
            .map_err(|_| format!("Invalid seconds in duration: {s}"))?;
        Ok(Duration::from_secs(secs))
    } else if let Ok(mins) = s.parse::<u64>() {
        Ok(Duration::from_secs(mins * 60))
    } else {
        Err(format!(
            "Unrecognized time window '{s}'. Expected e.g. 15m, 30m, 1h, 3h, 24h"
        ))
    }
}

pub fn format_duration_label(d: Duration) -> String {
    let secs = d.as_secs();
    if secs % 86400 == 0 && secs > 0 {
        format!("{}d", secs / 86400)
    } else if secs % 3600 == 0 && secs > 0 {
        format!("{}h", secs / 3600)
    } else if secs % 60 == 0 && secs > 0 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

fn extract_container_images(rs: &ReplicaSet) -> Vec<String> {
    rs.spec
        .as_ref()
        .and_then(|s| s.template.as_ref())
        .and_then(|t| t.spec.as_ref())
        .map(|ps| {
            ps.containers
                .iter()
                .filter_map(|c| c.image.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn format_image_diff(prev: &[String], curr: &[String]) -> String {
    if prev.is_empty() && curr.is_empty() {
        return "Unknown image".to_string();
    }
    if prev.is_empty() {
        return curr.join(", ");
    }
    if curr.is_empty() {
        return prev.join(", ");
    }

    if prev == curr {
        let img = curr.first().map(|s| s.as_str()).unwrap_or("");
        let short = img.split('/').last().unwrap_or(img);
        return format!("{short} (reconfigured)");
    }

    let mut diffs = Vec::new();
    for (i, c) in curr.iter().enumerate() {
        let c_short = c.split('/').last().unwrap_or(c);
        if let Some(p) = prev.get(i) {
            let p_short = p.split('/').last().unwrap_or(p);
            if p_short != c_short {
                diffs.push(format!("{p_short} ➔ {c_short}"));
            } else {
                diffs.push(c_short.to_string());
            }
        } else {
            diffs.push(format!("+ {c_short}"));
        }
    }
    if diffs.is_empty() {
        curr.join(", ")
    } else {
        diffs.join(", ")
    }
}

fn parse_revision(rs: &ReplicaSet) -> i64 {
    rs.metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get("deployment.kubernetes.io/revision"))
        .and_then(|r| r.parse::<i64>().ok())
        .unwrap_or(0)
}

/// Evaluates deployments, replicasets, pods, and events against a time window to produce
/// a holistic deployment triage report answering "Should I get a page?".
pub fn evaluate_changed_triage(
    deployments: &[Deployment],
    replicasets: &[ReplicaSet],
    pods: &[Pod],
    events: &[Event],
    window: Duration,
    now: Timestamp,
    namespace: Option<String>,
) -> ChangedTriageReport {
    let window_secs = window.as_secs();
    let cutoff_ts = now
        .checked_sub(k8s_openapi::jiff::SignedDuration::from_secs(
            window_secs as i64,
        ))
        .ok();

    // 1. Group ReplicaSets by owning Deployment name and namespace
    let mut rs_by_deployment: HashMap<(String, String), Vec<&ReplicaSet>> = HashMap::new();
    for rs in replicasets {
        let ns = rs.metadata.namespace.clone().unwrap_or_default();
        if let Some(ref target_ns) = namespace {
            if !target_ns.is_empty() && &ns != target_ns {
                continue;
            }
        }
        for owner in rs.metadata.owner_references.iter().flatten() {
            if owner.kind == "Deployment" {
                rs_by_deployment
                    .entry((ns.clone(), owner.name.clone()))
                    .or_default()
                    .push(rs);
            }
        }
    }

    // 2. Index Pods by owner ReplicaSet name and namespace
    let mut pods_by_rs: HashMap<(String, String), Vec<&Pod>> = HashMap::new();
    for pod in pods {
        let ns = pod.metadata.namespace.clone().unwrap_or_default();
        if let Some(ref target_ns) = namespace {
            if !target_ns.is_empty() && &ns != target_ns {
                continue;
            }
        }
        for owner in pod.metadata.owner_references.iter().flatten() {
            if owner.kind == "ReplicaSet" {
                pods_by_rs
                    .entry((ns.clone(), owner.name.clone()))
                    .or_default()
                    .push(pod);
            }
        }
    }

    // 3. Index Events by involved object (kind, namespace, name)
    let mut events_by_object: HashMap<(String, String, String), Vec<&Event>> = HashMap::new();
    for ev in events {
        let ns = ev.metadata.namespace.clone().unwrap_or_default();
        if let Some(ref target_ns) = namespace {
            if !target_ns.is_empty() && &ns != target_ns {
                continue;
            }
        }
        let kind = ev.involved_object.kind.clone().unwrap_or_default();
        let name = ev.involved_object.name.clone().unwrap_or_default();
        events_by_object
            .entry((kind, ns, name))
            .or_default()
            .push(ev);
    }

    let mut deployment_changes = Vec::new();
    let mut deployments_seen = std::collections::HashSet::new();

    // 4. Examine each Deployment
    for dep in deployments {
        let dep_ns = dep.metadata.namespace.clone().unwrap_or_default();
        if let Some(ref target_ns) = namespace {
            if !target_ns.is_empty() && &dep_ns != target_ns {
                continue;
            }
        }
        let dep_name = dep.metadata.name.clone().unwrap_or_default();
        deployments_seen.insert((dep_ns.clone(), dep_name.clone()));

        let mut owned_rs = rs_by_deployment
            .get(&(dep_ns.clone(), dep_name.clone()))
            .cloned()
            .unwrap_or_default();
        // Sort ReplicaSets descending by revision, fallback to creation timestamp
        owned_rs.sort_by(|a, b| {
            let rev_a = parse_revision(a);
            let rev_b = parse_revision(b);
            rev_b.cmp(&rev_a).then_with(|| {
                let ts_a = a.metadata.creation_timestamp.as_ref().map(|t| t.0);
                let ts_b = b.metadata.creation_timestamp.as_ref().map(|t| t.0);
                ts_b.cmp(&ts_a)
            })
        });

        let current_rs = owned_rs.first().copied();
        let prev_rs = owned_rs.get(1).copied();

        // Check if deployment or its active revision falls within the time window
        let mut in_window = false;
        let mut deployed_at = None;
        let mut deployed_age = "-".to_string();

        if let Some(rs) = current_rs {
            if let Some(ref ct) = rs.metadata.creation_timestamp {
                deployed_at = Some(ct.0.to_string());
                deployed_age = crate::format_age(now.duration_since(ct.0).as_secs().max(0));
                if let Some(ref cutoff) = cutoff_ts {
                    if ct.0 >= *cutoff {
                        in_window = true;
                    }
                }
            }
        }

        // Also check if deployment events happened within window
        let dep_events = events_by_object
            .get(&("Deployment".to_string(), dep_ns.clone(), dep_name.clone()))
            .cloned()
            .unwrap_or_default();
        for ev in &dep_events {
            if let Some(last_ts) = event_last_timestamp(ev) {
                if let Some(ref cutoff) = cutoff_ts {
                    if last_ts >= *cutoff {
                        in_window = true;
                        break;
                    }
                }
            }
        }

        if !in_window {
            continue;
        }

        let current_images = current_rs.map(extract_container_images).unwrap_or_default();
        let prev_images = prev_rs.map(extract_container_images).unwrap_or_default();
        let image_diff = format_image_diff(&prev_images, &current_images);

        let current_revision = current_rs
            .and_then(|rs| {
                rs.metadata
                    .annotations
                    .as_ref()
                    .and_then(|a| a.get("deployment.kubernetes.io/revision"))
            })
            .cloned()
            .unwrap_or_else(|| "1".to_string());

        let prev_revision = prev_rs.and_then(|rs| {
            rs.metadata
                .annotations
                .as_ref()
                .and_then(|a| a.get("deployment.kubernetes.io/revision"))
                .cloned()
        });

        let desired_replicas = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
        let status = dep.status.as_ref();
        let updated_replicas = status.and_then(|s| s.updated_replicas).unwrap_or(0);
        let ready_replicas = status.and_then(|s| s.ready_replicas).unwrap_or(0);
        let available_replicas = status.and_then(|s| s.available_replicas).unwrap_or(0);

        // Check progress deadline condition
        let mut progress_deadline_exceeded = false;
        if let Some(st) = status {
            if let Some(ref conds) = st.conditions {
                for c in conds {
                    if c.type_ == "Progressing"
                        && c.status == "False"
                        && c.reason.as_deref() == Some("ProgressDeadlineExceeded")
                    {
                        progress_deadline_exceeded = true;
                    }
                }
            }
        }

        // Gather pods belonging to current ReplicaSet
        let current_rs_name = current_rs
            .and_then(|rs| rs.metadata.name.as_deref())
            .unwrap_or("");
        let current_pods = pods_by_rs
            .get(&(dep_ns.clone(), current_rs_name.to_string()))
            .cloned()
            .unwrap_or_default();

        let mut failing_pod_names = Vec::new();
        let mut crash_loop_count = 0;
        let mut oom_killed_count = 0;
        let mut restart_count = 0;
        let mut primary_symptoms = Vec::new();

        for p in &current_pods {
            let pod_name = p.metadata.name.clone().unwrap_or_default();
            let mut is_pod_failing = false;

            if let Some(ref st) = p.status {
                if let Some(ref c_statuses) = st.container_statuses {
                    for cs in c_statuses {
                        restart_count += cs.restart_count;
                        if let Some(ref waiting) =
                            cs.state.as_ref().and_then(|s| s.waiting.as_ref())
                        {
                            let reason = waiting.reason.as_deref().unwrap_or("");
                            if reason == "CrashLoopBackOff"
                                || reason == "ImagePullBackOff"
                                || reason == "CreateContainerConfigError"
                                || reason == "ErrImagePull"
                            {
                                is_pod_failing = true;
                                crash_loop_count += 1;
                                let msg = format!("{pod_name}: {reason}");
                                if !primary_symptoms.contains(&msg) {
                                    primary_symptoms.push(msg);
                                }
                            }
                        }
                        if let Some(ref term) =
                            cs.last_state.as_ref().and_then(|s| s.terminated.as_ref())
                        {
                            if term.exit_code == 137 || term.reason.as_deref() == Some("OOMKilled")
                            {
                                is_pod_failing = true;
                                oom_killed_count += 1;
                                let msg = format!("{pod_name}: OOMKilled (exit 137)");
                                if !primary_symptoms.contains(&msg) {
                                    primary_symptoms.push(msg);
                                }
                            } else if term.exit_code != 0 {
                                is_pod_failing = true;
                                let msg =
                                    format!("{pod_name}: exited with code {}", term.exit_code);
                                if !primary_symptoms.contains(&msg) {
                                    primary_symptoms.push(msg);
                                }
                            }
                        }
                    }
                }
            }

            if is_pod_failing && !failing_pod_names.contains(&pod_name) {
                failing_pod_names.push(pod_name);
            }
        }

        // Correlate Events for this Deployment, its RS, and its Pods
        let mut correlated_events: Vec<EventSummary> = Vec::new();
        let mut probe_failure_count = 0;

        for ev in &dep_events {
            correlated_events.push(crate::events::summarise((*ev).clone()));
        }
        if !current_rs_name.is_empty() {
            if let Some(rs_events) = events_by_object.get(&(
                "ReplicaSet".to_string(),
                dep_ns.clone(),
                current_rs_name.to_string(),
            )) {
                for ev in rs_events {
                    correlated_events.push(crate::events::summarise((*ev).clone()));
                }
            }
        }
        for p in &current_pods {
            let p_name = p.metadata.name.clone().unwrap_or_default();
            if let Some(p_events) =
                events_by_object.get(&("Pod".to_string(), dep_ns.clone(), p_name))
            {
                for ev in p_events {
                    if ev.reason.as_deref() == Some("Unhealthy") {
                        probe_failure_count += ev.count.unwrap_or(1) as usize;
                        let msg = ev.message.clone().unwrap_or_default();
                        let first_line = msg
                            .lines()
                            .next()
                            .unwrap_or(&msg)
                            .chars()
                            .take(80)
                            .collect::<String>();
                        let s = format!("Probe failed: {first_line}");
                        if !primary_symptoms.contains(&s) {
                            primary_symptoms.push(s);
                        }
                    }
                    correlated_events.push(crate::events::summarise((*ev).clone()));
                }
            }
        }

        // Determine Rollout Status
        let rollout_status = if desired_replicas == 0 {
            RolloutStatus::ScaledDown
        } else if progress_deadline_exceeded {
            RolloutStatus::Stalled
        } else if ready_replicas == desired_replicas
            && updated_replicas == desired_replicas
            && crash_loop_count == 0
        {
            RolloutStatus::Complete
        } else if !failing_pod_names.is_empty() {
            RolloutStatus::Failed
        } else {
            RolloutStatus::Progressing
        };

        // Determine Paging Verdict
        let (verdict, verdict_reason) = if !failing_pod_names.is_empty()
            || progress_deadline_exceeded
            || (desired_replicas > 0 && ready_replicas == 0 && !current_pods.is_empty())
            || oom_killed_count > 0
            || probe_failure_count >= 3
        {
            let mut reasons = Vec::new();
            if oom_killed_count > 0 {
                reasons.push(format!("{oom_killed_count} pod(s) OOMKilled"));
            }
            if crash_loop_count > 0 {
                reasons.push(format!(
                    "{crash_loop_count} pod(s) in CrashLoop/ImagePullBackOff"
                ));
            }
            if probe_failure_count > 0 {
                reasons.push(format!("{probe_failure_count} probe failure(s)"));
            }
            if progress_deadline_exceeded {
                reasons.push("Rollout stalled (deadline exceeded)".to_string());
            }
            if ready_replicas < desired_replicas {
                reasons.push(format!("{ready_replicas}/{desired_replicas} Ready"));
            }
            (
                PagingVerdict::Page,
                if reasons.is_empty() {
                    "Deployment failing: pods unready".to_string()
                } else {
                    reasons.join(", ")
                },
            )
        } else if rollout_status == RolloutStatus::Progressing {
            (
                PagingVerdict::InProgress,
                format!("Rollout progressing ({ready_replicas}/{desired_replicas} Ready)"),
            )
        } else if rollout_status == RolloutStatus::Complete {
            (
                PagingVerdict::Healthy,
                format!("Rollout complete ({ready_replicas}/{desired_replicas} Ready, healthy)"),
            )
        } else {
            (
                PagingVerdict::None,
                "Deployment scaled down or quiet".to_string(),
            )
        };

        deployment_changes.push(AppDeploymentChange {
            app_name: dep_name,
            kind: "Deployment".to_string(),
            namespace: dep_ns,
            verdict,
            verdict_reason,
            deployed_at,
            deployed_age,
            current_revision,
            previous_revision: prev_revision,
            current_images,
            previous_images: prev_images,
            image_diff,
            desired_replicas,
            updated_replicas,
            ready_replicas,
            available_replicas,
            rollout_status,
            failing_pods_count: failing_pod_names.len(),
            crash_loop_count,
            oom_killed_count,
            probe_failure_count,
            restart_count,
            primary_symptoms,
            failing_pod_names,
            top_events: correlated_events,
        });
    }

    // Sort deployments: PAGE first, then IN_PROGRESS, then HEALTHY, then quiet
    deployment_changes.sort_by(|a, b| {
        let rank = |v: PagingVerdict| match v {
            PagingVerdict::Page => 0,
            PagingVerdict::InProgress => 1,
            PagingVerdict::Healthy => 2,
            PagingVerdict::None => 3,
        };
        rank(a.verdict).cmp(&rank(b.verdict))
    });

    // 5. Gather non-deployment Infrastructure & Config changes
    let mut infra_changes = Vec::new();
    let mut seen_infra: HashMap<(String, String, String, String), (InfraChangeItem, i32)> =
        HashMap::new();

    for ev in events {
        let kind = ev.involved_object.kind.clone().unwrap_or_default();
        let obj_name = ev.involved_object.name.clone().unwrap_or_default();
        let ev_ns = ev.metadata.namespace.clone().unwrap_or_default();

        if let Some(ref target_ns) = namespace {
            if !target_ns.is_empty() && &ev_ns != target_ns {
                continue;
            }
        }

        // Filter events within window
        let last_ts = event_last_timestamp(ev);
        let in_window = match (&cutoff_ts, &last_ts) {
            (Some(cutoff), Some(ts)) => ts >= cutoff,
            (None, _) => true,
            _ => true,
        };
        if !in_window {
            continue;
        }

        // Skip events belonging to deployments already tracked in deployment_changes
        if (kind == "Deployment" || kind == "ReplicaSet")
            && deployments_seen.contains(&(ev_ns.clone(), obj_name.clone()))
        {
            continue;
        }

        let reason = ev.reason.clone().unwrap_or_default();
        let message = ev.message.clone().unwrap_or_default();
        let count = ev.count.unwrap_or(1);
        let is_warning = ev.type_.as_deref() == Some("Warning");
        let age = last_ts
            .map(|t| crate::format_age(now.duration_since(t).as_secs().max(0)))
            .unwrap_or_else(|| "-".to_string());

        let key = (
            kind.clone(),
            obj_name.clone(),
            ev_ns.clone(),
            reason.clone(),
        );
        if let Some((existing, existing_count)) = seen_infra.get_mut(&key) {
            *existing_count += count;
            existing.count = *existing_count;
        } else {
            seen_infra.insert(
                key,
                (
                    InfraChangeItem {
                        age,
                        last_ts: last_ts.map(|t| t.to_string()),
                        kind,
                        name: obj_name,
                        namespace: ev_ns,
                        reason,
                        message,
                        count,
                        is_warning,
                    },
                    count,
                ),
            );
        }
    }

    for (_, (item, _)) in seen_infra {
        infra_changes.push(item);
    }
    // Warnings first, then sort by count descending
    infra_changes.sort_by(|a, b| {
        b.is_warning
            .cmp(&a.is_warning)
            .then_with(|| b.count.cmp(&a.count))
    });

    // 6. Compute Triage Summary
    let total_deployments = deployment_changes.len();
    let paging_alerts = deployment_changes
        .iter()
        .filter(|d| d.verdict == PagingVerdict::Page)
        .count();
    let in_progress = deployment_changes
        .iter()
        .filter(|d| d.verdict == PagingVerdict::InProgress)
        .count();
    let healthy = deployment_changes
        .iter()
        .filter(|d| d.verdict == PagingVerdict::Healthy)
        .count();

    let overall_verdict = if paging_alerts > 0 {
        PagingVerdict::Page
    } else if in_progress > 0 {
        PagingVerdict::InProgress
    } else if healthy > 0 {
        PagingVerdict::Healthy
    } else {
        PagingVerdict::None
    };

    let headline_message = match overall_verdict {
        PagingVerdict::Page => {
            let first_failing = deployment_changes
                .iter()
                .find(|d| d.verdict == PagingVerdict::Page)
                .map(|d| format!("{}: {}", d.app_name, d.verdict_reason))
                .unwrap_or_default();
            format!("PAGE REQUIRED: {first_failing}")
        }
        PagingVerdict::InProgress => {
            format!("{in_progress} deployment(s) progressing within deadline")
        }
        PagingVerdict::Healthy => {
            format!("{healthy} deployment(s) healthy, no page needed")
        }
        PagingVerdict::None => "No active deployments or alerts in window".to_string(),
    };

    ChangedTriageReport {
        window_seconds: window_secs,
        window_label: format_duration_label(window),
        namespace,
        summary: TriageSummary {
            total_deployments,
            paging_alerts,
            in_progress,
            healthy,
            overall_verdict,
            headline_message,
        },
        deployments: deployment_changes,
        infra_changes,
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListChangesIn {
    pub context: String,
    #[serde(default)]
    pub namespace: String,
    #[serde(default = "default_since")]
    pub since: String,
}

fn default_since() -> String {
    "30m".to_string()
}

/// Fetches and evaluates the holistic triage report for recent changes across a cluster context.
pub async fn fetch_changed_triage(
    cache: &ClientCache,
    context: &str,
    namespace: Option<&str>,
    window: Duration,
) -> Result<ChangedTriageReport, String> {
    let client = cache.get(context).await.map_err(|e| e.to_string())?;
    let ns_str = namespace.unwrap_or("");
    let now = Timestamp::now();

    let dep_api: Api<Deployment> = crate::scoped_api(client.clone(), ns_str);
    let rs_api: Api<ReplicaSet> = crate::scoped_api(client.clone(), ns_str);
    let pod_api: Api<Pod> = crate::scoped_api(client.clone(), ns_str);
    let ev_api: Api<Event> = crate::scoped_api(client, ns_str);

    let timeout = request_timeout();
    let deps = tokio::time::timeout(timeout, dep_api.list(&ListParams::default()))
        .await
        .map_err(|_| "list deployments timed out".to_string())?
        .map_err(|e| e.to_string())?
        .items;

    let rs = tokio::time::timeout(timeout, rs_api.list(&ListParams::default()))
        .await
        .map_err(|_| "list replicasets timed out".to_string())?
        .map_err(|e| e.to_string())?
        .items;

    let pods = tokio::time::timeout(timeout, pod_api.list(&ListParams::default()))
        .await
        .map_err(|_| "list pods timed out".to_string())?
        .map_err(|e| e.to_string())?
        .items;

    let events = tokio::time::timeout(timeout, ev_api.list(&ListParams::default()))
        .await
        .map_err(|_| "list events timed out".to_string())?
        .map_err(|e| e.to_string())?
        .items;

    let ns_opt = namespace.filter(|s| !s.is_empty()).map(|s| s.to_string());

    let report = evaluate_changed_triage(&deps, &rs, &pods, &events, window, now, ns_opt);
    Ok(report)
}

/// `k8s.listChanges` — provides holistic deployment & change triage report for MCP and agents.
pub fn list_changes_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListChangesIn, ChangedTriageReport, _, _>(
        "k8s.listChanges",
        "holistic deployment and change incident triage: evaluates recent rollouts, image diffs, pod crash loops, probe failures, and produces a paging triage verdict",
        Annotations::READ_ONLY,
        move |input: ListChangesIn| {
            let cache = cache.clone();
            async move {
                let window = parse_duration(&input.since).map_err(CapabilityError::Handler)?;
                let ns_opt = if input.namespace.is_empty() {
                    None
                } else {
                    Some(input.namespace.as_str())
                };
                fetch_changed_triage(&cache, &input.context, ns_opt, window)
                    .await
                    .map_err(CapabilityError::Handler)
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::apps::v1::{
        DeploymentSpec, DeploymentStatus, ReplicaSetSpec, ReplicaSetStatus,
    };
    use k8s_openapi::api::core::v1::{
        Container, ContainerState, ContainerStateTerminated, ContainerStateWaiting,
        ContainerStatus, PodSpec, PodStatus, PodTemplateSpec,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference, Time};
    use std::collections::BTreeMap;

    fn make_owner_ref(kind: &str, name: &str) -> OwnerReference {
        OwnerReference {
            api_version: "apps/v1".to_string(),
            kind: kind.to_string(),
            name: name.to_string(),
            uid: "uid-123".to_string(),
            controller: Some(true),
            block_owner_deletion: Some(true),
        }
    }

    #[test]
    fn parse_duration_accepts_common_time_windows() {
        assert_eq!(parse_duration("15m").unwrap(), Duration::from_secs(15 * 60));
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(30 * 60));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_duration("3h").unwrap(), Duration::from_secs(3 * 3600));
        assert_eq!(
            parse_duration("24h").unwrap(),
            Duration::from_secs(24 * 3600)
        );
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
        assert_eq!(parse_duration("60s").unwrap(), Duration::from_secs(60));
        assert_eq!(parse_duration("45").unwrap(), Duration::from_secs(45 * 60));
        assert!(parse_duration("invalid").is_err());
    }

    #[test]
    fn triage_detects_deployment_with_crash_loop_and_verdicts_page() {
        let now = Timestamp::from_second(1_700_000_000).unwrap();
        let dep_time = Timestamp::from_second(1_700_000_000 - 300).unwrap(); // 5m ago

        let dep = Deployment {
            metadata: ObjectMeta {
                name: Some("checkout".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: Some(3),
                ..Default::default()
            }),
            status: Some(DeploymentStatus {
                replicas: Some(3),
                updated_replicas: Some(3),
                ready_replicas: Some(0),
                available_replicas: Some(0),
                ..Default::default()
            }),
        };

        let mut rs_ann = BTreeMap::new();
        rs_ann.insert(
            "deployment.kubernetes.io/revision".to_string(),
            "2".to_string(),
        );
        let current_rs = ReplicaSet {
            metadata: ObjectMeta {
                name: Some("checkout-v2".to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("Deployment", "checkout")]),
                annotations: Some(rs_ann),
                creation_timestamp: Some(Time(dep_time)),
                ..Default::default()
            },
            spec: Some(ReplicaSetSpec {
                replicas: Some(3),
                template: Some(PodTemplateSpec {
                    spec: Some(PodSpec {
                        containers: vec![Container {
                            name: "checkout".to_string(),
                            image: Some("acme/checkout:v2.0".to_string()),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            status: Some(ReplicaSetStatus {
                replicas: 3,
                ready_replicas: Some(0),
                ..Default::default()
            }),
        };

        let mut prev_ann = BTreeMap::new();
        prev_ann.insert(
            "deployment.kubernetes.io/revision".to_string(),
            "1".to_string(),
        );
        let prev_rs = ReplicaSet {
            metadata: ObjectMeta {
                name: Some("checkout-v1".to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("Deployment", "checkout")]),
                annotations: Some(prev_ann),
                creation_timestamp: Some(Time(
                    Timestamp::from_second(1_700_000_000 - 7200).unwrap(),
                )),
                ..Default::default()
            },
            spec: Some(ReplicaSetSpec {
                replicas: Some(0),
                template: Some(PodTemplateSpec {
                    spec: Some(PodSpec {
                        containers: vec![Container {
                            name: "checkout".to_string(),
                            image: Some("acme/checkout:v1.0".to_string()),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            status: Some(ReplicaSetStatus {
                replicas: 0,
                ready_replicas: Some(0),
                ..Default::default()
            }),
        };

        // Pod crashing with CrashLoopBackOff and OOM exit code 137
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("checkout-v2-abcde".to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("ReplicaSet", "checkout-v2")]),
                ..Default::default()
            },
            spec: Some(PodSpec::default()),
            status: Some(PodStatus {
                container_statuses: Some(vec![ContainerStatus {
                    name: "checkout".to_string(),
                    ready: false,
                    restart_count: 5,
                    state: Some(ContainerState {
                        waiting: Some(ContainerStateWaiting {
                            reason: Some("CrashLoopBackOff".to_string()),
                            message: Some("back-off restarting".to_string()),
                        }),
                        ..Default::default()
                    }),
                    last_state: Some(ContainerState {
                        terminated: Some(ContainerStateTerminated {
                            exit_code: 137,
                            reason: Some("OOMKilled".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
        };

        let report = evaluate_changed_triage(
            &[dep],
            &[current_rs, prev_rs],
            &[pod],
            &[],
            Duration::from_secs(1800), // 30m
            now,
            Some("default".to_string()),
        );

        assert_eq!(report.summary.total_deployments, 1);
        assert_eq!(report.summary.paging_alerts, 1);
        assert_eq!(report.summary.overall_verdict, PagingVerdict::Page);

        let change = &report.deployments[0];
        assert_eq!(change.app_name, "checkout");
        assert_eq!(change.verdict, PagingVerdict::Page);
        assert_eq!(change.current_revision, "2");
        assert_eq!(change.previous_revision.as_deref(), Some("1"));
        assert_eq!(change.image_diff, "checkout:v1.0 ➔ checkout:v2.0");
        assert_eq!(change.ready_replicas, 0);
        assert_eq!(change.desired_replicas, 3);
        assert_eq!(change.oom_killed_count, 1);
        assert_eq!(change.crash_loop_count, 1);
        assert!(change.verdict_reason.contains("OOMKilled"));
    }

    #[test]
    fn triage_detects_healthy_deployment_and_verdicts_healthy() {
        let now = Timestamp::from_second(1_700_000_000).unwrap();
        let dep_time = Timestamp::from_second(1_700_000_000 - 300).unwrap();

        let dep = Deployment {
            metadata: ObjectMeta {
                name: Some("auth".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: Some(2),
                ..Default::default()
            }),
            status: Some(DeploymentStatus {
                replicas: Some(2),
                updated_replicas: Some(2),
                ready_replicas: Some(2),
                available_replicas: Some(2),
                ..Default::default()
            }),
        };

        let mut rs_ann = BTreeMap::new();
        rs_ann.insert(
            "deployment.kubernetes.io/revision".to_string(),
            "5".to_string(),
        );
        let current_rs = ReplicaSet {
            metadata: ObjectMeta {
                name: Some("auth-v5".to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("Deployment", "auth")]),
                annotations: Some(rs_ann),
                creation_timestamp: Some(Time(dep_time)),
                ..Default::default()
            },
            spec: Some(ReplicaSetSpec {
                replicas: Some(2),
                template: Some(PodTemplateSpec {
                    spec: Some(PodSpec {
                        containers: vec![Container {
                            name: "auth".to_string(),
                            image: Some("acme/auth:v5.0".to_string()),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            status: Some(ReplicaSetStatus {
                replicas: 2,
                ready_replicas: Some(2),
                ..Default::default()
            }),
        };

        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("auth-v5-12345".to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("ReplicaSet", "auth-v5")]),
                ..Default::default()
            },
            spec: Some(PodSpec::default()),
            status: Some(PodStatus {
                container_statuses: Some(vec![ContainerStatus {
                    name: "auth".to_string(),
                    ready: true,
                    restart_count: 0,
                    ..Default::default()
                }]),
                ..Default::default()
            }),
        };

        let report = evaluate_changed_triage(
            &[dep],
            &[current_rs],
            &[pod],
            &[],
            Duration::from_secs(1800),
            now,
            Some("default".to_string()),
        );

        assert_eq!(report.summary.total_deployments, 1);
        assert_eq!(report.summary.paging_alerts, 0);
        assert_eq!(report.summary.healthy, 1);
        assert_eq!(report.summary.overall_verdict, PagingVerdict::Healthy);

        let change = &report.deployments[0];
        assert_eq!(change.app_name, "auth");
        assert_eq!(change.verdict, PagingVerdict::Healthy);
        assert_eq!(change.rollout_status, RolloutStatus::Complete);
    }
}
