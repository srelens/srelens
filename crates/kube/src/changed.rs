//! Changed & Rollout Triage engine.
//!
//! Provides an SRE incident investigation dashboard for 2 AM post-page triage:
//! 1. What broke? (CrashLoopBackOff, OOM exit 137, probe failures, stalled rollout).
//! 2. Did a release cause it? (GitOps / ArgoCD commit SHA, target revision, sync state).
//! 3. Why is it Pending? (Compute/CPU/Memory/Taint, Storage/PVC unbound, Network/CNI, Image).
//! 4. Show the error! (Inline 5-line log snippet from the crashing/terminated container).

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

use crate::argo::ArgoApplication;
use crate::client_cache::ClientCache;
use crate::connect::request_timeout;
use crate::events::{event_last_timestamp, EventSummary};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum IncidentStatus {
    #[serde(rename = "crashLoop")]
    CrashLoop,
    #[serde(rename = "oomKilled")]
    OomKilled,
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "stalled")]
    Stalled,
    #[serde(rename = "rolling")]
    Rolling,
    #[serde(rename = "healthy")]
    Healthy,
    #[serde(rename = "scaledDown")]
    ScaledDown,
    #[serde(rename = "unknown")]
    Unknown,
}

impl IncidentStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::CrashLoop => "CrashLoop",
            Self::OomKilled => "OOMKilled",
            Self::Pending => "Pending",
            Self::Stalled => "Stalled",
            Self::Rolling => "Rolling",
            Self::Healthy => "Healthy",
            Self::ScaledDown => "Scaled Down",
            Self::Unknown => "Unknown",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::CrashLoop => "💥",
            Self::OomKilled => "💀",
            Self::Pending => "⏳",
            Self::Stalled => "🚫",
            Self::Rolling => "🔄",
            Self::Healthy => "🟢",
            Self::ScaledDown => "⚪",
            Self::Unknown => "❓",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum FailureCategory {
    #[serde(rename = "compute")]
    Compute, // Insufficient CPU/Memory, untolerated taints, node affinity
    #[serde(rename = "storage")]
    Storage, // PVC unbound, mount failure, volume attach failed
    #[serde(rename = "network")]
    Network, // CNI failure, IP allocation failure, pod sandbox error
    #[serde(rename = "image")]
    Image, // ImagePullBackOff, ErrImagePull, auth failure
    #[serde(rename = "app")]
    App, // CrashLoopBackOff, container exit code != 0, panic
    #[serde(rename = "none")]
    None,
}

impl FailureCategory {
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Compute => "[COMPUTE]",
            Self::Storage => "[STORAGE]",
            Self::Network => "[NETWORK]",
            Self::Image => "[IMAGE]",
            Self::App => "[APP]",
            Self::None => "[OK]",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitOpsReleaseInfo {
    #[serde(rename = "appName")]
    pub app_name: String,
    #[serde(rename = "syncStatus")]
    pub sync_status: String,
    #[serde(rename = "healthStatus")]
    pub health_status: String,
    #[serde(rename = "repoUrl")]
    pub repo_url: String,
    #[serde(rename = "targetRevision")]
    pub target_revision: String,
    #[serde(rename = "syncRevision")]
    pub sync_revision: String,
    #[serde(rename = "syncAge")]
    pub sync_age: String,
    #[serde(rename = "syncMessage")]
    pub sync_message: Option<String>,
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
    #[serde(rename = "incidentStatus")]
    pub incident_status: IncidentStatus,
    #[serde(rename = "failureCategory")]
    pub failure_category: FailureCategory,
    #[serde(rename = "failureDetail")]
    pub failure_detail: String,
    pub gitops: Option<GitOpsReleaseInfo>,
    #[serde(rename = "errorLogSnippet")]
    pub error_log_snippet: Option<Vec<String>>,
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
    #[serde(rename = "crashingCount")]
    pub crashing_count: usize,
    #[serde(rename = "pendingCount")]
    pub pending_count: usize,
    #[serde(rename = "rollingCount")]
    pub rolling_count: usize,
    #[serde(rename = "healthyCount")]
    pub healthy_count: usize,
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

/// Analyzes a Pod's conditions, container states, and warning events to determine root cause category.
pub fn analyze_pod_failure(pod: &Pod, pod_events: &[&Event]) -> (FailureCategory, Option<String>) {
    if let Some(ref st) = pod.status {
        // 1. Container Waiting / Terminated state
        if let Some(ref c_statuses) = st.container_statuses {
            for cs in c_statuses {
                if let Some(ref term) = cs.last_state.as_ref().and_then(|s| s.terminated.as_ref()) {
                    if term.exit_code == 137 || term.reason.as_deref() == Some("OOMKilled") {
                        return (
                            FailureCategory::App,
                            Some(format!("{} OOMKilled (Exit Code 137)", cs.name)),
                        );
                    }
                    if term.exit_code != 0 {
                        return (
                            FailureCategory::App,
                            Some(format!(
                                "{} terminated with Exit Code {}",
                                cs.name, term.exit_code
                            )),
                        );
                    }
                }
                if let Some(ref term) = cs.state.as_ref().and_then(|s| s.terminated.as_ref()) {
                    if term.exit_code == 137 || term.reason.as_deref() == Some("OOMKilled") {
                        return (
                            FailureCategory::App,
                            Some(format!("{} OOMKilled (Exit Code 137)", cs.name)),
                        );
                    }
                    if term.exit_code != 0 {
                        return (
                            FailureCategory::App,
                            Some(format!(
                                "{} terminated with Exit Code {}",
                                cs.name, term.exit_code
                            )),
                        );
                    }
                }
                if let Some(ref waiting) = cs.state.as_ref().and_then(|s| s.waiting.as_ref()) {
                    let reason = waiting.reason.as_deref().unwrap_or("");
                    if reason == "ImagePullBackOff"
                        || reason == "ErrImagePull"
                        || reason == "InvalidImageName"
                    {
                        let detail = waiting
                            .message
                            .clone()
                            .unwrap_or_else(|| format!("{reason} on {}", cs.name));
                        return (FailureCategory::Image, Some(detail));
                    }
                    if reason == "CrashLoopBackOff"
                        || reason == "CreateContainerConfigError"
                        || reason == "CreateContainerError"
                    {
                        let detail = waiting
                            .message
                            .clone()
                            .unwrap_or_else(|| format!("{reason} on {}", cs.name));
                        return (FailureCategory::App, Some(detail));
                    }
                }
            }
        }

        // 2. Pod Conditions (e.g. Unschedulable)
        if let Some(ref conds) = st.conditions {
            for cond in conds {
                if cond.type_ == "PodScheduled" && cond.status == "False" {
                    let msg = cond.message.clone().unwrap_or_default();
                    let lower = msg.to_lowercase();
                    if lower.contains("insufficient")
                        || lower.contains("taint")
                        || lower.contains("affinity")
                        || lower.contains("nodes are available")
                    {
                        return (FailureCategory::Compute, Some(msg));
                    }
                }
            }
        }
    }

    // 3. Pod Warning Events
    for ev in pod_events {
        let reason = ev.reason.as_deref().unwrap_or("");
        let msg = ev.message.as_deref().unwrap_or("");
        let msg_lower = msg.to_lowercase();

        if reason == "FailedScheduling"
            || msg_lower.contains("insufficient cpu")
            || msg_lower.contains("insufficient memory")
            || msg_lower.contains("untolerated taint")
            || msg_lower.contains("node(s) didn't match")
        {
            return (FailureCategory::Compute, Some(msg.to_string()));
        }
        if reason == "FailedMount"
            || reason == "FailedAttachVolume"
            || reason == "VolumeBindingWaiting"
            || msg_lower.contains("persistentvolumeclaim")
            || msg_lower.contains("unbound immediate")
        {
            return (FailureCategory::Storage, Some(msg.to_string()));
        }
        if reason == "FailedCreatePodSandBox"
            || reason == "NetworkNotReady"
            || msg_lower.contains("cni")
            || msg_lower.contains("no ip addresses available")
        {
            return (FailureCategory::Network, Some(msg.to_string()));
        }
        if reason == "Failed" && (msg_lower.contains("image") || msg_lower.contains("pull")) {
            return (FailureCategory::Image, Some(msg.to_string()));
        }
    }

    (FailureCategory::None, None)
}

/// Evaluates deployments, replicasets, pods, events, and GitOps applications to produce
/// an SRE post-page incident triage report.
pub fn evaluate_changed_triage(
    deployments: &[Deployment],
    replicasets: &[ReplicaSet],
    pods: &[Pod],
    events: &[Event],
    argo_apps: &[ArgoApplication],
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
        let mut pending_pod_count = 0;
        let mut primary_symptoms = Vec::new();
        let mut detected_failure_category = FailureCategory::None;
        let mut detected_failure_detail = String::new();

        for p in &current_pods {
            let pod_name = p.metadata.name.clone().unwrap_or_default();
            let mut is_pod_failing = false;

            if let Some(ref st) = p.status {
                if st.phase.as_deref() == Some("Pending") {
                    pending_pod_count += 1;
                    is_pod_failing = true;
                }
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

            // Gather pod events for root cause analysis
            let pod_events = events_by_object
                .get(&("Pod".to_string(), dep_ns.clone(), pod_name.clone()))
                .cloned()
                .unwrap_or_default();
            let (cat, detail) = analyze_pod_failure(p, &pod_events);
            if cat != FailureCategory::None && detected_failure_category == FailureCategory::None {
                detected_failure_category = cat;
                if let Some(d) = detail {
                    detected_failure_detail = d;
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

        // Match ArgoCD Application if present
        let matched_argo = argo_apps.iter().find(|app| {
            app.resources
                .iter()
                .any(|r| r.kind == "Deployment" && r.name == dep_name && r.namespace == dep_ns)
                || dep
                    .metadata
                    .labels
                    .as_ref()
                    .and_then(|l| {
                        l.get("app.kubernetes.io/instance")
                            .or_else(|| l.get("argocd.argoproj.io/instance"))
                    })
                    .map(|val| val == &app.name)
                    .unwrap_or(false)
        });

        let gitops = matched_argo.map(|app| GitOpsReleaseInfo {
            app_name: app.name.clone(),
            sync_status: app.sync_status.clone(),
            health_status: app.health_status.clone(),
            repo_url: app.repo_url.clone(),
            target_revision: app.target_revision.clone(),
            sync_revision: app.sync_revision.chars().take(7).collect(),
            sync_age: if app.last_sync_time.is_empty() {
                "-".to_string()
            } else {
                app.last_sync_time.clone()
            },
            sync_message: if !app.operation_message.is_empty() {
                Some(app.operation_message.clone())
            } else if !app.health_message.is_empty() {
                Some(app.health_message.clone())
            } else {
                None
            },
        });

        // Determine Rollout Status
        let rollout_status = if desired_replicas == 0 {
            RolloutStatus::ScaledDown
        } else if progress_deadline_exceeded {
            RolloutStatus::Stalled
        } else if ready_replicas == desired_replicas
            && updated_replicas == desired_replicas
            && crash_loop_count == 0
            && pending_pod_count == 0
        {
            RolloutStatus::Complete
        } else if !failing_pod_names.is_empty() {
            RolloutStatus::Failed
        } else {
            RolloutStatus::Progressing
        };

        // Determine Incident Status & Failure Category
        let incident_status = if oom_killed_count > 0 {
            IncidentStatus::OomKilled
        } else if crash_loop_count > 0 {
            IncidentStatus::CrashLoop
        } else if pending_pod_count > 0 {
            IncidentStatus::Pending
        } else if progress_deadline_exceeded {
            IncidentStatus::Stalled
        } else if rollout_status == RolloutStatus::Progressing {
            IncidentStatus::Rolling
        } else if rollout_status == RolloutStatus::Complete {
            IncidentStatus::Healthy
        } else if desired_replicas == 0 {
            IncidentStatus::ScaledDown
        } else {
            IncidentStatus::Unknown
        };

        let failure_category = if detected_failure_category != FailureCategory::None {
            detected_failure_category
        } else if oom_killed_count > 0 || crash_loop_count > 0 {
            FailureCategory::App
        } else if pending_pod_count > 0 {
            FailureCategory::Compute
        } else {
            FailureCategory::None
        };

        let failure_detail = if !detected_failure_detail.is_empty() {
            detected_failure_detail
        } else if oom_killed_count > 0 {
            format!("{oom_killed_count} pod(s) OOMKilled (Exit 137)")
        } else if crash_loop_count > 0 {
            format!("{crash_loop_count} pod(s) in CrashLoopBackOff")
        } else if progress_deadline_exceeded {
            "Rollout stalled: ProgressDeadlineExceeded".to_string()
        } else if ready_replicas < desired_replicas {
            format!("{ready_replicas}/{desired_replicas} Ready")
        } else {
            "Healthy".to_string()
        };

        deployment_changes.push(AppDeploymentChange {
            app_name: dep_name,
            kind: "Deployment".to_string(),
            namespace: dep_ns,
            incident_status,
            failure_category,
            failure_detail,
            gitops,
            error_log_snippet: None,
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

    // Sort deployments: CrashLoop / OOM first, then Pending, Stalled, Rolling, Healthy, ScaledDown
    deployment_changes.sort_by(|a, b| {
        let rank = |s: IncidentStatus| match s {
            IncidentStatus::OomKilled => 0,
            IncidentStatus::CrashLoop => 1,
            IncidentStatus::Pending => 2,
            IncidentStatus::Stalled => 3,
            IncidentStatus::Rolling => 4,
            IncidentStatus::Healthy => 5,
            IncidentStatus::ScaledDown => 6,
            IncidentStatus::Unknown => 7,
        };
        rank(a.incident_status).cmp(&rank(b.incident_status))
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
    let crashing_count = deployment_changes
        .iter()
        .filter(|d| {
            d.incident_status == IncidentStatus::CrashLoop
                || d.incident_status == IncidentStatus::OomKilled
        })
        .count();
    let pending_count = deployment_changes
        .iter()
        .filter(|d| d.incident_status == IncidentStatus::Pending)
        .count();
    let rolling_count = deployment_changes
        .iter()
        .filter(|d| d.incident_status == IncidentStatus::Rolling)
        .count();
    let healthy_count = deployment_changes
        .iter()
        .filter(|d| d.incident_status == IncidentStatus::Healthy)
        .count();

    let headline_message = if crashing_count > 0 {
        let first_failing = deployment_changes
            .iter()
            .find(|d| {
                d.incident_status == IncidentStatus::CrashLoop
                    || d.incident_status == IncidentStatus::OomKilled
            })
            .map(|d| format!("{}: {}", d.app_name, d.failure_detail))
            .unwrap_or_default();
        format!("CRITICAL: {first_failing}")
    } else if pending_count > 0 {
        let first_pending = deployment_changes
            .iter()
            .find(|d| d.incident_status == IncidentStatus::Pending)
            .map(|d| format!("{}: {}", d.app_name, d.failure_detail))
            .unwrap_or_default();
        format!("BLOCKED (INFRA): {first_pending}")
    } else if rolling_count > 0 {
        format!("{rolling_count} deployment(s) currently rolling update")
    } else if healthy_count > 0 {
        format!("{healthy_count} deployment(s) healthy")
    } else {
        "No recent changes in window".to_string()
    };

    ChangedTriageReport {
        window_seconds: window_secs,
        window_label: format_duration_label(window),
        namespace,
        summary: TriageSummary {
            total_deployments,
            crashing_count,
            pending_count,
            rolling_count,
            healthy_count,
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
    cache: &Arc<ClientCache>,
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
    let ev_api: Api<Event> = crate::scoped_api(client.clone(), ns_str);

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

    // Query ArgoCD Applications if available
    let argo_apps = match crate::argo::fetch_argo_applications_cached(
        cache,
        context,
        None,
        None,
        None,
        ns_opt.as_deref(),
        true,
        false,
    )
    .await
    {
        Ok(res) => res.filtered_apps,
        Err(_) => Vec::new(),
    };

    let mut report =
        evaluate_changed_triage(&deps, &rs, &pods, &events, &argo_apps, window, now, ns_opt);

    // Fetch inline 5-line error logs for failing deployments
    for dep in &mut report.deployments {
        if !dep.failing_pod_names.is_empty() {
            if let Some(failing_pod_name) = dep.failing_pod_names.first() {
                let pod_client: Api<Pod> = Api::namespaced(client.clone(), &dep.namespace);
                // Try fetching previous terminated container logs first
                let lp_prev = kube::api::LogParams {
                    previous: true,
                    tail_lines: Some(5),
                    timestamps: false,
                    ..Default::default()
                };
                let log_text = match pod_client.logs(failing_pod_name, &lp_prev).await {
                    Ok(text) if !text.trim().is_empty() => Some(text),
                    _ => {
                        let lp_curr = kube::api::LogParams {
                            previous: false,
                            tail_lines: Some(5),
                            timestamps: false,
                            ..Default::default()
                        };
                        pod_client.logs(failing_pod_name, &lp_curr).await.ok()
                    }
                };

                if let Some(raw_log) = log_text {
                    let lines: Vec<String> = raw_log
                        .lines()
                        .map(|l| l.trim_end().to_string())
                        .filter(|l| !l.is_empty())
                        .collect();
                    if !lines.is_empty() {
                        dep.error_log_snippet = Some(lines);
                    }
                }
            }
        }
    }

    Ok(report)
}

/// `k8s.listChanges` — provides holistic deployment & change incident triage for MCP and agents.
pub fn list_changes_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListChangesIn, ChangedTriageReport, _, _>(
        "k8s.listChanges",
        "holistic deployment and change incident triage: evaluates recent rollouts, GitOps release info, pod crash loops, compute/storage blockers, and error log snippets",
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
        ContainerStatus, PodCondition, PodSpec, PodStatus, PodTemplateSpec,
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
    fn triage_detects_deployment_with_crash_loop_and_app_root_cause() {
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
            &[],
            Duration::from_secs(1800), // 30m
            now,
            Some("default".to_string()),
        );

        assert_eq!(report.summary.total_deployments, 1);
        assert_eq!(report.summary.crashing_count, 1);

        let change = &report.deployments[0];
        assert_eq!(change.app_name, "checkout");
        assert_eq!(change.incident_status, IncidentStatus::OomKilled);
        assert_eq!(change.failure_category, FailureCategory::App);
        assert_eq!(change.current_revision, "2");
        assert_eq!(change.previous_revision.as_deref(), Some("1"));
        assert_eq!(change.image_diff, "checkout:v1.0 ➔ checkout:v2.0");
        assert_eq!(change.ready_replicas, 0);
        assert_eq!(change.desired_replicas, 3);
        assert_eq!(change.oom_killed_count, 1);
        assert_eq!(change.crash_loop_count, 1);
        assert!(change.failure_detail.contains("OOMKilled"));
    }

    #[test]
    fn triage_detects_pending_compute_blocker() {
        let now = Timestamp::from_second(1_700_000_000).unwrap();
        let dep_time = Timestamp::from_second(1_700_000_000 - 300).unwrap();

        let dep = Deployment {
            metadata: ObjectMeta {
                name: Some("worker".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: Some(1),
                ..Default::default()
            }),
            status: Some(DeploymentStatus {
                replicas: Some(1),
                updated_replicas: Some(1),
                ready_replicas: Some(0),
                ..Default::default()
            }),
        };

        let current_rs = ReplicaSet {
            metadata: ObjectMeta {
                name: Some("worker-v1".to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("Deployment", "worker")]),
                creation_timestamp: Some(Time(dep_time)),
                ..Default::default()
            },
            spec: Some(ReplicaSetSpec::default()),
            status: Some(ReplicaSetStatus::default()),
        };

        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("worker-v1-987".to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("ReplicaSet", "worker-v1")]),
                ..Default::default()
            },
            spec: Some(PodSpec::default()),
            status: Some(PodStatus {
                phase: Some("Pending".to_string()),
                conditions: Some(vec![PodCondition {
                    type_: "PodScheduled".to_string(),
                    status: "False".to_string(),
                    reason: Some("Unschedulable".to_string()),
                    message: Some("0/3 nodes are available: 3 Insufficient cpu.".to_string()),
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
            &[],
            Duration::from_secs(1800),
            now,
            Some("default".to_string()),
        );

        assert_eq!(report.summary.pending_count, 1);
        let change = &report.deployments[0];
        assert_eq!(change.app_name, "worker");
        assert_eq!(change.incident_status, IncidentStatus::Pending);
        assert_eq!(change.failure_category, FailureCategory::Compute);
        assert!(change.failure_detail.contains("Insufficient cpu"));
    }

    #[test]
    fn triage_detects_pending_storage_blocker_from_event() {
        let now = Timestamp::from_second(1_700_000_000).unwrap();
        let dep_time = Timestamp::from_second(1_700_000_000 - 300).unwrap();

        let dep = Deployment {
            metadata: ObjectMeta {
                name: Some("database".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: Some(1),
                ..Default::default()
            }),
            status: Some(DeploymentStatus {
                replicas: Some(1),
                updated_replicas: Some(1),
                ready_replicas: Some(0),
                ..Default::default()
            }),
        };

        let current_rs = ReplicaSet {
            metadata: ObjectMeta {
                name: Some("database-v1".to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("Deployment", "database")]),
                creation_timestamp: Some(Time(dep_time)),
                ..Default::default()
            },
            spec: Some(ReplicaSetSpec::default()),
            status: Some(ReplicaSetStatus::default()),
        };

        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("database-v1-abc".to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("ReplicaSet", "database-v1")]),
                ..Default::default()
            },
            spec: Some(PodSpec::default()),
            status: Some(PodStatus {
                phase: Some("Pending".to_string()),
                ..Default::default()
            }),
        };

        let event = Event {
            metadata: ObjectMeta {
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            involved_object: k8s_openapi::api::core::v1::ObjectReference {
                kind: Some("Pod".to_string()),
                name: Some("database-v1-abc".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            type_: Some("Warning".to_string()),
            reason: Some("FailedMount".to_string()),
            message: Some("unbound immediate PersistentVolumeClaims data-pvc".to_string()),
            last_timestamp: Some(Time(dep_time)),
            ..Default::default()
        };

        let report = evaluate_changed_triage(
            &[dep],
            &[current_rs],
            &[pod],
            &[event],
            &[],
            Duration::from_secs(1800),
            now,
            Some("default".to_string()),
        );

        assert_eq!(report.summary.pending_count, 1);
        let change = &report.deployments[0];
        assert_eq!(change.failure_category, FailureCategory::Storage);
        assert!(change.failure_detail.contains("PersistentVolumeClaims"));
    }

    #[test]
    fn triage_detects_healthy_deployment() {
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

        let current_rs = ReplicaSet {
            metadata: ObjectMeta {
                name: Some("auth-v5".to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("Deployment", "auth")]),
                creation_timestamp: Some(Time(dep_time)),
                ..Default::default()
            },
            spec: Some(ReplicaSetSpec::default()),
            status: Some(ReplicaSetStatus::default()),
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
            &[],
            Duration::from_secs(1800),
            now,
            Some("default".to_string()),
        );

        assert_eq!(report.summary.healthy_count, 1);
        let change = &report.deployments[0];
        assert_eq!(change.app_name, "auth");
        assert_eq!(change.incident_status, IncidentStatus::Healthy);
        assert_eq!(change.rollout_status, RolloutStatus::Complete);
    }

    #[test]
    fn triage_correlates_argocd_release() {
        let now = Timestamp::from_second(1_700_000_000).unwrap();
        let dep_time = Timestamp::from_second(1_700_000_000 - 300).unwrap();

        let mut labels = BTreeMap::new();
        labels.insert(
            "app.kubernetes.io/instance".to_string(),
            "payment-app".to_string(),
        );

        let dep = Deployment {
            metadata: ObjectMeta {
                name: Some("payment".to_string()),
                namespace: Some("default".to_string()),
                labels: Some(labels),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: Some(1),
                ..Default::default()
            }),
            status: Some(DeploymentStatus {
                replicas: Some(1),
                updated_replicas: Some(1),
                ready_replicas: Some(1),
                available_replicas: Some(1),
                ..Default::default()
            }),
        };

        let current_rs = ReplicaSet {
            metadata: ObjectMeta {
                name: Some("payment-v1".to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("Deployment", "payment")]),
                creation_timestamp: Some(Time(dep_time)),
                ..Default::default()
            },
            spec: Some(ReplicaSetSpec::default()),
            status: Some(ReplicaSetStatus::default()),
        };

        let argo_app = ArgoApplication {
            name: "payment-app".to_string(),
            namespace: "argocd".to_string(),
            uid: "uid-argo-1".to_string(),
            resource_version: "1".to_string(),
            project: "default".to_string(),
            destination_server: "https://kubernetes.default.svc".to_string(),
            destination_name: "".to_string(),
            destination_namespace: "default".to_string(),
            repo_url: "https://github.com/org/repo.git".to_string(),
            target_revision: "main".to_string(),
            path: "manifests".to_string(),
            sync_status: "Synced".to_string(),
            health_status: "Healthy".to_string(),
            health_message: "".to_string(),
            sync_revision: "48697106c123456789".to_string(),
            operation_phase: "".to_string(),
            operation_message: "".to_string(),
            auto_sync_enabled: true,
            self_heal_enabled: true,
            prune_enabled: true,
            last_sync_time: "5m ago".to_string(),
            created_at: "10d ago".to_string(),
            resources: vec![],
            sync_history: vec![],
        };

        let report = evaluate_changed_triage(
            &[dep],
            &[current_rs],
            &[],
            &[],
            &[argo_app],
            Duration::from_secs(1800),
            now,
            Some("default".to_string()),
        );

        let change = &report.deployments[0];
        assert!(change.gitops.is_some());
        let gitops = change.gitops.as_ref().unwrap();
        assert_eq!(gitops.app_name, "payment-app");
        assert_eq!(gitops.sync_status, "Synced");
        assert_eq!(gitops.sync_revision, "4869710");
        assert_eq!(gitops.target_revision, "main");
        assert_eq!(gitops.repo_url, "https://github.com/org/repo.git");
    }
}
