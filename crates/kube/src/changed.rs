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

use k8s_openapi::api::apps::v1::{Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Event, Pod};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::jiff::{SignedDuration, Timestamp};
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError, ReferenceFormat};

use crate::argo::{ArgoApplication, ArgoSyncHistoryItem};
use crate::client_cache::ClientCache;
use crate::connect::request_timeout;
use crate::events::{event_last_timestamp, EventSummary};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum IncidentStatus {
    #[serde(rename = "crashLoop")]
    CrashLoop,
    #[serde(rename = "oomKilled")]
    OomKilled,
    #[serde(rename = "configError")]
    ConfigError,
    #[serde(rename = "imageError")]
    ImageError,
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "flapping")]
    Flapping,
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
            Self::ConfigError => "ConfigError",
            Self::ImageError => "ImageError",
            Self::Pending => "Pending",
            Self::Flapping => "Flapping",
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
            Self::ConfigError => "⚠️",
            Self::ImageError => "🚫",
            Self::Pending => "⏳",
            Self::Flapping => "⚡",
            Self::Stalled => "🚫",
            Self::Rolling => "🔄",
            Self::Healthy => "🟢",
            Self::ScaledDown => "⚪",
            Self::Unknown => "❓",
        }
    }

    pub fn is_incident(&self) -> bool {
        matches!(
            self,
            Self::CrashLoop
                | Self::OomKilled
                | Self::ConfigError
                | Self::ImageError
                | Self::Pending
                | Self::Stalled
                | Self::Flapping
        )
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    /// Whether `sync_message` holds a degraded health message rather than an
    /// operation/sync error.
    #[serde(default, rename = "isHealthMessage")]
    pub is_health_message: bool,
    /// How the workload was matched to the app: `trackingId` (Argo's
    /// `argocd.argoproj.io/tracking-id` annotation), `resources` (the app's
    /// resource list) or `label` (the instance label).
    #[serde(default, rename = "matchedBy")]
    pub matched_by: String,
    /// The Argo sync that produced the workload's current rollout.
    #[serde(default)]
    pub rollout: Option<ArgoRollout>,
    /// Why no sync could be named as the rollout's cause. Set exactly when
    /// `rollout` is `None`.
    #[serde(default, rename = "rolloutUnmatched")]
    pub rollout_unmatched: Option<String>,
}

/// One `status.history` entry of an Argo Application, named as the sync that
/// rolled a workload out, with the entry before it so a reader can ask what
/// changed between the two revisions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArgoRollout {
    #[serde(rename = "historyId")]
    pub history_id: i64,
    /// Full revision: a git SHA, or a chart version when `isChart`.
    pub revision: String,
    #[serde(rename = "previousRevision")]
    pub previous_revision: Option<String>,
    #[serde(rename = "deployedAt")]
    pub deployed_at: String,
    #[serde(rename = "initiatedBy")]
    pub initiated_by: Option<String>,
    #[serde(rename = "repoUrl")]
    pub repo_url: String,
    /// The source path at that sync, which may differ from today's spec.
    pub path: String,
    #[serde(rename = "isChart")]
    pub is_chart: bool,
    /// Matched by "latest sync in the window", not by the time the rollout
    /// was created: StatefulSets and CronJobs expose no such time here.
    pub approximate: bool,
}

/// Why a workload is in the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ChangeKind {
    /// A new rollout (ReplicaSet) or another non-scaling event on the
    /// workload in the window. The default report holds only these.
    #[serde(rename = "rollout")]
    Rollout,
    /// Only its replica count changed (HPA or `kubectl scale`); reported
    /// when [`TriageOptions::include_scaled`] is set.
    #[serde(rename = "scaled")]
    Scaled,
    /// Nothing changed, but it is failing in the window; reported when
    /// [`TriageOptions::include_failing`] is set.
    #[serde(rename = "failing")]
    FailingOnly,
}

impl Default for ChangeKind {
    fn default() -> Self {
        Self::Rollout
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PodIncidentDetail {
    #[serde(rename = "podName")]
    pub pod_name: String,
    pub status: String,
    #[serde(rename = "detailMessage")]
    pub detail_message: String,
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
    #[serde(default, rename = "argoRolloutInWindow")]
    pub argo_rollout_in_window: Option<String>,
    /// The workload's tracking id names an Argo app that could not be read:
    /// Argo was unreachable, or did not list that app. Not the same fact as
    /// "not managed by Argo", which is `gitops: None` with this unset.
    #[serde(default, rename = "gitopsUnresolved")]
    pub gitops_unresolved: Option<String>,
    /// A cause the cluster itself records, e.g. a rollout restart.
    #[serde(default, rename = "localCause")]
    pub local_cause: Option<String>,
    #[serde(rename = "errorLogSnippet")]
    pub error_log_snippet: Option<Vec<String>>,
    /// The pod the snippet was tailed from; not always the first failing
    /// pod, because a Pending or image-pull pod has no logs to tail.
    #[serde(default, rename = "errorLogPod")]
    pub error_log_pod: Option<String>,
    #[serde(default, rename = "errorLogContainer")]
    pub error_log_container: Option<String>,
    /// Why the workload is in the report.
    #[serde(default, rename = "changeKind")]
    pub change_kind: ChangeKind,
    /// When that change happened: the rollout's age, or the latest scale's.
    /// Not `deployed_age`, which is always the last rollout: a Deployment
    /// scaled five minutes ago would otherwise read "66d".
    #[serde(default, rename = "changedAge")]
    pub changed_age: String,
    /// What changed, when it is not a rollout, e.g. "Scaled 3→4".
    #[serde(default, rename = "changeDetail")]
    pub change_detail: Option<String>,
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
    #[serde(default, rename = "podSymptoms")]
    pub pod_symptoms: Vec<PodIncidentDetail>,
    #[serde(rename = "failingPodNames")]
    pub failing_pod_names: Vec<String>,
    #[serde(rename = "topEvents")]
    pub top_events: Vec<EventSummary>,
}

impl AppDeploymentChange {
    pub fn is_incident(&self) -> bool {
        self.incident_status.is_incident()
    }
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
    #[serde(default, rename = "oomCount")]
    pub oom_count: usize,
    #[serde(default, rename = "errorCount")]
    pub error_count: usize,
    #[serde(rename = "pendingCount")]
    pub pending_count: usize,
    #[serde(default, rename = "flappingCount")]
    pub flapping_count: usize,
    #[serde(rename = "rollingCount")]
    pub rolling_count: usize,
    #[serde(rename = "healthyCount")]
    pub healthy_count: usize,
    #[serde(rename = "headlineMessage")]
    pub headline_message: String,
}

/// What a triage run includes and fetches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriageOptions {
    /// Also report workloads that did not change in the window but are
    /// failing in it (a Warning on a pod or the current ReplicaSet). They
    /// are marked [`ChangeKind::FailingOnly`]. Enabled by default.
    pub include_failing: bool,
    /// Also report Deployments whose only change in the window is a replica
    /// count (HPA or `kubectl scale`), marked [`ChangeKind::Scaled`]. Enabled by default.
    pub include_scaled: bool,
    /// Tail a five-line log snippet per failing workload. The TUI leaves
    /// this off (its card links to the full logs); MCP callers get it.
    pub log_snippets: bool,
}

impl Default for TriageOptions {
    fn default() -> Self {
        Self {
            include_failing: true,
            include_scaled: true,
            log_snippets: false,
        }
    }
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
    /// Whether failing-but-unchanged workloads were included, so a reader
    /// can tell a stale answer from a current one.
    #[serde(default, rename = "includesFailing")]
    pub includes_failing: bool,
    #[serde(default, rename = "includesScaled")]
    pub includes_scaled: bool,
    /// How complete the Argo data behind the GitOps fields is, so a reader
    /// can tell "not managed by Argo" from "Argo data not all in yet".
    #[serde(default)]
    pub argo: ArgoCoverage,
}

/// How much of the Argo picture a report was matched against.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ArgoCoverageState {
    /// Every Application Argo lists for this cluster, fetched recently.
    #[serde(rename = "complete")]
    Complete,
    /// Some Applications are still loading, or the listing stopped early.
    #[serde(rename = "partial")]
    Partial,
    /// A complete list, but older than [`ARGO_STALE_AFTER`], or kept after a
    /// refresh failed.
    #[serde(rename = "stale")]
    Stale,
    /// Argo could not be read, and nothing is known.
    #[serde(rename = "unavailable")]
    Unavailable,
    /// No Argo CD manages this cluster.
    #[default]
    #[serde(rename = "none")]
    NoArgo,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArgoCoverage {
    pub state: ArgoCoverageState,
    #[serde(default, rename = "fetchedAt")]
    pub fetched_at: Option<String>,
    #[serde(default, rename = "appsLoaded")]
    pub apps_loaded: usize,
    /// The hub cluster the Applications were read from, when not this one.
    #[serde(default)]
    pub hub: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Maximum window for triage query (30 days).
pub const MAX_DURATION_SECS: u64 = 30 * 86400;

pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim().to_ascii_lowercase();
    let secs = if let Some(stripped) = s.strip_suffix('m') {
        let mins: u64 = stripped
            .parse()
            .map_err(|_| format!("Invalid minutes in duration: {s}"))?;
        mins.checked_mul(60)
            .ok_or_else(|| format!("Duration value overflow: {s}"))?
    } else if let Some(stripped) = s.strip_suffix('h') {
        let hrs: u64 = stripped
            .parse()
            .map_err(|_| format!("Invalid hours in duration: {s}"))?;
        hrs.checked_mul(3600)
            .ok_or_else(|| format!("Duration value overflow: {s}"))?
    } else if let Some(stripped) = s.strip_suffix('d') {
        let days: u64 = stripped
            .parse()
            .map_err(|_| format!("Invalid days in duration: {s}"))?;
        days.checked_mul(86400)
            .ok_or_else(|| format!("Duration value overflow: {s}"))?
    } else if let Some(stripped) = s.strip_suffix('s') {
        stripped
            .parse::<u64>()
            .map_err(|_| format!("Invalid seconds in duration: {s}"))?
    } else if let Ok(mins) = s.parse::<u64>() {
        mins.checked_mul(60)
            .ok_or_else(|| format!("Duration value overflow: {s}"))?
    } else {
        return Err(format!(
            "Unrecognized time window '{s}'. Expected e.g. 15m, 30m, 1h, 3h, 24h"
        ));
    };

    if secs > MAX_DURATION_SECS {
        return Err(format!(
            "Duration exceeds maximum supported window of 30d ({}s): {s}",
            MAX_DURATION_SECS
        ));
    }
    Ok(Duration::from_secs(secs))
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

fn extract_sts_container_images(sts: &StatefulSet) -> Vec<String> {
    sts.spec
        .as_ref()
        .and_then(|s| s.template.spec.as_ref())
        .map(|ps| {
            ps.containers
                .iter()
                .filter_map(|c| c.image.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn extract_cronjob_container_images(cj: &CronJob) -> Vec<String> {
    cj.spec
        .as_ref()
        .and_then(|s| s.job_template.spec.as_ref())
        .and_then(|js| js.template.spec.as_ref())
        .map(|ps| {
            ps.containers
                .iter()
                .filter_map(|c| c.image.clone())
                .collect()
        })
        .unwrap_or_default()
}

pub fn is_noisy_normal_event(reason: &str, event_type: Option<&str>) -> bool {
    if event_type == Some("Warning") {
        return false;
    }
    matches!(
        reason,
        "ScalingReplicaSet"
            | "SuccessfulCreate"
            | "SuccessfulDelete"
            | "Pulling"
            | "Pulled"
            | "Created"
            | "Started"
            | "Scheduled"
            | "SandboxChanged"
    )
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

/// The pod template annotation `kubectl rollout restart` (and Argo's
/// Restart action) stamps to force a new ReplicaSet.
const RESTARTED_AT: &str = "kubectl.kubernetes.io/restartedAt";

/// "rollout restart at T" when the current ReplicaSet differs from the
/// previous one only by its restart stamp. A stamp that arrived alongside a
/// spec change does not explain the rollout, so it is not reported.
fn restart_cause(current: &ReplicaSet, previous: Option<&ReplicaSet>) -> Option<String> {
    let template = |rs: &ReplicaSet| rs.spec.as_ref()?.template.clone();
    let stamp = |rs: &ReplicaSet| {
        template(rs)?
            .metadata?
            .annotations?
            .get(RESTARTED_AT)
            .cloned()
    };
    let previous = previous?;
    let stamped = stamp(current)?;
    let same_spec =
        template(current).and_then(|t| t.spec) == template(previous).and_then(|t| t.spec);
    (same_spec && stamp(previous).as_ref() != Some(&stamped))
        .then(|| format!("rollout restart at {stamped}"))
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

/// "Scaled 3→4" from a Deployment `ScalingReplicaSet` message. Kubernetes
/// writes "Scaled up replica set web-7f from 3 to 4" (newer) or "Scaled up
/// replica set web-7f to 4" (older); anything else reads "Scaled".
pub fn parse_scale_event(message: &str) -> String {
    let words: Vec<&str> = message.split_whitespace().collect();
    let num = |i: usize| {
        words
            .get(i)
            .map(|w| w.trim_end_matches(|c: char| !c.is_ascii_digit()))
            .filter(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_digit()))
    };
    if let Some(i) = words.iter().rposition(|w| *w == "to") {
        if let Some(to) = num(i + 1) {
            if i >= 2 && words[i - 2] == "from" {
                if let Some(from) = num(i - 1) {
                    return format!("Scaled {from}→{to}");
                }
            }
            return format!("Scaled to {to}");
        }
    }
    "Scaled".to_string()
}

/// Whether any of `events` is a Warning that last fired at or after `cutoff`.
fn warning_in_window(events: Option<&Vec<&Event>>, cutoff: &Timestamp) -> bool {
    events.into_iter().flatten().any(|ev| {
        ev.type_.as_deref() == Some("Warning")
            && event_last_timestamp(ev).is_some_and(|t| t >= *cutoff)
    })
}

/// Whether a Warning fired on any of `pods` inside the window. A workload
/// rolled out days ago that started crash-looping tonight has no new
/// ReplicaSet, but its pods' BackOff warnings are recent, and that is the
/// workload the page is about. Pod creation alone is not counted: an HPA
/// scale-up or a rescheduled pod is not an incident.
fn pods_warned_in_window(
    pods: &[&Pod],
    events_by_object: &HashMap<(String, String, String), Vec<&Event>>,
    ns: &str,
    cutoff: &Timestamp,
) -> bool {
    pods.iter().any(|p| {
        let name = p.metadata.name.clone().unwrap_or_default();
        warning_in_window(
            events_by_object.get(&("Pod".to_string(), ns.to_string(), name)),
            cutoff,
        )
    })
}

/// The container whose logs explain a pod's failure: the first one that is
/// waiting on an error, terminated non-zero now or last time, else the first
/// that has restarted, else the pod's first container. The log API refuses a
/// multi-container pod without a container name, so this must be named.
pub fn failing_container_name(pod: &Pod) -> Option<String> {
    use k8s_openapi::api::core::v1::ContainerState;
    let exited_non_zero = |st: Option<&ContainerState>| {
        st.and_then(|s| s.terminated.as_ref())
            .is_some_and(|t| t.exit_code != 0)
    };
    let statuses = pod
        .status
        .as_ref()
        .and_then(|s| s.container_statuses.as_ref());
    if let Some(list) = statuses {
        let failing = list
            .iter()
            .find(|cs| {
                let waiting_on_error = cs
                    .state
                    .as_ref()
                    .and_then(|s| s.waiting.as_ref())
                    .and_then(|w| w.reason.as_deref())
                    .is_some_and(|r| r != "ContainerCreating" && r != "PodInitializing");
                waiting_on_error
                    || exited_non_zero(cs.state.as_ref())
                    || exited_non_zero(cs.last_state.as_ref())
            })
            .or_else(|| list.iter().find(|cs| cs.restart_count > 0));
        if let Some(cs) = failing {
            return Some(cs.name.clone());
        }
    }
    pod.spec
        .as_ref()
        .and_then(|s| s.containers.first())
        .map(|c| c.name.clone())
}

/// Pod statuses whose container actually ran, so a log tail has something to
/// show. Pending, image-pull and config-error pods never started a process.
fn status_has_logs(status: &str) -> bool {
    matches!(status, "CrashLoopBackOff" | "OOMKilled" | "Error")
}

/// The pod and container whose logs explain a workload's failure: the first
/// failing pod that actually ran a process, and its failing container.
/// Chosen here, from the pods already listed, so `l` and Quick AI RCA know
/// where to look without a log call on every refresh.
fn log_target(symptoms: &[PodIncidentDetail], pods: &[&Pod]) -> (Option<String>, Option<String>) {
    let Some(sym) = symptoms.iter().find(|s| status_has_logs(&s.status)) else {
        return (None, None);
    };
    let container = pods
        .iter()
        .find(|p| p.metadata.name.as_deref() == Some(sym.pod_name.as_str()))
        .and_then(|p| failing_container_name(p));
    (Some(sym.pod_name.clone()), container)
}

/// Argo CD's resource tracking annotation.
const TRACKING_ID: &str = "argocd.argoproj.io/tracking-id";

/// Longest gap between a rollout's creation and the end of the sync that
/// made it, used when Argo recorded no start time for the sync.
const SYNC_SKEW: SignedDuration = SignedDuration::from_secs(15 * 60);

/// Clock difference allowed between Argo's controller, which stamps the
/// history, and the API server, which stamps the ReplicaSet.
const CLOCK_SLACK: SignedDuration = SignedDuration::from_secs(60);

/// How a row's `gitops_unresolved` ends when Argo answered without the app;
/// [`apply_argo_coverage`] replaces it when the answer was not all of Argo.
const NOT_LISTED: &str = ", which Argo did not list";

/// A complete Argo list older than this is reported as stale.
const ARGO_STALE_AFTER: SignedDuration =
    SignedDuration::from_secs(crate::argo::ARGO_FRESH_FOR.as_secs() as i64);

/// What a rollout newer than the Argo data says instead of "no Argo sync
/// around this rollout": its sync cannot be in that history yet.
const ARGO_OLDER_THAN_ROLLOUT: &str = "Argo data is older than this rollout; refreshing";

/// How complete `snapshot` is, as the report states it.
pub fn argo_coverage(snapshot: &ArgoSnapshot, now: Timestamp) -> ArgoCoverage {
    let old = snapshot
        .fetched_at
        .is_some_and(|t| now.duration_since(t) > ARGO_STALE_AFTER);
    let state = match (&snapshot.error, snapshot.apps.is_empty()) {
        (Some(e), _) if e == crate::argo::NO_ARGO => ArgoCoverageState::NoArgo,
        (Some(_), true) => ArgoCoverageState::Unavailable,
        _ if !snapshot.complete => ArgoCoverageState::Partial,
        (Some(_), false) => ArgoCoverageState::Stale,
        _ if old => ArgoCoverageState::Stale,
        _ => ArgoCoverageState::Complete,
    };
    ArgoCoverage {
        state,
        fetched_at: snapshot.fetched_at.map(|t| t.to_string()),
        apps_loaded: snapshot.apps.len(),
        hub: snapshot.hub.clone(),
        error: snapshot.error.clone(),
    }
}

/// Put `coverage` on the report and keep every row honest about it: with
/// part of Argo, or none, a row without an app has an unknown owner, not no
/// owner; and a rollout newer than the data has a sync the data cannot hold.
fn apply_argo_coverage(report: &mut ChangedTriageReport, coverage: ArgoCoverage) {
    let fetched_at = coverage
        .fetched_at
        .as_deref()
        .and_then(|t| t.parse::<Timestamp>().ok());
    let n = coverage.apps_loaded;
    let err = coverage.error.as_deref().unwrap_or("unknown error");
    // (after a tracking id's app name, for a row with no app at all)
    let pending = match coverage.state {
        ArgoCoverageState::Partial => Some((
            format!("the Argo list is still loading ({n} apps so far)"),
            format!("Argo list incomplete ({n} apps loaded); ownership not known yet"),
        )),
        ArgoCoverageState::Unavailable => Some((
            format!("Argo unavailable: {err}"),
            format!("Argo unavailable: {err}; ownership not known"),
        )),
        _ => None,
    };
    for d in &mut report.deployments {
        if let Some((after_app, unowned)) = &pending {
            match d.gitops_unresolved.as_mut() {
                Some(msg) => {
                    if let Some(head) = msg.strip_suffix(NOT_LISTED) {
                        *msg = format!("{head}; {after_app}");
                    }
                }
                None if d.gitops.is_none() => d.gitops_unresolved = Some(unowned.clone()),
                None => {}
            }
        }
        let created = d
            .deployed_at
            .as_deref()
            .and_then(|t| t.parse::<Timestamp>().ok());
        if let (Some(g), Some(at), Some(created), "Deployment") =
            (d.gitops.as_mut(), fetched_at, created, d.kind.as_str())
        {
            if g.rollout.is_none() && created > at {
                g.rollout_unmatched = Some(ARGO_OLDER_THAN_ROLLOUT.to_string());
            }
        }
    }
    report.argo = coverage;
}

/// The workload a [`gitops_release_for`] lookup is about.
struct Workload<'a> {
    api_version: &'a str,
    kind: &'a str,
    name: &'a str,
    ns: &'a str,
    meta: &'a ObjectMeta,
    /// When the current rollout was created: a Deployment's current
    /// ReplicaSet. `None` where this report holds no such object.
    rollout_created: Option<Timestamp>,
}

/// What [`gitops_release_for`] found for one workload.
#[derive(Default)]
struct GitOpsMatch {
    info: Option<GitOpsReleaseInfo>,
    rollout_in_window: Option<String>,
    unresolved: Option<String>,
}

/// The index of the `history` entry whose sync created a rollout at
/// `created`: the sync running at that moment, else the first to finish
/// within [`SYNC_SKEW`] after it. `None` when no sync accounts for it — a
/// change made outside Argo, or a sync already trimmed from the history.
pub fn causal_history_entry(history: &[ArgoSyncHistoryItem], created: Timestamp) -> Option<usize> {
    let parse = |s: &str| s.parse::<Timestamp>().ok();
    let earliest =
        |hits: Vec<(usize, Timestamp)>| hits.into_iter().min_by_key(|(_, d)| *d).map(|(i, _)| i);

    let during = history
        .iter()
        .enumerate()
        .filter_map(|(i, h)| {
            let deployed = parse(&h.deployed_at)?;
            let started = parse(&h.deploy_started_at)?;
            (created.duration_since(started) >= -CLOCK_SLACK
                && deployed.duration_since(created) >= -CLOCK_SLACK)
                .then_some((i, deployed))
        })
        .collect();
    if let Some(i) = earliest(during) {
        return Some(i);
    }

    let after = history
        .iter()
        .enumerate()
        .filter_map(|(i, h)| {
            let deployed = parse(&h.deployed_at)?;
            let gap = deployed.duration_since(created);
            (gap >= -CLOCK_SLACK && gap <= SYNC_SKEW).then_some((i, deployed))
        })
        .collect();
    earliest(after)
}

/// The sync behind a workload's current rollout, or why none can be named.
fn rollout_for(
    history: &[ArgoSyncHistoryItem],
    created: Option<Timestamp>,
    cutoff: Option<&Timestamp>,
) -> Result<ArgoRollout, String> {
    let (idx, approximate) = match created {
        Some(created) => (
            causal_history_entry(history, created).ok_or_else(|| {
                "no Argo sync around this rollout: a change made outside Argo, \
                 or a sync older than the app's history"
                    .to_string()
            })?,
            false,
        ),
        None => {
            let latest = history
                .iter()
                .enumerate()
                .filter_map(|(i, h)| Some((i, h.deployed_at.parse::<Timestamp>().ok()?)))
                .filter(|(_, d)| cutoff.is_some_and(|c| d >= c))
                .max_by_key(|(_, d)| *d)
                .map(|(i, _)| i);
            (
                latest.ok_or_else(|| "no Argo sync in this window".to_string())?,
                true,
            )
        }
    };
    let entry = &history[idx];
    // Only a sync of the same source bounds a meaningful range of changes.
    let previous_revision = history
        .iter()
        .filter(|h| h.id < entry.id && h.repo_url == entry.repo_url && h.is_chart == entry.is_chart)
        .max_by_key(|h| h.id)
        .map(|h| h.revision.clone())
        .filter(|r| !r.is_empty() && r != &entry.revision);
    Ok(ArgoRollout {
        history_id: entry.id,
        revision: entry.revision.clone(),
        previous_revision,
        deployed_at: entry.deployed_at.clone(),
        initiated_by: entry.initiated_by.clone(),
        repo_url: entry.repo_url.clone(),
        path: entry.path.clone(),
        is_chart: entry.is_chart,
        approximate,
    })
}

/// A revision as a reader scans it: seven characters of a git SHA, a chart
/// version whole.
fn short_revision(rev: &str, is_chart: bool) -> String {
    if is_chart {
        rev.to_string()
    } else {
        rev.chars().take(7).collect()
    }
}

/// Match a workload to the ArgoCD Application that manages it, describe its
/// last sync, and name the sync behind its current rollout.
///
/// A tracking id that names the workload decides the app, because Argo
/// manages a resource by that id. When it names an app Argo did not list,
/// the answer is "unresolved", not an app guessed from labels. Without one,
/// the app's resource list decides, then the instance label.
///
/// `rollout_in_window` is set only when that sync finished inside the
/// window: "a release landed during the incident" is a claim about time, and
/// an app that last synced a week ago must not make it.
fn gitops_release_for(
    argo_apps: &[ArgoApplication],
    w: &Workload,
    now: Timestamp,
    cutoff: Option<&Timestamp>,
) -> GitOpsMatch {
    let tracked = w
        .meta
        .annotations
        .as_ref()
        .and_then(|a| a.get(TRACKING_ID))
        .and_then(|id| {
            let obj = serde_json::json!({
                "apiVersion": w.api_version,
                "kind": w.kind,
                "metadata": { "name": w.name, "namespace": w.ns },
            });
            ReferenceFormat::ArgocdTrackingId.owner(id, &obj)
        });
    let (app, matched_by) = if let Some((app_ns, app_name)) = &tracked {
        let found = argo_apps
            .iter()
            .find(|a| &a.name == app_name && app_ns.as_ref().is_none_or(|n| n == &a.namespace));
        match found {
            Some(app) => (app, "trackingId"),
            None => {
                let label = match app_ns {
                    Some(n) => format!("{n}/{app_name}"),
                    None => app_name.clone(),
                };
                return GitOpsMatch {
                    unresolved: Some(format!("tracking id names Argo app {label}{NOT_LISTED}")),
                    ..Default::default()
                };
            }
        }
    } else {
        let instance = w.meta.labels.as_ref().and_then(|l| {
            l.get("app.kubernetes.io/instance")
                .or_else(|| l.get("argocd.argoproj.io/instance"))
        });
        let by_resource = argo_apps.iter().find(|app| {
            app.resources
                .iter()
                .any(|r| r.kind == w.kind && r.name == w.name && r.namespace == w.ns)
        });
        if let Some(app) = by_resource {
            (app, "resources")
        } else if let Some(app) = instance.and_then(|v| argo_apps.iter().find(|a| &a.name == v)) {
            (app, "label")
        } else {
            return GitOpsMatch::default();
        }
    };

    let short_rev: String = app.sync_revision.chars().take(7).collect();
    let synced_at = app.last_sync_time.parse::<Timestamp>().ok();
    let age_since = |ts: Timestamp| crate::format_age(now.duration_since(ts).as_secs().max(0));
    let sync_age = match synced_at {
        Some(ts) => age_since(ts),
        None if app.last_sync_time.is_empty() => "-".to_string(),
        None => app.last_sync_time.clone(),
    };

    let (rollout, rollout_unmatched) = if app.sync_history.is_empty() {
        (None, Some("the app records no sync history".to_string()))
    } else {
        match rollout_for(&app.sync_history, w.rollout_created, cutoff) {
            Ok(r) => (Some(r), None),
            Err(why) => (None, Some(why)),
        }
    };

    let in_window = |ts: Option<Timestamp>| match (ts, cutoff) {
        (Some(ts), Some(cutoff)) => ts >= *cutoff,
        _ => false,
    };
    let rollout_in_window = match &rollout {
        Some(r) => {
            let deployed = r.deployed_at.parse::<Timestamp>().ok();
            in_window(deployed).then(|| {
                format!(
                    "rev {} synced {} ago",
                    short_revision(&r.revision, r.is_chart),
                    deployed.map(age_since).unwrap_or_default()
                )
            })
        }
        // Without history the last sync is all Argo tells us.
        None if app.sync_history.is_empty() && in_window(synced_at) && !short_rev.is_empty() => {
            Some(format!("rev {short_rev} synced {sync_age} ago"))
        }
        None => None,
    };

    let (sync_message, is_health_message) = if !app.operation_message.is_empty()
        && !app.operation_message.starts_with("successfully synced")
        && (app.operation_phase == "Failed"
            || app.operation_phase == "Error"
            || app.sync_status != "Synced")
    {
        (Some(app.operation_message.clone()), false)
    } else if !app.health_message.is_empty() && app.health_status != "Healthy" {
        (Some(app.health_message.clone()), true)
    } else {
        (None, false)
    };

    let info = GitOpsReleaseInfo {
        matched_by: matched_by.to_string(),
        rollout,
        rollout_unmatched,
        app_name: app.name.clone(),
        sync_status: app.sync_status.clone(),
        health_status: app.health_status.clone(),
        repo_url: app.repo_url.clone(),
        target_revision: app.target_revision.clone(),
        sync_revision: short_rev,
        sync_age,
        sync_message,
        is_health_message,
    };
    GitOpsMatch {
        info: Some(info),
        rollout_in_window,
        unresolved: None,
    }
}

fn evaluate_pod_failures(
    pods: &[&Pod],
    events_by_object: &HashMap<(String, String, String), Vec<&Event>>,
    ns: &str,
) -> (
    Vec<String>,
    usize,
    usize,
    usize,
    usize,
    usize,
    i32,
    Vec<String>,
    Vec<PodIncidentDetail>,
    FailureCategory,
    String,
) {
    let mut failing_pod_names = Vec::new();
    let mut crash_loop_count = 0;
    let mut oom_killed_count = 0;
    let mut config_error_count = 0;
    let mut image_error_count = 0;
    let mut pending_pod_count = 0;
    let mut restart_count = 0;
    let mut primary_symptoms = Vec::new();
    let mut pod_symptoms = Vec::new();
    let mut detected_failure_category = FailureCategory::None;
    let mut detected_failure_detail = String::new();

    for p in pods {
        let pod_name = p.metadata.name.clone().unwrap_or_default();
        let mut is_pod_failing = false;
        let mut pod_status_label = String::new();
        let mut pod_error_msg = String::new();
        let mut exited_non_zero = false;

        if let Some(ref st) = p.status {
            if st.phase.as_deref() == Some("Pending") {
                pending_pod_count += 1;
                is_pod_failing = true;
                pod_status_label = "Pending".to_string();
            }
            if let Some(ref c_statuses) = st.container_statuses {
                for cs in c_statuses {
                    restart_count += cs.restart_count;
                    if let Some(ref waiting) = cs.state.as_ref().and_then(|s| s.waiting.as_ref()) {
                        let reason = waiting.reason.as_deref().unwrap_or("");
                        if reason == "CrashLoopBackOff" {
                            is_pod_failing = true;
                            crash_loop_count += 1;
                            pod_status_label = "CrashLoopBackOff".to_string();
                            if let Some(ref msg) = waiting.message {
                                pod_error_msg = msg.clone();
                            }
                        } else if reason == "CreateContainerConfigError"
                            || reason == "CreateContainerError"
                        {
                            is_pod_failing = true;
                            config_error_count += 1;
                            pod_status_label = reason.to_string();
                            pod_error_msg = waiting
                                .message
                                .clone()
                                .unwrap_or_else(|| reason.to_string());
                        } else if reason == "ImagePullBackOff"
                            || reason == "ErrImagePull"
                            || reason == "InvalidImageName"
                        {
                            is_pod_failing = true;
                            image_error_count += 1;
                            pod_status_label = reason.to_string();
                            pod_error_msg = waiting
                                .message
                                .clone()
                                .unwrap_or_else(|| reason.to_string());
                        }
                    }
                    if let Some(ref term) =
                        cs.last_state.as_ref().and_then(|s| s.terminated.as_ref())
                    {
                        if term.exit_code == 137 || term.reason.as_deref() == Some("OOMKilled") {
                            is_pod_failing = true;
                            oom_killed_count += 1;
                            pod_status_label = "OOMKilled".to_string();
                            pod_error_msg = "exit code 137".to_string();
                        } else if term.exit_code != 0 {
                            is_pod_failing = true;
                            exited_non_zero = true;
                            if pod_status_label.is_empty() {
                                pod_status_label = "Error".to_string();
                            }
                            if pod_status_label == "CrashLoopBackOff"
                                || pod_error_msg.is_empty()
                                || pod_error_msg.starts_with("back-off")
                            {
                                pod_error_msg = format!("exited with code {}", term.exit_code);
                            }
                        }
                    }
                    if let Some(ref term) = cs.state.as_ref().and_then(|s| s.terminated.as_ref()) {
                        if term.exit_code == 137 || term.reason.as_deref() == Some("OOMKilled") {
                            is_pod_failing = true;
                            oom_killed_count += 1;
                            pod_status_label = "OOMKilled".to_string();
                            pod_error_msg = "exit code 137".to_string();
                        } else if term.exit_code != 0 {
                            is_pod_failing = true;
                            exited_non_zero = true;
                            if pod_status_label.is_empty() {
                                pod_status_label = "Error".to_string();
                            }
                            if pod_status_label == "CrashLoopBackOff"
                                || pod_error_msg.is_empty()
                                || pod_error_msg.starts_with("back-off")
                            {
                                pod_error_msg = format!("exited with code {}", term.exit_code);
                            }
                        }
                    }
                }
            }
        }

        let pod_events = events_by_object
            .get(&("Pod".to_string(), ns.to_string(), pod_name.clone()))
            .cloned()
            .unwrap_or_default();
        let (cat, detail) = analyze_pod_failure(p, &pod_events);
        // Only a pod failing now explains the workload. A pod unschedulable
        // forty minutes ago keeps its FailedScheduling event for about an
        // hour after it was placed; read from a Running pod, that event
        // gave a healthy 3/3 Deployment a [COMPUTE] root cause.
        if is_pod_failing
            && cat != FailureCategory::None
            && detected_failure_category == FailureCategory::None
        {
            detected_failure_category = cat;
            if let Some(ref d) = detail {
                detected_failure_detail = d.clone();
            }
        }
        if pod_status_label == "Pending" && pod_error_msg.is_empty() {
            if let Some(d) = detail {
                pod_error_msg = d;
            }
        }

        // Between crashes a container is Running with a non-zero last exit.
        // That is still a crash loop; without counting it the workload's
        // status fell through to Unknown and flickered on every refresh.
        if exited_non_zero && pod_status_label == "Error" {
            crash_loop_count += 1;
        }

        if is_pod_failing {
            if pod_status_label.is_empty() {
                pod_status_label = "Error".to_string();
            }
            if pod_error_msg.is_empty() {
                pod_error_msg = "unknown error".to_string();
            }
            let sym_str = format!("{pod_name}: {pod_status_label} | {pod_error_msg}");
            if !primary_symptoms.contains(&sym_str) {
                primary_symptoms.push(sym_str);
            }
            pod_symptoms.push(PodIncidentDetail {
                pod_name: pod_name.clone(),
                status: pod_status_label,
                detail_message: pod_error_msg,
            });
            if !failing_pod_names.contains(&pod_name) {
                failing_pod_names.push(pod_name);
            }
        }
    }

    (
        failing_pod_names,
        crash_loop_count,
        oom_killed_count,
        config_error_count,
        image_error_count,
        pending_pod_count,
        restart_count,
        primary_symptoms,
        pod_symptoms,
        detected_failure_category,
        detected_failure_detail,
    )
}

/// Evaluates deployments, statefulsets, cronjobs, replicasets, pods, events, and GitOps applications
/// to produce an SRE post-page incident triage report.
pub fn evaluate_changed_triage(
    deployments: &[Deployment],
    statefulsets: &[StatefulSet],
    cronjobs: &[CronJob],
    jobs: &[Job],
    replicasets: &[ReplicaSet],
    pods: &[Pod],
    events: &[Event],
    argo_apps: &[ArgoApplication],
    window: Duration,
    now: Timestamp,
    namespace: Option<String>,
    opts: TriageOptions,
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

    // 2. Index Pods by owner ReplicaSet and StatefulSet
    let mut pods_by_rs: HashMap<(String, String), Vec<&Pod>> = HashMap::new();
    let mut pods_by_sts: HashMap<(String, String), Vec<&Pod>> = HashMap::new();
    let mut pods_by_job: HashMap<(String, String), Vec<&Pod>> = HashMap::new();

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
            } else if owner.kind == "StatefulSet" {
                pods_by_sts
                    .entry((ns.clone(), owner.name.clone()))
                    .or_default()
                    .push(pod);
            } else if owner.kind == "Job" {
                pods_by_job
                    .entry((ns.clone(), owner.name.clone()))
                    .or_default()
                    .push(pod);
            }
        }
    }

    // Index Jobs by owner CronJob
    let mut jobs_by_cj: HashMap<(String, String), Vec<&Job>> = HashMap::new();
    for job in jobs {
        let ns = job.metadata.namespace.clone().unwrap_or_default();
        if let Some(ref target_ns) = namespace {
            if !target_ns.is_empty() && &ns != target_ns {
                continue;
            }
        }
        for owner in job.metadata.owner_references.iter().flatten() {
            if owner.kind == "CronJob" {
                jobs_by_cj
                    .entry((ns.clone(), owner.name.clone()))
                    .or_default()
                    .push(job);
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
    // Every object whose events already sit on a workload's card, so the
    // Infra tab does not repeat them. Keyed by kind as well as name: a
    // ReplicaSet or Pod never shares its owner's name.
    let mut tracked_objects: std::collections::HashSet<(String, String, String)> =
        std::collections::HashSet::new();

    // 4a. Examine each Deployment
    for dep in deployments {
        let dep_ns = dep.metadata.namespace.clone().unwrap_or_default();
        if let Some(ref target_ns) = namespace {
            if !target_ns.is_empty() && &dep_ns != target_ns {
                continue;
            }
        }
        let dep_name = dep.metadata.name.clone().unwrap_or_default();

        let mut owned_rs = rs_by_deployment
            .get(&(dep_ns.clone(), dep_name.clone()))
            .cloned()
            .unwrap_or_default();
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

        let mut deployed_at = None;
        let mut deployed_age = "-".to_string();
        let mut rolled_out = false;

        if let Some(rs) = current_rs {
            if let Some(ref ct) = rs.metadata.creation_timestamp {
                deployed_at = Some(ct.0.to_string());
                deployed_age = crate::format_age(now.duration_since(ct.0).as_secs().max(0));
                if let Some(ref cutoff) = cutoff_ts {
                    if ct.0 >= *cutoff {
                        rolled_out = true;
                    }
                }
            }
        }

        let dep_events = events_by_object
            .get(&("Deployment".to_string(), dep_ns.clone(), dep_name.clone()))
            .cloned()
            .unwrap_or_default();

        // The latest in-window Deployment event of each sort. A scale is not
        // a rollout: an HPA emits ScalingReplicaSet on every step, and
        // counting it put most of an autoscaled cluster in a 1h "changed"
        // list with its months-old rollout age beside it.
        let latest_in_window = |scaling: bool| {
            dep_events
                .iter()
                .filter(|ev| (ev.reason.as_deref() == Some("ScalingReplicaSet")) == scaling)
                .filter_map(|ev| event_last_timestamp(ev).map(|t| (t, *ev)))
                .filter(|(t, _)| cutoff_ts.as_ref().is_some_and(|c| t >= c))
                .max_by_key(|(t, _)| *t)
        };
        let age_of = |t: Timestamp| crate::format_age(now.duration_since(t).as_secs().max(0));

        let current_rs_name = current_rs
            .and_then(|rs| rs.metadata.name.as_deref())
            .unwrap_or("");
        let current_pods = pods_by_rs
            .get(&(dep_ns.clone(), current_rs_name.to_string()))
            .cloned()
            .unwrap_or_default();

        let desired_replicas = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
        let status = dep.status.as_ref();
        let updated_replicas = status.and_then(|s| s.updated_replicas).unwrap_or(0);
        let ready_replicas = status.and_then(|s| s.ready_replicas).unwrap_or(0);
        let available_replicas = status.and_then(|s| s.available_replicas).unwrap_or(0);

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

        let (
            failing_pod_names,
            crash_loop_count,
            oom_killed_count,
            config_error_count,
            image_error_count,
            pending_pod_count,
            restart_count,
            primary_symptoms,
            pod_symptoms,
            detected_failure_category,
            detected_failure_detail,
        ) = evaluate_pod_failures(&current_pods, &events_by_object, &dep_ns);

        let restart_in_window = current_pods.iter().any(|p| {
            let pod_created_in_window = p
                .metadata
                .creation_timestamp
                .as_ref()
                .and_then(|t| cutoff_ts.as_ref().map(|c| t.0 >= *c))
                .unwrap_or(false);
            if pod_created_in_window
                && p.status
                    .as_ref()
                    .and_then(|s| s.container_statuses.as_ref())
                    .map_or(false, |cs| cs.iter().any(|c| c.restart_count > 0))
            {
                return true;
            }
            if let Some(ref st) = p.status {
                if let Some(ref cs_list) = st.container_statuses {
                    for cs in cs_list {
                        if cs.restart_count > 0 {
                            if let Some(ref term) =
                                cs.last_state.as_ref().and_then(|s| s.terminated.as_ref())
                            {
                                if let Some(ref finished) = term.finished_at {
                                    if cutoff_ts.as_ref().map_or(true, |c| finished.0 >= *c) {
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            false
        });

        let mut probe_failure_count = 0;
        for p in &current_pods {
            let p_name = p.metadata.name.clone().unwrap_or_default();
            if let Some(p_events) =
                events_by_object.get(&("Pod".to_string(), dep_ns.clone(), p_name))
            {
                for ev in p_events {
                    if ev.reason.as_deref() == Some("Unhealthy") {
                        if cutoff_ts
                            .as_ref()
                            .map_or(true, |c| event_last_timestamp(ev).is_some_and(|t| t >= *c))
                        {
                            probe_failure_count += ev.count.unwrap_or(1) as usize;
                        }
                    }
                }
            }
        }
        let is_flapping = (restart_in_window && restart_count >= 2) || probe_failure_count > 0;

        let is_actively_failing = crash_loop_count > 0
            || oom_killed_count > 0
            || config_error_count > 0
            || image_error_count > 0
            || pending_pod_count > 0
            || progress_deadline_exceeded
            || (desired_replicas > 0 && ready_replicas < desired_replicas)
            || is_flapping;

        let mut change_detail = None;
        let change = if rolled_out {
            Some((ChangeKind::Rollout, deployed_age.clone()))
        } else if let Some((t, _)) = latest_in_window(false) {
            Some((ChangeKind::Rollout, age_of(t)))
        } else if let (true, Some((t, ev))) = (opts.include_scaled, latest_in_window(true)) {
            change_detail = Some(parse_scale_event(ev.message.as_deref().unwrap_or("")));
            Some((ChangeKind::Scaled, age_of(t)))
        } else if opts.include_failing {
            // Actively failing or warned in the window
            let failing = is_actively_failing
                || cutoff_ts.as_ref().is_some_and(|cutoff| {
                    warning_in_window(
                        events_by_object.get(&(
                            "ReplicaSet".to_string(),
                            dep_ns.clone(),
                            current_rs_name.to_string(),
                        )),
                        cutoff,
                    ) || pods_warned_in_window(&current_pods, &events_by_object, &dep_ns, cutoff)
                });
            failing.then(|| (ChangeKind::FailingOnly, deployed_age.clone()))
        } else {
            None
        };
        let Some((change_kind, changed_age)) = change else {
            continue;
        };

        tracked_objects.insert(("Deployment".to_string(), dep_ns.clone(), dep_name.clone()));
        for rs in &owned_rs {
            let rs_name = rs.metadata.name.clone().unwrap_or_default();
            for p in pods_by_rs
                .get(&(dep_ns.clone(), rs_name.clone()))
                .into_iter()
                .flatten()
            {
                let pod_name = p.metadata.name.clone().unwrap_or_default();
                tracked_objects.insert(("Pod".to_string(), dep_ns.clone(), pod_name));
            }
            tracked_objects.insert(("ReplicaSet".to_string(), dep_ns.clone(), rs_name));
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

        let mut correlated_events: Vec<EventSummary> = Vec::new();

        for ev in &dep_events {
            if !is_noisy_normal_event(ev.reason.as_deref().unwrap_or(""), ev.type_.as_deref()) {
                correlated_events.push(crate::events::summarise((*ev).clone()));
            }
        }
        if !current_rs_name.is_empty() {
            if let Some(rs_events) = events_by_object.get(&(
                "ReplicaSet".to_string(),
                dep_ns.clone(),
                current_rs_name.to_string(),
            )) {
                for ev in rs_events {
                    if !is_noisy_normal_event(
                        ev.reason.as_deref().unwrap_or(""),
                        ev.type_.as_deref(),
                    ) {
                        correlated_events.push(crate::events::summarise((*ev).clone()));
                    }
                }
            }
        }
        for p in &current_pods {
            let p_name = p.metadata.name.clone().unwrap_or_default();
            if let Some(p_events) =
                events_by_object.get(&("Pod".to_string(), dep_ns.clone(), p_name))
            {
                for ev in p_events {
                    if !is_noisy_normal_event(
                        ev.reason.as_deref().unwrap_or(""),
                        ev.type_.as_deref(),
                    ) {
                        correlated_events.push(crate::events::summarise((*ev).clone()));
                    }
                }
            }
        }

        let gitops_match = gitops_release_for(
            argo_apps,
            &Workload {
                api_version: "apps/v1",
                kind: "Deployment",
                name: &dep_name,
                ns: &dep_ns,
                meta: &dep.metadata,
                rollout_created: current_rs
                    .and_then(|rs| rs.metadata.creation_timestamp.as_ref())
                    .map(|t| t.0),
            },
            now,
            cutoff_ts.as_ref(),
        );
        let local_cause = current_rs.and_then(|rs| restart_cause(rs, prev_rs));

        let rollout_status = if desired_replicas == 0 {
            RolloutStatus::ScaledDown
        } else if progress_deadline_exceeded {
            RolloutStatus::Stalled
        } else if ready_replicas == desired_replicas
            && updated_replicas == desired_replicas
            && crash_loop_count == 0
            && oom_killed_count == 0
            && config_error_count == 0
            && image_error_count == 0
            && pending_pod_count == 0
            && !is_flapping
        {
            RolloutStatus::Complete
        } else if !failing_pod_names.is_empty() {
            RolloutStatus::Failed
        } else {
            RolloutStatus::Progressing
        };

        let incident_status = if oom_killed_count > 0 {
            IncidentStatus::OomKilled
        } else if crash_loop_count > 0 {
            IncidentStatus::CrashLoop
        } else if config_error_count > 0 {
            IncidentStatus::ConfigError
        } else if image_error_count > 0 {
            IncidentStatus::ImageError
        } else if pending_pod_count > 0 {
            IncidentStatus::Pending
        } else if progress_deadline_exceeded {
            IncidentStatus::Stalled
        } else if is_flapping {
            IncidentStatus::Flapping
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
        } else if oom_killed_count > 0 || crash_loop_count > 0 || config_error_count > 0 {
            FailureCategory::App
        } else if image_error_count > 0 {
            FailureCategory::Image
        } else if pending_pod_count > 0 {
            FailureCategory::Compute
        } else if is_flapping {
            if probe_failure_count > 0 && !restart_in_window {
                FailureCategory::Network
            } else {
                FailureCategory::App
            }
        } else {
            FailureCategory::None
        };

        let failure_detail = if !detected_failure_detail.is_empty() {
            detected_failure_detail
        } else if oom_killed_count > 0 {
            format!("{oom_killed_count} pod(s) OOMKilled (Exit 137)")
        } else if crash_loop_count > 0 {
            format!("{crash_loop_count} pod(s) in CrashLoopBackOff")
        } else if config_error_count > 0 {
            format!("{config_error_count} pod(s) configuration error")
        } else if image_error_count > 0 {
            format!("{image_error_count} pod(s) image pull failure")
        } else if progress_deadline_exceeded {
            "Rollout stalled: ProgressDeadlineExceeded".to_string()
        } else if is_flapping {
            if probe_failure_count > 0 && restart_in_window && restart_count >= 2 {
                format!("{restart_count} restart(s), {probe_failure_count} probe failure(s)")
            } else if probe_failure_count > 0 {
                format!("{probe_failure_count} probe failure(s) in window")
            } else {
                format!("{restart_count} container restart(s) in window")
            }
        } else if ready_replicas < desired_replicas {
            format!("{ready_replicas}/{desired_replicas} Ready")
        } else {
            "Healthy".to_string()
        };
        // A scaled, healthy row: the scale is the news, not "Healthy".
        let failure_detail = match (&change_detail, failure_category) {
            (Some(scale), FailureCategory::None) => scale.clone(),
            _ => failure_detail,
        };

        if change_kind == ChangeKind::FailingOnly && incident_status == IncidentStatus::Healthy {
            continue;
        }

        let (error_log_pod, error_log_container) = log_target(&pod_symptoms, &current_pods);
        deployment_changes.push(AppDeploymentChange {
            app_name: dep_name,
            kind: "Deployment".to_string(),
            namespace: dep_ns,
            incident_status,
            failure_category,
            failure_detail,
            gitops: gitops_match.info,
            argo_rollout_in_window: gitops_match.rollout_in_window,
            gitops_unresolved: gitops_match.unresolved,
            local_cause,
            error_log_snippet: None,
            error_log_pod,
            error_log_container,
            change_kind,
            changed_age,
            change_detail,
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
            pod_symptoms,
            failing_pod_names,
            top_events: correlated_events,
        });
    }

    // 4b. Examine each StatefulSet
    for sts in statefulsets {
        let sts_ns = sts.metadata.namespace.clone().unwrap_or_default();
        if let Some(ref target_ns) = namespace {
            if !target_ns.is_empty() && &sts_ns != target_ns {
                continue;
            }
        }
        let sts_name = sts.metadata.name.clone().unwrap_or_default();

        let current_pods = pods_by_sts
            .get(&(sts_ns.clone(), sts_name.clone()))
            .cloned()
            .unwrap_or_default();

        let desired_replicas = sts.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
        let st = sts.status.as_ref();
        let updated_replicas = st.and_then(|s| s.updated_replicas).unwrap_or(0);
        let ready_replicas = st.and_then(|s| s.ready_replicas).unwrap_or(0);
        let available_replicas = st
            .and_then(|s| s.available_replicas)
            .unwrap_or(ready_replicas);

        let (
            failing_pod_names,
            crash_loop_count,
            oom_killed_count,
            config_error_count,
            image_error_count,
            pending_pod_count,
            restart_count,
            primary_symptoms,
            pod_symptoms,
            detected_failure_category,
            detected_failure_detail,
        ) = evaluate_pod_failures(&current_pods, &events_by_object, &sts_ns);

        let restart_in_window = current_pods.iter().any(|p| {
            let pod_created_in_window = p
                .metadata
                .creation_timestamp
                .as_ref()
                .and_then(|t| cutoff_ts.as_ref().map(|c| t.0 >= *c))
                .unwrap_or(false);
            if pod_created_in_window
                && p.status
                    .as_ref()
                    .and_then(|s| s.container_statuses.as_ref())
                    .map_or(false, |cs| cs.iter().any(|c| c.restart_count > 0))
            {
                return true;
            }
            if let Some(ref st) = p.status {
                if let Some(ref cs_list) = st.container_statuses {
                    for cs in cs_list {
                        if cs.restart_count > 0 {
                            if let Some(ref term) =
                                cs.last_state.as_ref().and_then(|s| s.terminated.as_ref())
                            {
                                if let Some(ref finished) = term.finished_at {
                                    if cutoff_ts.as_ref().map_or(true, |c| finished.0 >= *c) {
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            false
        });

        let mut probe_failure_count = 0;
        for p in &current_pods {
            let p_name = p.metadata.name.clone().unwrap_or_default();
            if let Some(p_events) =
                events_by_object.get(&("Pod".to_string(), sts_ns.clone(), p_name))
            {
                for ev in p_events {
                    if ev.reason.as_deref() == Some("Unhealthy") {
                        if cutoff_ts
                            .as_ref()
                            .map_or(true, |c| event_last_timestamp(ev).is_some_and(|t| t >= *c))
                        {
                            probe_failure_count += ev.count.unwrap_or(1) as usize;
                        }
                    }
                }
            }
        }
        let is_flapping = (restart_in_window && restart_count >= 2) || probe_failure_count > 0;

        let is_actively_failing = crash_loop_count > 0
            || oom_killed_count > 0
            || config_error_count > 0
            || image_error_count > 0
            || pending_pod_count > 0
            || (desired_replicas > 0 && ready_replicas < desired_replicas)
            || is_flapping;

        let sts_events = events_by_object
            .get(&("StatefulSet".to_string(), sts_ns.clone(), sts_name.clone()))
            .cloned()
            .unwrap_or_default();

        let latest_pod_created = current_pods
            .iter()
            .filter_map(|p| p.metadata.creation_timestamp.as_ref().map(|t| t.0))
            .max();
        let latest_event_ts = sts_events
            .iter()
            .filter_map(|ev| event_last_timestamp(ev))
            .max();

        let deployed_at_ts =
            latest_pod_created.or_else(|| sts.metadata.creation_timestamp.as_ref().map(|t| t.0));
        let deployed_at = deployed_at_ts.map(|t| t.to_string());
        let deployed_age = deployed_at_ts
            .map(|t| crate::format_age(now.duration_since(t).as_secs().max(0)))
            .unwrap_or_else(|| "-".to_string());

        let mut in_window = false;
        let mut change_kind = ChangeKind::Rollout;
        let mut changed_age = deployed_age.clone();

        if let Some(pod_ts) = latest_pod_created {
            if cutoff_ts.as_ref().is_some_and(|c| &pod_ts >= c) {
                in_window = true;
                change_kind = ChangeKind::Rollout;
                changed_age = crate::format_age(now.duration_since(pod_ts).as_secs().max(0));
            }
        }
        if !in_window {
            if let Some(ev_t) = latest_event_ts {
                if cutoff_ts.as_ref().is_some_and(|c| &ev_t >= c) {
                    in_window = true;
                    change_kind = ChangeKind::Scaled;
                    changed_age = crate::format_age(now.duration_since(ev_t).as_secs().max(0));
                }
            }
        }
        if !in_window && (opts.include_failing || is_actively_failing) {
            let failing = is_actively_failing
                || cutoff_ts.as_ref().is_some_and(|cutoff| {
                    pods_warned_in_window(&current_pods, &events_by_object, &sts_ns, cutoff)
                });
            if failing {
                in_window = true;
                change_kind = ChangeKind::FailingOnly;
                changed_age = deployed_age.clone();
            }
        }
        if !in_window && opts.include_scaled {
            if let Some(ref ct) = sts.metadata.creation_timestamp {
                if cutoff_ts.as_ref().is_some_and(|c| &ct.0 >= c) {
                    in_window = true;
                    change_kind = ChangeKind::Rollout;
                }
            }
        }

        if !in_window {
            continue;
        }

        tracked_objects.insert(("StatefulSet".to_string(), sts_ns.clone(), sts_name.clone()));
        for p in &current_pods {
            let pod_name = p.metadata.name.clone().unwrap_or_default();
            tracked_objects.insert(("Pod".to_string(), sts_ns.clone(), pod_name));
        }

        let current_images = extract_sts_container_images(sts);
        let image_diff = current_images.join(", ");
        let current_revision = sts
            .status
            .as_ref()
            .and_then(|s| s.current_revision.clone())
            .or_else(|| sts.status.as_ref().and_then(|s| s.update_revision.clone()))
            .unwrap_or_else(|| "1".to_string());

        let mut correlated_events: Vec<EventSummary> = Vec::new();

        for ev in &sts_events {
            if !is_noisy_normal_event(ev.reason.as_deref().unwrap_or(""), ev.type_.as_deref()) {
                correlated_events.push(crate::events::summarise((*ev).clone()));
            }
        }
        for p in &current_pods {
            let p_name = p.metadata.name.clone().unwrap_or_default();
            if let Some(p_events) =
                events_by_object.get(&("Pod".to_string(), sts_ns.clone(), p_name))
            {
                for ev in p_events {
                    if !is_noisy_normal_event(
                        ev.reason.as_deref().unwrap_or(""),
                        ev.type_.as_deref(),
                    ) {
                        correlated_events.push(crate::events::summarise((*ev).clone()));
                    }
                }
            }
        }

        let gitops_match = gitops_release_for(
            argo_apps,
            &Workload {
                api_version: "apps/v1",
                kind: "StatefulSet",
                name: &sts_name,
                ns: &sts_ns,
                meta: &sts.metadata,
                rollout_created: None,
            },
            now,
            cutoff_ts.as_ref(),
        );
        let local_cause = None;

        let rollout_status = if desired_replicas == 0 {
            RolloutStatus::ScaledDown
        } else if ready_replicas == desired_replicas
            && updated_replicas == desired_replicas
            && crash_loop_count == 0
            && oom_killed_count == 0
            && config_error_count == 0
            && image_error_count == 0
            && pending_pod_count == 0
            && !is_flapping
        {
            RolloutStatus::Complete
        } else if !failing_pod_names.is_empty() {
            RolloutStatus::Failed
        } else {
            RolloutStatus::Progressing
        };

        let incident_status = if oom_killed_count > 0 {
            IncidentStatus::OomKilled
        } else if crash_loop_count > 0 {
            IncidentStatus::CrashLoop
        } else if config_error_count > 0 {
            IncidentStatus::ConfigError
        } else if image_error_count > 0 {
            IncidentStatus::ImageError
        } else if pending_pod_count > 0 {
            IncidentStatus::Pending
        } else if is_flapping {
            IncidentStatus::Flapping
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
        } else if oom_killed_count > 0 || crash_loop_count > 0 || config_error_count > 0 {
            FailureCategory::App
        } else if image_error_count > 0 {
            FailureCategory::Image
        } else if pending_pod_count > 0 {
            FailureCategory::Compute
        } else if is_flapping {
            if probe_failure_count > 0 && !restart_in_window {
                FailureCategory::Network
            } else {
                FailureCategory::App
            }
        } else {
            FailureCategory::None
        };

        let failure_detail = if !detected_failure_detail.is_empty() {
            detected_failure_detail
        } else if oom_killed_count > 0 {
            format!("{oom_killed_count} pod(s) OOMKilled (Exit 137)")
        } else if crash_loop_count > 0 {
            format!("{crash_loop_count} pod(s) in CrashLoopBackOff")
        } else if config_error_count > 0 {
            format!("{config_error_count} pod(s) configuration error")
        } else if image_error_count > 0 {
            format!("{image_error_count} pod(s) image pull failure")
        } else if is_flapping {
            if probe_failure_count > 0 && restart_in_window && restart_count >= 2 {
                format!("{restart_count} restart(s), {probe_failure_count} probe failure(s)")
            } else if probe_failure_count > 0 {
                format!("{probe_failure_count} probe failure(s) in window")
            } else {
                format!("{restart_count} container restart(s) in window")
            }
        } else if ready_replicas < desired_replicas {
            format!("{ready_replicas}/{desired_replicas} Ready")
        } else {
            "Healthy".to_string()
        };

        if change_kind == ChangeKind::FailingOnly && incident_status == IncidentStatus::Healthy {
            continue;
        }

        let (error_log_pod, error_log_container) = log_target(&pod_symptoms, &current_pods);
        deployment_changes.push(AppDeploymentChange {
            app_name: sts_name,
            kind: "StatefulSet".to_string(),
            namespace: sts_ns,
            incident_status,
            failure_category,
            failure_detail,
            gitops: gitops_match.info,
            argo_rollout_in_window: gitops_match.rollout_in_window,
            gitops_unresolved: gitops_match.unresolved,
            local_cause,
            error_log_snippet: None,
            error_log_pod,
            error_log_container,
            change_kind,
            changed_age,
            change_detail: None,
            deployed_at,
            deployed_age,
            current_revision,
            previous_revision: None,
            current_images,
            previous_images: Vec::new(),
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
            pod_symptoms,
            failing_pod_names,
            top_events: correlated_events,
        });
    }

    // 4c. Examine each CronJob
    for cj in cronjobs {
        let cj_ns = cj.metadata.namespace.clone().unwrap_or_default();
        if let Some(ref target_ns) = namespace {
            if !target_ns.is_empty() && &cj_ns != target_ns {
                continue;
            }
        }
        let cj_name = cj.metadata.name.clone().unwrap_or_default();

        let mut current_pods: Vec<&Pod> = Vec::new();
        if let Some(owned_jobs) = jobs_by_cj.get(&(cj_ns.clone(), cj_name.clone())) {
            for job in owned_jobs {
                let job_name = job.metadata.name.clone().unwrap_or_default();
                if let Some(job_pods) = pods_by_job.get(&(cj_ns.clone(), job_name)) {
                    for p in job_pods {
                        current_pods.push(p);
                    }
                }
            }
        }

        let is_suspended = cj.spec.as_ref().and_then(|s| s.suspend).unwrap_or(false);
        let desired_replicas = if is_suspended { 0 } else { 1 };
        let ready_replicas = current_pods
            .iter()
            .filter(|p| {
                p.status.as_ref().and_then(|s| s.phase.as_deref()) == Some("Running")
                    || p.status.as_ref().and_then(|s| s.phase.as_deref()) == Some("Succeeded")
            })
            .count() as i32;
        let updated_replicas = ready_replicas;
        let available_replicas = ready_replicas;

        let (
            failing_pod_names,
            crash_loop_count,
            oom_killed_count,
            config_error_count,
            image_error_count,
            pending_pod_count,
            restart_count,
            primary_symptoms,
            pod_symptoms,
            detected_failure_category,
            detected_failure_detail,
        ) = evaluate_pod_failures(&current_pods, &events_by_object, &cj_ns);

        let restart_in_window = current_pods.iter().any(|p| {
            let pod_created_in_window = p
                .metadata
                .creation_timestamp
                .as_ref()
                .and_then(|t| cutoff_ts.as_ref().map(|c| t.0 >= *c))
                .unwrap_or(false);
            if pod_created_in_window
                && p.status
                    .as_ref()
                    .and_then(|s| s.container_statuses.as_ref())
                    .map_or(false, |cs| cs.iter().any(|c| c.restart_count > 0))
            {
                return true;
            }
            if let Some(ref st) = p.status {
                if let Some(ref cs_list) = st.container_statuses {
                    for cs in cs_list {
                        if cs.restart_count > 0 {
                            if let Some(ref term) =
                                cs.last_state.as_ref().and_then(|s| s.terminated.as_ref())
                            {
                                if let Some(ref finished) = term.finished_at {
                                    if cutoff_ts.as_ref().map_or(true, |c| finished.0 >= *c) {
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            false
        });

        let mut probe_failure_count = 0;
        for p in &current_pods {
            let p_name = p.metadata.name.clone().unwrap_or_default();
            if let Some(p_events) =
                events_by_object.get(&("Pod".to_string(), cj_ns.clone(), p_name))
            {
                for ev in p_events {
                    if ev.reason.as_deref() == Some("Unhealthy") {
                        if cutoff_ts
                            .as_ref()
                            .map_or(true, |c| event_last_timestamp(ev).is_some_and(|t| t >= *c))
                        {
                            probe_failure_count += ev.count.unwrap_or(1) as usize;
                        }
                    }
                }
            }
        }
        let is_flapping = (restart_in_window && restart_count >= 2) || probe_failure_count > 0;

        let is_actively_failing = crash_loop_count > 0
            || oom_killed_count > 0
            || config_error_count > 0
            || image_error_count > 0
            || pending_pod_count > 0
            || is_flapping;

        let cj_events = events_by_object
            .get(&("CronJob".to_string(), cj_ns.clone(), cj_name.clone()))
            .cloned()
            .unwrap_or_default();

        let latest_pod_created = current_pods
            .iter()
            .filter_map(|p| p.metadata.creation_timestamp.as_ref().map(|t| t.0))
            .max();
        let latest_sched = cj
            .status
            .as_ref()
            .and_then(|s| s.last_schedule_time.as_ref().map(|t| t.0));
        let latest_event_ts = cj_events
            .iter()
            .filter_map(|ev| event_last_timestamp(ev))
            .max();

        let cj_created = cj.metadata.creation_timestamp.as_ref().map(|t| t.0);
        let deployed_at_ts = cj_created.or(latest_sched).or(latest_pod_created);
        let deployed_at = deployed_at_ts.map(|t| t.to_string());
        let deployed_age = deployed_at_ts
            .map(|t| crate::format_age(now.duration_since(t).as_secs().max(0)))
            .unwrap_or_else(|| "-".to_string());

        let mut in_window = false;
        let mut change_kind = ChangeKind::Rollout;
        let mut changed_age = deployed_age.clone();

        if let Some(ts) = cj_created {
            if cutoff_ts.as_ref().is_some_and(|c| &ts >= c) {
                in_window = true;
                change_kind = ChangeKind::Rollout;
                changed_age = crate::format_age(now.duration_since(ts).as_secs().max(0));
            }
        }
        if !in_window {
            if let Some(ev_t) = latest_event_ts {
                if cutoff_ts.as_ref().is_some_and(|c| &ev_t >= c) {
                    in_window = true;
                    change_kind = ChangeKind::Scaled;
                    changed_age = crate::format_age(now.duration_since(ev_t).as_secs().max(0));
                }
            }
        }
        if !in_window && (opts.include_failing || is_actively_failing) {
            let failing = is_actively_failing
                || cutoff_ts.as_ref().is_some_and(|cutoff| {
                    pods_warned_in_window(&current_pods, &events_by_object, &cj_ns, cutoff)
                });
            if failing {
                in_window = true;
                change_kind = ChangeKind::FailingOnly;
                changed_age = deployed_age.clone();
            }
        }
        if !in_window && opts.include_scaled {
            if let Some(ref ct) = cj.metadata.creation_timestamp {
                if cutoff_ts.as_ref().is_some_and(|c| &ct.0 >= c) {
                    in_window = true;
                    change_kind = ChangeKind::Rollout;
                }
            }
        }

        if !in_window {
            continue;
        }

        tracked_objects.insert(("CronJob".to_string(), cj_ns.clone(), cj_name.clone()));
        for job in jobs_by_cj
            .get(&(cj_ns.clone(), cj_name.clone()))
            .into_iter()
            .flatten()
        {
            let job_name = job.metadata.name.clone().unwrap_or_default();
            tracked_objects.insert(("Job".to_string(), cj_ns.clone(), job_name));
        }
        for p in &current_pods {
            let pod_name = p.metadata.name.clone().unwrap_or_default();
            tracked_objects.insert(("Pod".to_string(), cj_ns.clone(), pod_name));
        }

        let current_images = extract_cronjob_container_images(cj);
        let image_diff = current_images.join(", ");
        let schedule_str = cj
            .spec
            .as_ref()
            .map(|s| s.schedule.clone())
            .unwrap_or_else(|| "-".to_string());

        let mut correlated_events: Vec<EventSummary> = Vec::new();

        for ev in &cj_events {
            if !is_noisy_normal_event(ev.reason.as_deref().unwrap_or(""), ev.type_.as_deref()) {
                correlated_events.push(crate::events::summarise((*ev).clone()));
            }
        }
        for p in &current_pods {
            let p_name = p.metadata.name.clone().unwrap_or_default();
            if let Some(p_events) =
                events_by_object.get(&("Pod".to_string(), cj_ns.clone(), p_name))
            {
                for ev in p_events {
                    if !is_noisy_normal_event(
                        ev.reason.as_deref().unwrap_or(""),
                        ev.type_.as_deref(),
                    ) {
                        correlated_events.push(crate::events::summarise((*ev).clone()));
                    }
                }
            }
        }

        let gitops_match = gitops_release_for(
            argo_apps,
            &Workload {
                api_version: "batch/v1",
                kind: "CronJob",
                name: &cj_name,
                ns: &cj_ns,
                meta: &cj.metadata,
                rollout_created: None,
            },
            now,
            cutoff_ts.as_ref(),
        );
        let local_cause = None;

        let rollout_status = if is_suspended {
            RolloutStatus::ScaledDown
        } else if !failing_pod_names.is_empty() {
            RolloutStatus::Failed
        } else {
            RolloutStatus::Complete
        };

        let incident_status = if oom_killed_count > 0 {
            IncidentStatus::OomKilled
        } else if crash_loop_count > 0 {
            IncidentStatus::CrashLoop
        } else if config_error_count > 0 {
            IncidentStatus::ConfigError
        } else if image_error_count > 0 {
            IncidentStatus::ImageError
        } else if pending_pod_count > 0 {
            IncidentStatus::Pending
        } else if is_flapping {
            IncidentStatus::Flapping
        } else if is_suspended {
            IncidentStatus::ScaledDown
        } else {
            IncidentStatus::Healthy
        };

        let failure_category = if detected_failure_category != FailureCategory::None {
            detected_failure_category
        } else if oom_killed_count > 0 || crash_loop_count > 0 || config_error_count > 0 {
            FailureCategory::App
        } else if image_error_count > 0 {
            FailureCategory::Image
        } else if pending_pod_count > 0 {
            FailureCategory::Compute
        } else if is_flapping {
            if probe_failure_count > 0 && !restart_in_window {
                FailureCategory::Network
            } else {
                FailureCategory::App
            }
        } else {
            FailureCategory::None
        };

        let failure_detail = if !detected_failure_detail.is_empty() {
            detected_failure_detail
        } else if oom_killed_count > 0 {
            format!("{oom_killed_count} pod(s) OOMKilled (Exit 137)")
        } else if crash_loop_count > 0 {
            format!("{crash_loop_count} pod(s) in CrashLoopBackOff")
        } else if config_error_count > 0 {
            format!("{config_error_count} pod(s) configuration error")
        } else if image_error_count > 0 {
            format!("{image_error_count} pod(s) image pull failure")
        } else if is_flapping {
            if probe_failure_count > 0 && restart_in_window && restart_count >= 2 {
                format!("{restart_count} restart(s), {probe_failure_count} probe failure(s)")
            } else if probe_failure_count > 0 {
                format!("{probe_failure_count} probe failure(s) in window")
            } else {
                format!("{restart_count} container restart(s) in window")
            }
        } else if is_suspended {
            format!("CronJob suspended ({schedule_str})")
        } else {
            format!("Scheduled ({schedule_str})")
        };

        if change_kind == ChangeKind::FailingOnly && incident_status == IncidentStatus::Healthy {
            continue;
        }

        let (error_log_pod, error_log_container) = log_target(&pod_symptoms, &current_pods);
        deployment_changes.push(AppDeploymentChange {
            app_name: cj_name,
            kind: "CronJob".to_string(),
            namespace: cj_ns,
            incident_status,
            failure_category,
            failure_detail,
            gitops: gitops_match.info,
            argo_rollout_in_window: gitops_match.rollout_in_window,
            gitops_unresolved: gitops_match.unresolved,
            local_cause,
            error_log_snippet: None,
            error_log_pod,
            error_log_container,
            change_kind,
            changed_age,
            change_detail: None,
            deployed_at,
            deployed_age,
            current_revision: schedule_str,
            previous_revision: None,
            current_images,
            previous_images: Vec::new(),
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
            pod_symptoms,
            failing_pod_names,
            top_events: correlated_events,
        });
    }

    // Sort deployments: OOM / CrashLoop / ConfigError / ImageError first, then Pending, Stalled, Rolling, Healthy, ScaledDown
    deployment_changes.sort_by(|a, b| {
        let rank = |s: IncidentStatus| match s {
            IncidentStatus::OomKilled => 0,
            IncidentStatus::CrashLoop => 1,
            IncidentStatus::ConfigError => 2,
            IncidentStatus::ImageError => 3,
            IncidentStatus::Pending => 4,
            IncidentStatus::Flapping => 5,
            IncidentStatus::Stalled => 6,
            IncidentStatus::Rolling => 7,
            IncidentStatus::Healthy => 8,
            IncidentStatus::ScaledDown => 9,
            IncidentStatus::Unknown => 10,
        };
        rank(a.incident_status)
            .cmp(&rank(b.incident_status))
            .then_with(|| {
                let a_deficit = a.ready_replicas < a.desired_replicas;
                let b_deficit = b.ready_replicas < b.desired_replicas;
                b_deficit.cmp(&a_deficit)
            })
            .then((a.change_kind as u8).cmp(&(b.change_kind as u8)))
    });

    // 5. Gather non-workload Infrastructure & Config changes
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

        // Skip events already shown on a tracked workload's card
        if tracked_objects.contains(&(kind.clone(), ev_ns.clone(), obj_name.clone())) {
            continue;
        }

        let reason = ev.reason.clone().unwrap_or_default();
        // Routine lifecycle noise (Pulling, Created, Started...) from pods
        // no card owns would bury the one ConfigMap or Node change.
        if is_noisy_normal_event(&reason, ev.type_.as_deref()) {
            continue;
        }
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
    infra_changes.sort_by(|a, b| {
        b.is_warning
            .cmp(&a.is_warning)
            .then_with(|| b.count.cmp(&a.count))
    });

    // 6. Compute Triage Summary
    let total_deployments = deployment_changes.len();
    let crashing_count = deployment_changes
        .iter()
        .filter(|d| d.incident_status == IncidentStatus::CrashLoop)
        .count();
    let oom_count = deployment_changes
        .iter()
        .filter(|d| d.incident_status == IncidentStatus::OomKilled)
        .count();
    let error_count = deployment_changes
        .iter()
        .filter(|d| {
            d.incident_status == IncidentStatus::ConfigError
                || d.incident_status == IncidentStatus::ImageError
        })
        .count();
    let pending_count = deployment_changes
        .iter()
        .filter(|d| d.incident_status == IncidentStatus::Pending)
        .count();
    let flapping_count = deployment_changes
        .iter()
        .filter(|d| d.incident_status == IncidentStatus::Flapping)
        .count();
    let rolling_count = deployment_changes
        .iter()
        .filter(|d| d.incident_status == IncidentStatus::Rolling)
        .count();
    let healthy_count = deployment_changes
        .iter()
        .filter(|d| d.incident_status == IncidentStatus::Healthy)
        .count();

    let headline_message = if oom_count > 0 {
        let first = deployment_changes
            .iter()
            .find(|d| d.incident_status == IncidentStatus::OomKilled)
            .map(|d| format!("{}: {}", d.app_name, d.failure_detail))
            .unwrap_or_default();
        format!("CRITICAL (OOM): {first}")
    } else if crashing_count > 0 {
        let first = deployment_changes
            .iter()
            .find(|d| d.incident_status == IncidentStatus::CrashLoop)
            .map(|d| format!("{}: {}", d.app_name, d.failure_detail))
            .unwrap_or_default();
        format!("CRITICAL (CRASH): {first}")
    } else if error_count > 0 {
        let first = deployment_changes
            .iter()
            .find(|d| {
                d.incident_status == IncidentStatus::ConfigError
                    || d.incident_status == IncidentStatus::ImageError
            })
            .map(|d| format!("{}: {}", d.app_name, d.failure_detail))
            .unwrap_or_default();
        format!("ERROR: {first}")
    } else if pending_count > 0 {
        let first = deployment_changes
            .iter()
            .find(|d| d.incident_status == IncidentStatus::Pending)
            .map(|d| format!("{}: {}", d.app_name, d.failure_detail))
            .unwrap_or_default();
        format!("BLOCKED (INFRA): {first}")
    } else if flapping_count > 0 {
        let first = deployment_changes
            .iter()
            .find(|d| d.incident_status == IncidentStatus::Flapping)
            .map(|d| format!("{}: {}", d.app_name, d.failure_detail))
            .unwrap_or_default();
        format!("DEGRADED (FLAPPING): {first}")
    } else if rolling_count > 0 {
        format!("{rolling_count} workload(s) currently rolling update")
    } else if healthy_count > 0 {
        format!("{healthy_count} workload(s) healthy")
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
            oom_count,
            error_count,
            pending_count,
            flapping_count,
            rolling_count,
            healthy_count,
            headline_message,
        },
        deployments: deployment_changes,
        infra_changes,
        includes_failing: opts.include_failing,
        includes_scaled: opts.include_scaled,
        argo: ArgoCoverage::default(),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListChangesIn {
    pub context: String,
    #[serde(default)]
    pub namespace: String,
    #[serde(default = "default_since")]
    pub since: String,
    /// Also report workloads failing in the window that did not change in it.
    #[serde(default, rename = "includeFailing")]
    pub include_failing: bool,
    /// Also report Deployments that only scaled in the window.
    #[serde(default, rename = "includeScaled")]
    pub include_scaled: bool,
}

fn default_since() -> String {
    "30m".to_string()
}

/// Where to look for the Argo Applications that manage a cluster: Argo on
/// the cluster itself, else the hub, matched to this cluster by its Argo
/// cluster name or API server URL. The default is the cluster itself only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArgoLookup {
    pub hub_context: Option<String>,
    pub cluster_name: Option<String>,
    pub server_url: Option<String>,
}

/// The Argo Applications a triage run matches workloads against, and how
/// much of Argo they are.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ArgoSnapshot {
    /// The Applications whose destination is this cluster.
    pub apps: Vec<ArgoApplication>,
    pub fetched_at: Option<Timestamp>,
    /// Every page arrived: not still streaming, not truncated.
    pub complete: bool,
    /// Why the latest read failed; `NO_ARGO` means there is no Argo at all.
    pub error: Option<String>,
    /// The hub the apps came from, when not this cluster.
    pub hub: Option<String>,
}

/// Where a triage run gets its Argo data.
#[derive(Debug, Clone)]
pub enum ArgoSource {
    /// Apps the caller already holds (the TUI's shared snapshot): no Argo
    /// call, so the report never waits on a hub listing thousands of apps.
    Snapshot(ArgoSnapshot),
    /// Look them up now, within `budget` overall (MCP `k8s.listChanges`).
    Lookup {
        lookup: ArgoLookup,
        budget: Duration,
    },
}

/// The Argo lookup behind [`ArgoSource::Lookup`], never longer than `budget`.
async fn lookup_snapshot(
    cache: &Arc<ClientCache>,
    context: &str,
    lookup: &ArgoLookup,
    budget: Duration,
) -> ArgoSnapshot {
    let fetch = crate::argo::fetch_argo_applications_cached(
        cache,
        context,
        lookup.cluster_name.as_deref(),
        lookup.server_url.as_deref(),
        lookup.hub_context.as_deref(),
        None,
        true,
        false,
    );
    match tokio::time::timeout(budget, fetch).await {
        Ok(Ok(r)) => ArgoSnapshot {
            complete: !r.truncated,
            hub: if r.is_remote_hub {
                lookup.hub_context.clone()
            } else {
                None
            },
            apps: r.filtered_apps,
            fetched_at: r
                .fetched_at
                .and_then(|ts| Timestamp::from_second(ts as i64).ok())
                .or_else(|| Some(Timestamp::now())),
            error: None,
        },
        Ok(Err(e)) => ArgoSnapshot {
            error: Some(e),
            ..Default::default()
        },
        Err(_) => ArgoSnapshot {
            error: Some(format!("Argo lookup timed out after {}s", budget.as_secs())),
            hub: lookup.hub_context.clone(),
            ..Default::default()
        },
    }
}

/// Fetches and evaluates the holistic triage report for recent changes across a cluster context.
pub async fn fetch_changed_triage(
    cache: &Arc<ClientCache>,
    context: &str,
    namespace: Option<&str>,
    window: Duration,
    opts: TriageOptions,
    argo: ArgoSource,
) -> Result<ChangedTriageReport, String> {
    let client = cache.get(context).await.map_err(|e| e.to_string())?;
    let ns_str = namespace.unwrap_or("");
    let now = Timestamp::now();

    let dep_api: Api<Deployment> = crate::scoped_api(client.clone(), ns_str);
    let sts_api: Api<StatefulSet> = crate::scoped_api(client.clone(), ns_str);
    let cj_api: Api<CronJob> = crate::scoped_api(client.clone(), ns_str);
    let job_api: Api<Job> = crate::scoped_api(client.clone(), ns_str);
    let rs_api: Api<ReplicaSet> = crate::scoped_api(client.clone(), ns_str);
    let pod_api: Api<Pod> = crate::scoped_api(client.clone(), ns_str);
    let ev_api: Api<Event> = crate::scoped_api(client.clone(), ns_str);

    let timeout = request_timeout();
    let lp = ListParams::default();
    let ns_opt = namespace.filter(|s| !s.is_empty()).map(|s| s.to_string());

    // A snapshot is ready now; a lookup runs beside the lists, bounded.
    let argo_fut = async {
        match argo {
            ArgoSource::Snapshot(snapshot) => snapshot,
            ArgoSource::Lookup { lookup, budget } => {
                lookup_snapshot(cache, context, &lookup, budget).await
            }
        }
    };

    // One round trip of wall time, not seven. Deployments, ReplicaSets, Pods
    // and Events are the core and fail the report; StatefulSets, CronJobs,
    // Jobs and Argo are optional and degrade to empty (RBAC may refuse them,
    // or Argo may not be installed).
    let (deps, sts, cjs, jobs, rs, pods, events, argo) = futures::join!(
        tokio::time::timeout(timeout, dep_api.list(&lp)),
        tokio::time::timeout(timeout, sts_api.list(&lp)),
        tokio::time::timeout(timeout, cj_api.list(&lp)),
        tokio::time::timeout(timeout, job_api.list(&lp)),
        tokio::time::timeout(timeout, rs_api.list(&lp)),
        tokio::time::timeout(timeout, pod_api.list(&lp)),
        tokio::time::timeout(timeout, ev_api.list(&lp)),
        argo_fut,
    );

    fn required<T>(
        what: &str,
        res: Result<Result<kube::core::ObjectList<T>, kube::Error>, tokio::time::error::Elapsed>,
    ) -> Result<Vec<T>, String>
    where
        T: Clone,
    {
        res.map_err(|_| format!("list {what} timed out"))?
            .map(|l| l.items)
            .map_err(|e| format!("list {what}: {e}"))
    }
    fn optional<T>(
        res: Result<Result<kube::core::ObjectList<T>, kube::Error>, tokio::time::error::Elapsed>,
    ) -> Vec<T>
    where
        T: Clone,
    {
        match res {
            Ok(Ok(l)) => l.items,
            _ => Vec::new(),
        }
    }

    let deps = required("deployments", deps)?;
    let rs = required("replicasets", rs)?;
    let pods = required("pods", pods)?;
    let events = required("events", events)?;
    let sts = optional(sts);
    let cjs = optional(cjs);
    let jobs = optional(jobs);
    let coverage = argo_coverage(&argo, now);
    let mut report = evaluate_changed_triage(
        &deps, &sts, &cjs, &jobs, &rs, &pods, &events, &argo.apps, window, now, ns_opt, opts,
    );
    apply_argo_coverage(&mut report, coverage);
    if !opts.log_snippets {
        return Ok(report);
    }

    // Tail five log lines per failing workload, from the pod and container
    // the evaluation chose.
    let targets: Vec<(usize, String, String, Option<String>)> = report
        .deployments
        .iter()
        .enumerate()
        .filter_map(|(i, d)| {
            let pod = d.error_log_pod.clone()?;
            Some((i, d.namespace.clone(), pod, d.error_log_container.clone()))
        })
        .collect();

    use futures::StreamExt;
    let tails: Vec<_> =
        futures::stream::iter(targets.into_iter().map(|(i, ns, pod, container)| {
            let api: Api<Pod> = Api::namespaced(client.clone(), &ns);
            async move {
                let lines = tail_error_log(&api, &pod, container.as_deref(), 5, timeout).await;
                (i, pod, container, lines)
            }
        }))
        .buffer_unordered(8)
        .collect()
        .await;

    for (i, _pod, _container, lines) in tails {
        if let (Ok(Some(lines)), Some(d)) = (lines, report.deployments.get_mut(i)) {
            d.error_log_snippet = Some(lines);
        }
    }

    Ok(report)
}

/// The last `tail_lines` log lines from `container` of `pod`: the previous
/// (crashed) run first, else the current one. `Ok(None)` when neither returns
/// any text. `Err` when API calls fail or time out.
async fn tail_error_log(
    api: &Api<Pod>,
    pod: &str,
    container: Option<&str>,
    tail_lines: i64,
    timeout: Duration,
) -> Result<Option<Vec<String>>, String> {
    let mut last_err = None;
    for previous in [true, false] {
        let lp = kube::api::LogParams {
            container: container.map(str::to_string),
            previous,
            tail_lines: Some(tail_lines),
            timestamps: false,
            ..Default::default()
        };
        match tokio::time::timeout(timeout, api.logs(pod, &lp)).await {
            Ok(Ok(text)) => {
                if let Some(lines) = log_snippet_lines(&text) {
                    return Ok(Some(lines));
                }
            }
            Ok(Err(kube::Error::Api(ae))) if ae.code == 400 || ae.reason == "BadRequest" => {
                // Expected when previous=true and container was not previously terminated
                continue;
            }
            Ok(Err(e)) => {
                last_err = Some(format!("fetch logs: {e}"));
            }
            Err(_) => {
                last_err = Some("fetch logs timed out".to_string());
            }
        }
    }
    if let Some(err) = last_err {
        Err(err)
    } else {
        Ok(None)
    }
}

/// A longer error-log tail for one pod, for the TUI's on-demand Quick AI
/// RCA: the card keeps five lines, the model gets `tail_lines`. `Ok(None)`
/// means the pod answered with no usable log; `Err` means the cluster could
/// not be reached at all, which the caller must not report as "no logs".
pub async fn fetch_error_log_tail(
    cache: &Arc<ClientCache>,
    context: &str,
    namespace: &str,
    pod: &str,
    container: Option<&str>,
    tail_lines: i64,
) -> Result<Option<Vec<String>>, String> {
    let client = cache.get(context).await.map_err(|e| e.to_string())?;
    let api: Api<Pod> = Api::namespaced(client, namespace);
    tail_error_log(&api, pod, container, tail_lines, request_timeout()).await
}

/// The non-empty lines of a log tail, or `None` when there are none or the
/// body is the kubelet saying it has no log. That message arrives as log
/// text (a rotated or garbage-collected previous container), and shown on
/// the card it reads as the application's own last words.
fn log_snippet_lines(text: &str) -> Option<Vec<String>> {
    let lines: Vec<String> = text
        .lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let kubelet_refusal = lines.len() == 1
        && (lines[0].starts_with("unable to retrieve container logs")
            || lines[0].starts_with("failed to try resolving symlinks"));
    (!lines.is_empty() && !kubelet_refusal).then_some(lines)
}

/// How long `k8s.listChanges` waits for Argo before reporting its coverage
/// as incomplete, rather than holding the whole report.
const MCP_ARGO_BUDGET: Duration = Duration::from_secs(20);

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
                let opts = TriageOptions {
                    include_failing: input.include_failing,
                    include_scaled: input.include_scaled,
                    log_snippets: true,
                };
                fetch_changed_triage(
                    &cache,
                    &input.context,
                    ns_opt,
                    window,
                    opts,
                    ArgoSource::Lookup {
                        lookup: ArgoLookup::default(),
                        budget: MCP_ARGO_BUDGET,
                    },
                )
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
        assert_eq!(
            parse_duration("30d").unwrap(),
            Duration::from_secs(30 * 86400)
        );
        assert!(parse_duration("31d").is_err());
        assert!(parse_duration("1000000d").is_err());
        assert_eq!(parse_duration("60s").unwrap(), Duration::from_secs(60));
        assert_eq!(parse_duration("45").unwrap(), Duration::from_secs(45 * 60));
        assert!(parse_duration("invalid").is_err());
        assert!(parse_duration("18446744073709551615m").is_err());
        assert!(parse_duration("18446744073709551615h").is_err());
        assert!(parse_duration("18446744073709551615d").is_err());
        assert!(parse_duration("18446744073709551615").is_err());
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
            &[],
            &[],
            &[],
            &[current_rs, prev_rs],
            &[pod],
            &[],
            &[],
            Duration::from_secs(1800), // 30m
            now,
            Some("default".to_string()),
            TriageOptions::default(),
        );

        assert_eq!(report.summary.total_deployments, 1);
        assert_eq!(report.summary.oom_count, 1);

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
            &[],
            &[],
            &[],
            &[current_rs],
            &[pod],
            &[],
            &[],
            Duration::from_secs(1800),
            now,
            Some("default".to_string()),
            TriageOptions::default(),
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
            &[],
            &[],
            &[],
            &[current_rs],
            &[pod],
            &[event],
            &[],
            Duration::from_secs(1800),
            now,
            Some("default".to_string()),
            TriageOptions::default(),
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
            &[],
            &[],
            &[],
            &[current_rs],
            &[pod],
            &[],
            &[],
            Duration::from_secs(1800),
            now,
            Some("default".to_string()),
            TriageOptions::default(),
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
            // Argo writes operationState.finishedAt: RFC 3339, 5m before `now`.
            last_sync_time: "2023-11-14T22:08:20Z".to_string(),
            created_at: "10d ago".to_string(),
            resources: vec![],
            sync_history: vec![],
        };

        let report = evaluate_changed_triage(
            &[dep],
            &[],
            &[],
            &[],
            &[current_rs],
            &[],
            &[],
            &[argo_app],
            Duration::from_secs(1800),
            now,
            Some("default".to_string()),
            TriageOptions::default(),
        );

        let change = &report.deployments[0];
        assert!(change.gitops.is_some());
        let gitops = change.gitops.as_ref().unwrap();
        assert_eq!(gitops.app_name, "payment-app");
        assert_eq!(gitops.sync_status, "Synced");
        assert_eq!(gitops.sync_revision, "4869710");
        assert_eq!(gitops.target_revision, "main");
        assert_eq!(gitops.repo_url, "https://github.com/org/repo.git");
        assert_eq!(gitops.sync_age, "5m");
        assert_eq!(
            change.argo_rollout_in_window.as_deref(),
            Some("rev 4869710 synced 5m ago")
        );
        assert!(change.argo_rollout_in_window.is_some());
    }

    #[test]
    fn triage_detects_statefulset_with_crash_loop() {
        use k8s_openapi::api::apps::v1::{StatefulSetSpec, StatefulSetStatus};
        let now = Timestamp::from_second(1_700_000_000).unwrap();
        let dep_time = Timestamp::from_second(1_700_000_000 - 300).unwrap();

        let sts = StatefulSet {
            metadata: ObjectMeta {
                name: Some("redis-cluster".to_string()),
                namespace: Some("default".to_string()),
                creation_timestamp: Some(Time(dep_time)),
                ..Default::default()
            },
            spec: Some(StatefulSetSpec {
                replicas: Some(3),
                template: PodTemplateSpec {
                    spec: Some(PodSpec {
                        containers: vec![Container {
                            name: "redis".to_string(),
                            image: Some("redis:7.2".to_string()),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }),
            status: Some(StatefulSetStatus {
                replicas: 3,
                ready_replicas: Some(2),
                current_revision: Some("rev-1".to_string()),
                ..Default::default()
            }),
        };

        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("redis-cluster-2".to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("StatefulSet", "redis-cluster")]),
                ..Default::default()
            },
            spec: Some(PodSpec::default()),
            status: Some(PodStatus {
                container_statuses: Some(vec![ContainerStatus {
                    name: "redis".to_string(),
                    ready: false,
                    restart_count: 3,
                    state: Some(ContainerState {
                        waiting: Some(ContainerStateWaiting {
                            reason: Some("CrashLoopBackOff".to_string()),
                            message: Some("back-off restarting".to_string()),
                        }),
                        ..Default::default()
                    }),
                    last_state: Some(ContainerState {
                        terminated: Some(ContainerStateTerminated {
                            exit_code: 1,
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
            &[],
            &[sts],
            &[],
            &[],
            &[],
            &[pod],
            &[],
            &[],
            Duration::from_secs(1800),
            now,
            Some("default".to_string()),
            TriageOptions::default(),
        );

        assert_eq!(report.summary.total_deployments, 1);
        assert_eq!(report.summary.crashing_count, 1);
        let item = &report.deployments[0];
        assert_eq!(item.app_name, "redis-cluster");
        assert_eq!(item.kind, "StatefulSet");
        assert_eq!(item.incident_status, IncidentStatus::CrashLoop);
        assert_eq!(item.pod_symptoms.len(), 1);
        assert_eq!(item.pod_symptoms[0].pod_name, "redis-cluster-2");
        assert_eq!(item.pod_symptoms[0].status, "CrashLoopBackOff");
        assert_eq!(item.pod_symptoms[0].detail_message, "exited with code 1");
    }

    #[test]
    fn triage_detects_cronjob_with_failed_pod() {
        use k8s_openapi::api::batch::v1::{CronJobSpec, CronJobStatus, JobSpec, JobTemplateSpec};
        let now = Timestamp::from_second(1_700_000_000).unwrap();
        let dep_time = Timestamp::from_second(1_700_000_000 - 300).unwrap();

        let cj = CronJob {
            metadata: ObjectMeta {
                name: Some("nightly-backup".to_string()),
                namespace: Some("default".to_string()),
                creation_timestamp: Some(Time(dep_time)),
                ..Default::default()
            },
            spec: Some(CronJobSpec {
                schedule: "0 2 * * *".to_string(),
                job_template: JobTemplateSpec {
                    spec: Some(JobSpec {
                        template: PodTemplateSpec {
                            spec: Some(PodSpec {
                                containers: vec![Container {
                                    name: "backup".to_string(),
                                    image: Some("backup:v1".to_string()),
                                    ..Default::default()
                                }],
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }),
            status: Some(CronJobStatus {
                last_schedule_time: Some(Time(dep_time)),
                ..Default::default()
            }),
        };

        let job = Job {
            metadata: ObjectMeta {
                name: Some("nightly-backup-2810".to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("CronJob", "nightly-backup")]),
                ..Default::default()
            },
            spec: Some(JobSpec::default()),
            status: None,
        };

        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("nightly-backup-2810-abcd".to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("Job", "nightly-backup-2810")]),
                ..Default::default()
            },
            spec: Some(PodSpec::default()),
            status: Some(PodStatus {
                container_statuses: Some(vec![ContainerStatus {
                    name: "backup".to_string(),
                    ready: false,
                    restart_count: 0,
                    state: Some(ContainerState {
                        waiting: Some(ContainerStateWaiting {
                            reason: Some("CreateContainerConfigError".to_string()),
                            message: Some("secret 'backup-creds' not found".to_string()),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
        };

        let report = evaluate_changed_triage(
            &[],
            &[],
            &[cj],
            &[job],
            &[],
            &[pod],
            &[],
            &[],
            Duration::from_secs(1800),
            now,
            Some("default".to_string()),
            TriageOptions::default(),
        );

        assert_eq!(report.summary.total_deployments, 1);
        assert_eq!(report.summary.error_count, 1);
        let item = &report.deployments[0];
        assert_eq!(item.app_name, "nightly-backup");
        assert_eq!(item.kind, "CronJob");
        assert_eq!(item.incident_status, IncidentStatus::ConfigError);
        assert_eq!(item.pod_symptoms.len(), 1);
        assert_eq!(item.pod_symptoms[0].status, "CreateContainerConfigError");
        assert!(item.pod_symptoms[0].detail_message.contains("backup-creds"));
    }

    #[test]
    fn triage_filters_noisy_events_and_keeps_warnings() {
        assert!(is_noisy_normal_event("ScalingReplicaSet", Some("Normal")));
        assert!(is_noisy_normal_event("SuccessfulCreate", Some("Normal")));
        assert!(is_noisy_normal_event("Pulling", Some("Normal")));
        assert!(is_noisy_normal_event("Pulled", Some("Normal")));
        assert!(is_noisy_normal_event("Created", Some("Normal")));
        assert!(is_noisy_normal_event("Started", Some("Normal")));

        // Warnings are NEVER noisy
        assert!(!is_noisy_normal_event("BackOff", Some("Warning")));
        assert!(!is_noisy_normal_event("FailedScheduling", Some("Warning")));
        assert!(!is_noisy_normal_event("FailedMount", Some("Warning")));
        assert!(!is_noisy_normal_event("Unhealthy", Some("Warning")));
    }

    // ---- fixtures for the window, dedup and log-target tests ----

    const NOW: i64 = 1_700_000_000;

    fn ts(secs_ago: i64) -> Time {
        Time(Timestamp::from_second(NOW - secs_ago).unwrap())
    }

    fn deployment(name: &str, replicas: i32, ready: i32) -> Deployment {
        Deployment {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: Some(replicas),
                ..Default::default()
            }),
            status: Some(DeploymentStatus {
                replicas: Some(replicas),
                updated_replicas: Some(replicas),
                ready_replicas: Some(ready),
                available_replicas: Some(ready),
                ..Default::default()
            }),
        }
    }

    fn replicaset(name: &str, owner: &str, created_secs_ago: i64) -> ReplicaSet {
        ReplicaSet {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("Deployment", owner)]),
                creation_timestamp: Some(ts(created_secs_ago)),
                ..Default::default()
            },
            spec: Some(ReplicaSetSpec::default()),
            status: Some(ReplicaSetStatus::default()),
        }
    }

    fn pod(name: &str, rs: &str, statuses: Vec<ContainerStatus>) -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                owner_references: Some(vec![make_owner_ref("ReplicaSet", rs)]),
                creation_timestamp: Some(ts(3 * 86_400)),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: statuses
                    .iter()
                    .map(|cs| Container {
                        name: cs.name.clone(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }),
            status: Some(PodStatus {
                container_statuses: Some(statuses),
                ..Default::default()
            }),
        }
    }

    fn running(name: &str) -> ContainerStatus {
        ContainerStatus {
            name: name.to_string(),
            ready: true,
            ..Default::default()
        }
    }

    fn crash_looping(name: &str) -> ContainerStatus {
        ContainerStatus {
            name: name.to_string(),
            restart_count: 12,
            state: Some(ContainerState {
                waiting: Some(ContainerStateWaiting {
                    reason: Some("CrashLoopBackOff".to_string()),
                    message: Some("back-off 5m0s restarting failed container".to_string()),
                }),
                ..Default::default()
            }),
            last_state: Some(ContainerState {
                terminated: Some(ContainerStateTerminated {
                    exit_code: 1,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn event(kind: &str, name: &str, type_: &str, reason: &str, secs_ago: i64) -> Event {
        Event {
            metadata: ObjectMeta {
                name: Some(format!("{name}.{reason}")),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            involved_object: k8s_openapi::api::core::v1::ObjectReference {
                kind: Some(kind.to_string()),
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            type_: Some(type_.to_string()),
            reason: Some(reason.to_string()),
            message: Some(format!("{reason} on {name}")),
            last_timestamp: Some(ts(secs_ago)),
            count: Some(1),
            ..Default::default()
        }
    }

    fn triage(
        deps: &[Deployment],
        rs: &[ReplicaSet],
        pods: &[Pod],
        events: &[Event],
        argo: &[ArgoApplication],
    ) -> ChangedTriageReport {
        triage_with(deps, rs, pods, events, argo, TriageOptions::default())
    }

    fn triage_with(
        deps: &[Deployment],
        rs: &[ReplicaSet],
        pods: &[Pod],
        events: &[Event],
        argo: &[ArgoApplication],
        opts: TriageOptions,
    ) -> ChangedTriageReport {
        evaluate_changed_triage(
            deps,
            &[],
            &[],
            &[],
            rs,
            pods,
            events,
            argo,
            Duration::from_secs(1800),
            Timestamp::from_second(NOW).unwrap(),
            Some("default".to_string()),
            opts,
        )
    }

    #[test]
    fn an_old_rollout_failing_is_reported_by_default_and_suppressed_when_failing_disabled() {
        // Rolled out three days ago; crash-looping now, BackOff 2m ago.
        let crashing = [deployment("api", 1, 0), deployment("quiet", 1, 1)];
        let rs = [
            replicaset("api-6f7", "api", 3 * 86_400),
            replicaset("quiet-1a2", "quiet", 3 * 86_400),
        ];
        let pods = [
            pod("api-6f7-x1", "api-6f7", vec![crash_looping("api")]),
            pod("quiet-1a2-y1", "quiet-1a2", vec![running("quiet")]),
        ];
        let events = [event("Pod", "api-6f7-x1", "Warning", "BackOff", 120)];

        // By default active failures are ALWAYS reported: api is in, quiet stays out.
        let default_rep = triage(&crashing, &rs, &pods, &events, &[]);
        let names: Vec<&str> = default_rep
            .deployments
            .iter()
            .map(|d| d.app_name.as_str())
            .collect();
        assert_eq!(
            names,
            ["api"],
            "active failure is reported; the quiet, old, healthy deployment stays out"
        );
        assert!(default_rep.includes_failing);
        let api = &default_rep.deployments[0];
        assert_eq!(api.incident_status, IncidentStatus::CrashLoop);
        assert!(
            api.change_kind == ChangeKind::FailingOnly,
            "marked as included for failing, not for changing"
        );

        // When include_failing is explicitly disabled, unchanged failing workloads stay out.
        let strict = triage_with(
            &crashing,
            &rs,
            &pods,
            &events,
            &[],
            TriageOptions {
                include_failing: false,
                include_scaled: false,
                log_snippets: false,
            },
        );
        assert!(strict.deployments.is_empty(), "{:?}", strict.deployments);
        assert!(!strict.includes_failing);
    }

    #[test]
    fn a_replicaset_that_cannot_create_pods_is_reported_by_default() {
        let deps = [deployment("batch", 2, 0)];
        let rs = [replicaset("batch-9c", "batch", 3 * 86_400)];
        let events = [event(
            "ReplicaSet",
            "batch-9c",
            "Warning",
            "FailedCreate",
            60,
        )];

        let default_rep = triage(&deps, &rs, &[], &events, &[]);
        assert_eq!(default_rep.deployments.len(), 1);
        assert_eq!(default_rep.deployments[0].app_name, "batch");
        assert_eq!(
            default_rep.deployments[0].change_kind,
            ChangeKind::FailingOnly
        );

        let strict = triage_with(
            &deps,
            &rs,
            &[],
            &events,
            &[],
            TriageOptions {
                include_failing: false,
                include_scaled: false,
                log_snippets: false,
            },
        );
        assert!(strict.deployments.is_empty());
    }

    #[test]
    fn a_changed_workload_is_not_marked_unchanged_and_sorts_first() {
        // Both crash-loop; only "fresh" rolled out in the window.
        let deps = [deployment("stale", 1, 0), deployment("fresh", 1, 0)];
        let rs = [
            replicaset("stale-1", "stale", 3 * 86_400),
            replicaset("fresh-1", "fresh", 300),
        ];
        let pods = [
            pod("stale-1-a", "stale-1", vec![crash_looping("app")]),
            pod("fresh-1-a", "fresh-1", vec![crash_looping("app")]),
        ];
        let events = [event("Pod", "stale-1-a", "Warning", "BackOff", 60)];
        let report = triage_with(
            &deps,
            &rs,
            &pods,
            &events,
            &[],
            TriageOptions {
                include_failing: true,
                ..TriageOptions::default()
            },
        );
        let rows: Vec<(&str, ChangeKind)> = report
            .deployments
            .iter()
            .map(|d| (d.app_name.as_str(), d.change_kind))
            .collect();
        assert_eq!(
            rows,
            [
                ("fresh", ChangeKind::Rollout),
                ("stale", ChangeKind::FailingOnly)
            ]
        );
    }

    #[test]
    fn a_container_running_again_after_a_non_zero_exit_is_crash_looping() {
        // Between back-offs: Running, last exit 1. Used to fall to Unknown.
        let restarted = ContainerStatus {
            name: "payment".to_string(),
            ready: false,
            restart_count: 40,
            state: Some(ContainerState {
                running: Some(Default::default()),
                ..Default::default()
            }),
            last_state: Some(ContainerState {
                terminated: Some(ContainerStateTerminated {
                    exit_code: 1,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let deps = [deployment("payment-api", 2, 0)];
        let rs = [replicaset("payment-api-6d", "payment-api", 300)];
        let pods = [pod("payment-api-6d-a", "payment-api-6d", vec![restarted])];

        let report = triage(&deps, &rs, &pods, &[], &[]);

        let d = &report.deployments[0];
        assert_eq!(d.incident_status, IncidentStatus::CrashLoop);
        assert_eq!(d.failure_category, FailureCategory::App);
        assert_eq!(report.summary.crashing_count, 1);
    }

    #[test]
    fn the_log_target_is_chosen_during_evaluation() {
        let deps = [deployment("api", 2, 0)];
        let rs = [replicaset("api-new", "api", 300)];
        let mut pending = pod("api-new-p", "api-new", vec![]);
        pending.status = Some(PodStatus {
            phase: Some("Pending".to_string()),
            ..Default::default()
        });
        let pods = [
            pending,
            pod(
                "api-new-c",
                "api-new",
                vec![running("app"), crash_looping("sidecar")],
            ),
        ];

        let report = triage(&deps, &rs, &pods, &[], &[]);

        let d = &report.deployments[0];
        assert_eq!(
            d.error_log_pod.as_deref(),
            Some("api-new-c"),
            "skips the Pending pod"
        );
        assert_eq!(d.error_log_container.as_deref(), Some("sidecar"));
        assert_eq!(d.error_log_snippet, None, "no log call was made");
    }

    fn scale_event(name: &str, message: &str, secs_ago: i64) -> Event {
        let mut ev = event("Deployment", name, "Normal", "ScalingReplicaSet", secs_ago);
        ev.message = Some(message.to_string());
        ev
    }

    #[test]
    fn a_scale_is_not_a_rollout_and_is_reported_only_when_asked() {
        // Rolled out 66 days ago; an HPA scaled it 5 minutes ago.
        let deps = [deployment("price-index", 4, 4)];
        let rs = [replicaset("price-index-9f", "price-index", 66 * 86_400)];
        let pods = [pod(
            "price-index-9f-a",
            "price-index-9f",
            vec![running("app")],
        )];
        let events = [scale_event(
            "price-index",
            "Scaled up replica set price-index-9f from 3 to 4",
            300,
        )];

        let default_rep = triage(&deps, &rs, &pods, &events, &[]);
        assert_eq!(default_rep.deployments.len(), 1);
        assert!(default_rep.includes_scaled);
        let d = &default_rep.deployments[0];
        assert_eq!(d.change_kind, ChangeKind::Scaled);
        assert_eq!(
            d.changed_age, "5m",
            "when it scaled, not when it rolled out"
        );
        assert_eq!(d.deployed_age, "66d");
        assert_eq!(d.change_detail.as_deref(), Some("Scaled 3→4"));
        assert_eq!(
            d.failure_detail, "Scaled 3→4",
            "the scale is the news on a healthy row"
        );

        let strict = triage_with(
            &deps,
            &rs,
            &pods,
            &events,
            &[],
            TriageOptions {
                include_failing: false,
                include_scaled: false,
                log_snippets: false,
            },
        );
        assert!(
            strict.deployments.is_empty(),
            "a scale alone is not a rollout when scaled is excluded"
        );
        assert!(!strict.includes_scaled);
    }

    #[test]
    fn a_new_replicaset_in_the_window_is_a_rollout_even_with_scale_events() {
        let deps = [deployment("web", 2, 2)];
        let rs = [
            replicaset("web-new", "web", 600),
            replicaset("web-old", "web", 30 * 86_400),
        ];
        let events = [scale_event(
            "web",
            "Scaled up replica set web-new from 0 to 2",
            590,
        )];

        let report = triage(&deps, &rs, &[], &events, &[]);

        let d = &report.deployments[0];
        assert_eq!(d.change_kind, ChangeKind::Rollout);
        assert_eq!(d.changed_age, "10m");
        assert_eq!(d.change_detail, None);
    }

    #[test]
    fn a_non_scaling_deployment_event_still_counts_as_a_change() {
        let deps = [deployment("api", 1, 1)];
        let rs = [replicaset("api-1", "api", 30 * 86_400)];
        let events = [event(
            "Deployment",
            "api",
            "Warning",
            "ReplicaSetCreateError",
            120,
        )];

        let report = triage(&deps, &rs, &[], &events, &[]);

        assert_eq!(report.deployments.len(), 1);
        assert_eq!(report.deployments[0].change_kind, ChangeKind::Rollout);
        assert_eq!(report.deployments[0].changed_age, "2m");
    }

    #[test]
    fn a_stale_scheduling_event_on_a_running_pod_is_not_a_root_cause() {
        // Rolled out 20m ago; unschedulable at first, Running now. Its
        // FailedScheduling event (15m old) lingers for about an hour.
        let deps = [deployment("mirrormaker", 3, 3)];
        let rs = [replicaset("mirrormaker-1", "mirrormaker", 1_200)];
        let pods = [pod("mirrormaker-1-a", "mirrormaker-1", vec![running("mm")])];
        let mut stale = event("Pod", "mirrormaker-1-a", "Warning", "FailedScheduling", 900);
        stale.message = Some("0/69 nodes are available: 13 Insufficient cpu.".to_string());

        let report = triage(&deps, &rs, &pods, &[stale], &[]);

        let d = &report.deployments[0];
        assert_eq!(d.incident_status, IncidentStatus::Healthy);
        assert_eq!(d.failure_category, FailureCategory::None);
        assert_eq!(d.failure_detail, "Healthy");
    }

    #[test]
    fn parse_scale_event_reads_both_message_forms() {
        assert_eq!(
            parse_scale_event("Scaled up replica set web-7f from 3 to 4"),
            "Scaled 3→4"
        );
        assert_eq!(
            parse_scale_event("Scaled down replica set web-7f from 10 to 2"),
            "Scaled 10→2"
        );
        assert_eq!(
            parse_scale_event("Scaled up replica set web-7f to 4"),
            "Scaled to 4"
        );
        assert_eq!(
            parse_scale_event("Scaled up replica set web-7f to 4."),
            "Scaled to 4"
        );
        assert_eq!(parse_scale_event(""), "Scaled");
        assert_eq!(parse_scale_event("something else entirely"), "Scaled");
    }

    #[test]
    fn list_changes_input_reads_the_callers_camel_case() {
        let input: ListChangesIn = serde_json::from_value(serde_json::json!({
            "context": "kind-x", "since": "1h", "includeFailing": true, "includeScaled": true
        }))
        .unwrap();
        assert!(input.include_failing);
        assert!(input.include_scaled);

        // The struct's own spelling is not what callers send, and is ignored.
        let input: ListChangesIn = serde_json::from_value(serde_json::json!({
            "context": "kind-x", "include_failing": true
        }))
        .unwrap();
        assert!(!input.include_failing);
    }

    #[test]
    fn a_normal_pod_event_alone_does_not_pull_an_old_deployment_in() {
        let deps = [deployment("web", 1, 1)];
        let rs = [replicaset("web-3d", "web", 3 * 86_400)];
        let pods = [pod("web-3d-z", "web-3d", vec![running("web")])];
        let events = [event("Pod", "web-3d-z", "Normal", "Pulled", 60)];

        let report = triage(&deps, &rs, &pods, &events, &[]);

        assert!(report.deployments.is_empty());
    }

    #[test]
    fn infra_tab_leaves_out_events_already_on_a_workload_card() {
        let deps = [deployment("api", 1, 0)];
        let rs = [
            replicaset("api-new", "api", 300),
            replicaset("api-old", "api", 3 * 86_400),
        ];
        let pods = [
            pod("api-new-a", "api-new", vec![crash_looping("api")]),
            pod("api-old-b", "api-old", vec![running("api")]),
        ];
        let events = [
            event("Deployment", "api", "Normal", "ScalingReplicaSet", 300),
            event("ReplicaSet", "api-new", "Warning", "FailedCreate", 200),
            event("Pod", "api-new-a", "Warning", "BackOff", 100),
            event("Pod", "api-old-b", "Normal", "Killing", 100),
            event("ConfigMap", "api-config", "Normal", "Updated", 100),
        ];

        let report = triage(&deps, &rs, &pods, &events, &[]);

        let infra: Vec<(&str, &str)> = report
            .infra_changes
            .iter()
            .map(|i| (i.kind.as_str(), i.name.as_str()))
            .collect();
        assert_eq!(infra, [("ConfigMap", "api-config")]);
    }

    #[test]
    fn events_of_an_untracked_object_sharing_a_workload_name_stay_in_infra() {
        // A Service called "api" is not the Deployment called "api".
        let deps = [deployment("api", 1, 1)];
        let rs = [replicaset("api-new", "api", 300)];
        let events = [event(
            "Service",
            "api",
            "Warning",
            "SyncLoadBalancerFailed",
            60,
        )];

        let report = triage(&deps, &rs, &[], &events, &[]);

        assert_eq!(report.infra_changes.len(), 1);
        assert_eq!(report.infra_changes[0].kind, "Service");
    }

    #[test]
    fn an_argo_sync_before_the_window_is_not_reported_as_a_rollout_in_it() {
        let mut labels = BTreeMap::new();
        labels.insert(
            "app.kubernetes.io/instance".to_string(),
            "api-app".to_string(),
        );
        let mut dep = deployment("api", 1, 1);
        dep.metadata.labels = Some(labels);
        let rs = [replicaset("api-new", "api", 300)];
        let app = ArgoApplication {
            name: "api-app".to_string(),
            namespace: "argocd".to_string(),
            uid: "uid-argo-2".to_string(),
            resource_version: "1".to_string(),
            project: "default".to_string(),
            destination_server: "https://kubernetes.default.svc".to_string(),
            destination_name: String::new(),
            destination_namespace: "default".to_string(),
            repo_url: "https://github.com/org/repo.git".to_string(),
            target_revision: "main".to_string(),
            path: "manifests".to_string(),
            sync_status: "Synced".to_string(),
            health_status: "Healthy".to_string(),
            health_message: String::new(),
            sync_revision: "0123456789abcdef".to_string(),
            operation_phase: String::new(),
            operation_message: String::new(),
            auto_sync_enabled: true,
            self_heal_enabled: true,
            prune_enabled: true,
            // Two hours before `now`; the window is 30m.
            last_sync_time: "2023-11-14T20:13:20Z".to_string(),
            created_at: String::new(),
            resources: vec![],
            sync_history: vec![],
        };

        let report = triage(&[dep], &rs, &[], &[], &[app]);

        let change = &report.deployments[0];
        let gitops = change.gitops.as_ref().expect("still matched to its app");
        assert_eq!(gitops.sync_age, "2h");
        assert_eq!(change.argo_rollout_in_window, None);
    }

    #[test]
    fn failing_container_name_picks_the_container_that_is_failing() {
        let p = pod(
            "api-x",
            "api-rs",
            vec![running("app"), crash_looping("sidecar")],
        );
        assert_eq!(failing_container_name(&p).as_deref(), Some("sidecar"));

        let healthy = pod("api-y", "api-rs", vec![running("app"), running("sidecar")]);
        assert_eq!(failing_container_name(&healthy).as_deref(), Some("app"));

        let mut no_status = healthy.clone();
        no_status.status = None;
        assert_eq!(failing_container_name(&no_status).as_deref(), Some("app"));
    }

    #[test]
    fn infra_tab_leaves_out_routine_lifecycle_noise() {
        let events = [
            event("Pod", "other-abc", "Normal", "Pulling", 60),
            event("Pod", "other-abc", "Normal", "Started", 60),
            event("Pod", "other-abc", "Warning", "BackOff", 60),
            event("Node", "node-1", "Normal", "NodeNotReady", 60),
        ];

        let report = triage(&[], &[], &[], &events, &[]);

        let mut reasons: Vec<&str> = report
            .infra_changes
            .iter()
            .map(|i| i.reason.as_str())
            .collect();
        reasons.sort();
        assert_eq!(reasons, ["BackOff", "NodeNotReady"]);
    }

    #[test]
    fn a_kubelet_refusal_is_not_a_log_snippet() {
        let refusal = "unable to retrieve container logs for containerd://c7d5745da967\n";
        assert_eq!(log_snippet_lines(refusal), None);
        assert_eq!(log_snippet_lines("\n  \n"), None);
        assert_eq!(
            log_snippet_lines("booting\npanic: nil map\n"),
            Some(vec!["booting".to_string(), "panic: nil map".to_string()])
        );
    }

    #[test]
    fn only_pods_that_ran_a_process_are_log_tail_targets() {
        assert!(status_has_logs("CrashLoopBackOff"));
        assert!(status_has_logs("OOMKilled"));
        assert!(status_has_logs("Error"));
        assert!(!status_has_logs("Pending"));
        assert!(!status_has_logs("ImagePullBackOff"));
        assert!(!status_has_logs("CreateContainerConfigError"));
    }

    #[test]
    fn report_deserializes_without_the_log_source_fields() {
        // An older host's report has no errorLogPod / errorLogContainer.
        let report = triage(
            &[deployment("api", 1, 1)],
            &[replicaset("api-new", "api", 300)],
            &[],
            &[],
            &[],
        );
        let mut raw = serde_json::to_value(&report).unwrap();
        let dep = raw["deployments"][0].as_object_mut().unwrap();
        assert!(dep.remove("errorLogPod").is_some());
        assert!(dep.remove("errorLogContainer").is_some());
        let back: ChangedTriageReport = serde_json::from_value(raw).unwrap();
        assert_eq!(back.deployments[0].error_log_pod, None);
    }

    // NOW is 2023-11-14T22:13:20Z; `replicaset(.., 300)` was created at 22:08:20Z.
    const SHA_OLD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_PREV: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SHA_NOW: &str = "cccccccccccccccccccccccccccccccccccccccc";

    fn hist(id: i64, rev: &str, started: &str, deployed: &str) -> serde_json::Value {
        let mut h = serde_json::json!({
            "id": id,
            "revision": rev,
            "deployedAt": deployed,
            "initiatedBy": { "username": "alice" },
            "source": { "repoURL": "https://github.com/acme/deploy.git", "path": "apps/shop" }
        });
        if !started.is_empty() {
            h["deployStartedAt"] = serde_json::json!(started);
        }
        h
    }

    fn argo_app(name: &str, ns: &str, history: Vec<serde_json::Value>) -> ArgoApplication {
        ArgoApplication::from_json(&serde_json::json!({
            "metadata": { "name": name, "namespace": ns },
            "spec": { "source": { "repoURL": "https://github.com/acme/deploy.git", "path": "apps/shop" } },
            "status": { "sync": { "revision": SHA_NOW }, "history": history }
        }))
    }

    /// A sync three hours ago, then the one that ran while `api-new` was created.
    fn shop_history() -> Vec<serde_json::Value> {
        vec![
            hist(6, SHA_PREV, "2023-11-14T19:00:00Z", "2023-11-14T19:01:00Z"),
            hist(7, SHA_NOW, "2023-11-14T22:08:00Z", "2023-11-14T22:08:40Z"),
        ]
    }

    fn tracked_deployment(tracking_id: Option<&str>, instance: Option<&str>) -> Deployment {
        let mut dep = deployment("api", 1, 1);
        if let Some(id) = tracking_id {
            dep.metadata.annotations =
                Some(BTreeMap::from([(TRACKING_ID.to_string(), id.to_string())]));
        }
        if let Some(app) = instance {
            dep.metadata.labels = Some(BTreeMap::from([(
                "app.kubernetes.io/instance".to_string(),
                app.to_string(),
            )]));
        }
        dep
    }

    #[test]
    fn a_tracking_id_decides_the_app_and_names_the_sync_behind_the_rollout() {
        let dep = tracked_deployment(Some("shop:apps/Deployment:default/api"), Some("other"));
        let apps = [
            argo_app("other", "argocd", vec![]),
            argo_app("shop", "argocd", shop_history()),
        ];
        let report = triage(
            &[dep],
            &[replicaset("api-new", "api", 300)],
            &[],
            &[],
            &apps,
        );

        let d = &report.deployments[0];
        let g = d.gitops.as_ref().expect("matched");
        assert_eq!(
            g.app_name, "shop",
            "the tracking id beats the instance label"
        );
        assert_eq!(g.matched_by, "trackingId");
        let r = g.rollout.as_ref().expect("the sync running at 22:08:20");
        assert_eq!(r.history_id, 7);
        assert_eq!(r.revision, SHA_NOW);
        assert_eq!(r.previous_revision.as_deref(), Some(SHA_PREV));
        assert_eq!(r.initiated_by.as_deref(), Some("alice"));
        assert_eq!(r.path, "apps/shop");
        assert!(!r.approximate);
        assert_eq!(g.rollout_unmatched, None);
        assert!(d
            .argo_rollout_in_window
            .as_deref()
            .is_some_and(|s| s.starts_with("rev ccccccc synced")));
        assert_eq!(d.gitops_unresolved, None);
    }

    #[test]
    fn a_namespaced_tracking_id_picks_the_app_in_that_namespace() {
        let dep = tracked_deployment(Some("team_shop:apps/Deployment:default/api"), None);
        let apps = [
            argo_app("shop", "argocd", vec![]),
            argo_app("shop", "team", shop_history()),
        ];
        let report = triage(
            &[dep],
            &[replicaset("api-new", "api", 300)],
            &[],
            &[],
            &apps,
        );
        let g = report.deployments[0].gitops.as_ref().unwrap();
        assert!(
            g.rollout.is_some(),
            "the app in `team`, which has the history"
        );
    }

    #[test]
    fn a_tracking_id_copied_from_another_workload_is_ignored() {
        let dep = tracked_deployment(Some("shop:apps/Deployment:default/web"), Some("other"));
        let apps = [
            argo_app("shop", "argocd", shop_history()),
            argo_app("other", "argocd", vec![]),
        ];
        let report = triage(
            &[dep],
            &[replicaset("api-new", "api", 300)],
            &[],
            &[],
            &apps,
        );
        let g = report.deployments[0].gitops.as_ref().unwrap();
        assert_eq!(g.app_name, "other");
        assert_eq!(g.matched_by, "label");
    }

    #[test]
    fn the_app_resource_list_beats_an_instance_label_naming_another_app() {
        let dep = tracked_deployment(None, Some("other"));
        let mut owner = argo_app("shop", "argocd", vec![]);
        owner.resources.push(crate::argo::ArgoResourceItem {
            group: "apps".into(),
            version: "v1".into(),
            kind: "Deployment".into(),
            namespace: "default".into(),
            name: "api".into(),
            status: "Synced".into(),
            health: "Healthy".into(),
            message: String::new(),
            hook: None,
        });
        let apps = [argo_app("other", "argocd", vec![]), owner];
        let report = triage(
            &[dep],
            &[replicaset("api-new", "api", 300)],
            &[],
            &[],
            &apps,
        );
        let g = report.deployments[0].gitops.as_ref().unwrap();
        assert_eq!(
            (g.app_name.as_str(), g.matched_by.as_str()),
            ("shop", "resources")
        );
    }

    #[test]
    fn a_tracking_id_naming_an_unlisted_app_is_unresolved_not_unmanaged() {
        let dep = tracked_deployment(Some("shop:apps/Deployment:default/api"), Some("other"));
        let apps = [argo_app("other", "argocd", vec![])];
        let mut report = triage(
            &[dep],
            &[replicaset("api-new", "api", 300)],
            &[],
            &[],
            &apps,
        );

        let d = &report.deployments[0];
        assert_eq!(d.gitops, None, "no guess from the label");
        assert_eq!(
            d.gitops_unresolved.as_deref(),
            Some("tracking id names Argo app shop, which Argo did not list")
        );

        let unavailable = ArgoSnapshot {
            error: Some("list timed out".into()),
            ..Default::default()
        };
        apply_argo_coverage(&mut report, argo_coverage(&unavailable, now()));
        assert_eq!(report.argo.state, ArgoCoverageState::Unavailable);
        assert_eq!(report.argo.error.as_deref(), Some("list timed out"));
        assert_eq!(
            report.deployments[0].gitops_unresolved.as_deref(),
            Some("tracking id names Argo app shop; Argo unavailable: list timed out")
        );
    }

    #[test]
    fn a_rollout_no_sync_accounts_for_is_unmatched_with_a_reason() {
        let dep = tracked_deployment(Some("shop:apps/Deployment:default/api"), None);
        let apps = [argo_app(
            "shop",
            "argocd",
            vec![hist(
                6,
                SHA_PREV,
                "2023-11-14T19:00:00Z",
                "2023-11-14T19:01:00Z",
            )],
        )];
        let report = triage(
            &[dep],
            &[replicaset("api-new", "api", 300)],
            &[],
            &[],
            &apps,
        );
        let d = &report.deployments[0];
        let g = d.gitops.as_ref().unwrap();
        assert_eq!(g.rollout, None);
        assert!(g
            .rollout_unmatched
            .as_deref()
            .is_some_and(|s| s.starts_with("no Argo sync around this rollout")));
        assert_eq!(
            d.argo_rollout_in_window, None,
            "the old sync is not this rollout"
        );
    }

    #[test]
    fn causal_history_entry_prefers_the_running_sync_then_one_finishing_soon_after() {
        let created: Timestamp = "2023-11-14T22:08:20Z".parse().unwrap();
        let item = |id, started: &str, deployed: &str| ArgoSyncHistoryItem {
            id,
            deployed_at: deployed.into(),
            deploy_started_at: started.into(),
            ..Default::default()
        };

        let running = [
            item(1, "2023-11-14T19:00:00Z", "2023-11-14T19:01:00Z"),
            item(2, "2023-11-14T22:08:00Z", "2023-11-14T22:08:40Z"),
            item(3, "2023-11-14T22:10:00Z", "2023-11-14T22:10:30Z"),
        ];
        assert_eq!(causal_history_entry(&running, created), Some(1));

        // No start times recorded: the first sync to finish after, within 15m.
        let no_starts = [
            item(1, "", "2023-11-14T22:05:00Z"),
            item(2, "", "2023-11-14T22:09:00Z"),
            item(3, "", "2023-11-14T22:12:00Z"),
        ];
        assert_eq!(causal_history_entry(&no_starts, created), Some(1));

        let too_late = [item(1, "", "2023-11-14T22:30:00Z")];
        assert_eq!(causal_history_entry(&too_late, created), None);
        assert_eq!(causal_history_entry(&[], created), None);
    }

    #[test]
    fn the_first_sync_of_an_app_has_no_previous_revision() {
        let dep = tracked_deployment(Some("shop:apps/Deployment:default/api"), None);
        let apps = [argo_app(
            "shop",
            "argocd",
            vec![
                // A sync of another source does not bound this one's changes.
                serde_json::json!({
                    "id": 6, "revision": SHA_OLD, "deployedAt": "2023-11-14T19:01:00Z",
                    "source": { "repoURL": "https://github.com/acme/legacy.git" }
                }),
                hist(7, SHA_NOW, "2023-11-14T22:08:00Z", "2023-11-14T22:08:40Z"),
            ],
        )];
        let report = triage(
            &[dep],
            &[replicaset("api-new", "api", 300)],
            &[],
            &[],
            &apps,
        );
        let r = report.deployments[0]
            .gitops
            .as_ref()
            .unwrap()
            .rollout
            .as_ref()
            .unwrap();
        assert_eq!(r.revision, SHA_NOW);
        assert_eq!(r.previous_revision, None);
    }

    fn templated_rs(
        name: &str,
        created_secs_ago: i64,
        image: &str,
        restarted_at: Option<&str>,
    ) -> ReplicaSet {
        let mut rs = replicaset(name, "api", created_secs_ago);
        rs.spec = Some(ReplicaSetSpec {
            template: Some(PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    annotations: restarted_at
                        .map(|t| BTreeMap::from([(RESTARTED_AT.to_string(), t.to_string())])),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![Container {
                        name: "app".into(),
                        image: Some(image.into()),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            }),
            ..Default::default()
        });
        rs
    }

    #[test]
    fn a_rollout_restart_is_the_local_cause_only_when_nothing_else_changed() {
        let prev = templated_rs("api-old", 86_400, "shop:1.4.2", None);
        let restarted = templated_rs("api-new", 300, "shop:1.4.2", Some("2023-11-14T22:08:19Z"));
        assert_eq!(
            restart_cause(&restarted, Some(&prev)).as_deref(),
            Some("rollout restart at 2023-11-14T22:08:19Z")
        );

        let bumped = templated_rs("api-new", 300, "shop:1.4.3", Some("2023-11-14T22:08:19Z"));
        assert_eq!(
            restart_cause(&bumped, Some(&prev)),
            None,
            "the image change is the news"
        );

        let again = templated_rs(
            "api-old",
            86_400,
            "shop:1.4.2",
            Some("2023-11-14T22:08:19Z"),
        );
        assert_eq!(
            restart_cause(&restarted, Some(&again)),
            None,
            "same stamp: no restart"
        );
        assert_eq!(
            restart_cause(&restarted, None),
            None,
            "a first rollout is not a restart"
        );

        let report = triage(
            &[deployment("api", 1, 1)],
            &[prev, restarted],
            &[],
            &[],
            &[],
        );
        assert_eq!(
            report.deployments[0].local_cause.as_deref(),
            Some("rollout restart at 2023-11-14T22:08:19Z")
        );
    }

    #[test]
    fn the_report_names_the_why_fields_in_camel_case() {
        let dep = tracked_deployment(Some("shop:apps/Deployment:default/api"), None);
        let apps = [argo_app("shop", "argocd", shop_history())];
        let mut report = triage(
            &[dep],
            &[replicaset("api-new", "api", 300)],
            &[],
            &[],
            &apps,
        );
        report.argo = ArgoCoverage {
            state: ArgoCoverageState::Partial,
            fetched_at: Some("2023-11-14T22:10:00Z".into()),
            apps_loaded: 1,
            hub: Some("tools".into()),
            error: None,
        };
        let raw = serde_json::to_value(&report).unwrap();
        let d = &raw["deployments"][0];
        assert_eq!(d["gitops"]["matchedBy"], "trackingId");
        assert_eq!(d["gitops"]["rollout"]["previousRevision"], SHA_PREV);
        assert_eq!(d["gitops"]["rollout"]["historyId"], 7);
        assert!(d.get("gitopsUnresolved").is_some());
        assert!(d.get("localCause").is_some());
        assert_eq!(raw["argo"]["state"], "partial");
        assert_eq!(raw["argo"]["fetchedAt"], "2023-11-14T22:10:00Z");
        assert_eq!(raw["argo"]["appsLoaded"], 1);
        assert_eq!(raw["argo"]["hub"], "tools");

        // An older host's report, with none of the new keys, still reads.
        let mut old = raw.clone();
        old.as_object_mut().unwrap().remove("argo");
        let od = old["deployments"][0].as_object_mut().unwrap();
        od.remove("gitopsUnresolved");
        od.remove("localCause");
        let og = od["gitops"].as_object_mut().unwrap();
        og.remove("matchedBy");
        og.remove("rollout");
        og.remove("rolloutUnmatched");
        let back: ChangedTriageReport = serde_json::from_value(old).unwrap();
        assert_eq!(back.deployments[0].gitops.as_ref().unwrap().rollout, None);
    }

    fn now() -> Timestamp {
        Timestamp::from_second(NOW).unwrap()
    }

    fn ago(secs: i64) -> Timestamp {
        Timestamp::from_second(NOW - secs).unwrap()
    }

    #[test]
    fn argo_coverage_names_how_much_of_argo_a_report_saw() {
        let state = |s: ArgoSnapshot| argo_coverage(&s, now()).state;
        let app = || vec![argo_app("shop", "argocd", vec![])];
        assert_eq!(
            state(ArgoSnapshot {
                error: Some(crate::argo::NO_ARGO.into()),
                ..Default::default()
            }),
            ArgoCoverageState::NoArgo
        );
        assert_eq!(
            state(ArgoSnapshot {
                error: Some("boom".into()),
                ..Default::default()
            }),
            ArgoCoverageState::Unavailable
        );
        assert_eq!(
            state(ArgoSnapshot {
                apps: app(),
                complete: false,
                ..Default::default()
            }),
            ArgoCoverageState::Partial
        );
        assert_eq!(
            state(ArgoSnapshot {
                apps: app(),
                complete: true,
                fetched_at: Some(ago(3600)),
                ..Default::default()
            }),
            ArgoCoverageState::Stale
        );
        assert_eq!(
            state(ArgoSnapshot {
                apps: app(),
                complete: true,
                error: Some("refresh failed".into()),
                fetched_at: Some(ago(10)),
                ..Default::default()
            }),
            ArgoCoverageState::Stale,
            "kept after a failed refresh"
        );
        assert_eq!(
            state(ArgoSnapshot {
                apps: app(),
                complete: true,
                fetched_at: Some(ago(8 * 60)),
                ..Default::default()
            }),
            ArgoCoverageState::Complete,
            "8 minutes old is within 10m TTL, not stale"
        );
        assert_eq!(
            state(ArgoSnapshot {
                apps: app(),
                complete: true,
                fetched_at: Some(ago(10)),
                ..Default::default()
            }),
            ArgoCoverageState::Complete
        );
    }

    #[test]
    fn with_part_of_argo_a_row_without_an_app_has_an_unknown_owner() {
        let partial = ArgoSnapshot {
            apps: vec![argo_app("other", "argocd", vec![])],
            complete: false,
            ..Default::default()
        };
        let rs = [replicaset("api-new", "api", 300)];

        let mut plain = triage(&[deployment("api", 1, 1)], &rs, &[], &[], &partial.apps);
        apply_argo_coverage(&mut plain, argo_coverage(&partial, now()));
        let d = &plain.deployments[0];
        assert_eq!(d.gitops, None);
        assert_eq!(
            d.gitops_unresolved.as_deref(),
            Some("Argo list incomplete (1 apps loaded); ownership not known yet")
        );

        let tracked = tracked_deployment(Some("shop:apps/Deployment:default/api"), None);
        let mut report = triage(&[tracked], &rs, &[], &[], &partial.apps);
        apply_argo_coverage(&mut report, argo_coverage(&partial, now()));
        assert_eq!(
            report.deployments[0].gitops_unresolved.as_deref(),
            Some("tracking id names Argo app shop; the Argo list is still loading (1 apps so far)")
        );

        // With all of Argo, no app means no Argo owner.
        let complete = ArgoSnapshot {
            complete: true,
            fetched_at: Some(ago(10)),
            ..partial
        };
        let mut unmanaged = triage(&[deployment("api", 1, 1)], &rs, &[], &[], &complete.apps);
        apply_argo_coverage(&mut unmanaged, argo_coverage(&complete, now()));
        assert_eq!(unmanaged.deployments[0].gitops_unresolved, None);
        assert_eq!(unmanaged.argo.state, ArgoCoverageState::Complete);
    }

    #[test]
    fn a_rollout_newer_than_the_argo_data_says_the_data_is_older() {
        let dep = tracked_deployment(Some("shop:apps/Deployment:default/api"), None);
        let apps = vec![argo_app(
            "shop",
            "argocd",
            vec![hist(
                6,
                SHA_PREV,
                "2023-11-14T19:00:00Z",
                "2023-11-14T19:01:00Z",
            )],
        )];
        // The ReplicaSet was created 300s ago; the snapshot is an hour old.
        let old = ArgoSnapshot {
            apps: apps.clone(),
            complete: true,
            fetched_at: Some(ago(3600)),
            ..Default::default()
        };
        let mut report = triage(
            &[dep.clone()],
            &[replicaset("api-new", "api", 300)],
            &[],
            &[],
            &apps,
        );
        apply_argo_coverage(&mut report, argo_coverage(&old, now()));
        let g = report.deployments[0].gitops.as_ref().unwrap();
        assert_eq!(
            g.rollout_unmatched.as_deref(),
            Some(ARGO_OLDER_THAN_ROLLOUT)
        );

        // Fetched after the rollout: its sync is genuinely not in the history.
        let fresh = ArgoSnapshot {
            fetched_at: Some(ago(10)),
            ..old
        };
        let mut report = triage(
            &[dep],
            &[replicaset("api-new", "api", 300)],
            &[],
            &[],
            &apps,
        );
        apply_argo_coverage(&mut report, argo_coverage(&fresh, now()));
        let g = report.deployments[0].gitops.as_ref().unwrap();
        assert!(g
            .rollout_unmatched
            .as_deref()
            .is_some_and(|s| s.starts_with("no Argo sync around this rollout")));
    }

    /// A cluster answering every list with nothing and recording each path;
    /// with `argo_hangs`, Argo Application lists never answer.
    fn cluster_client(argo_hangs: bool) -> (kube::Client, Arc<std::sync::Mutex<Vec<String>>>) {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        let service = tower::service_fn(move |request: http::Request<kube::client::Body>| {
            let log = log.clone();
            async move {
                let path = request.uri().path().to_string();
                log.lock().unwrap().push(path.clone());
                if argo_hangs && path.contains("argoproj.io") {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                }
                let body = serde_json::json!({
                    "apiVersion": "v1", "kind": "List",
                    "metadata": {"resourceVersion": "1"}, "items": []
                });
                Ok::<_, std::convert::Infallible>(
                    http::Response::builder()
                        .status(200)
                        .header("content-type", "application/json")
                        .body(kube::client::Body::from(body.to_string().into_bytes()))
                        .unwrap(),
                )
            }
        });
        (kube::Client::new(service, "default"), seen)
    }

    #[tokio::test]
    async fn a_snapshot_triage_asks_argo_nothing() {
        let (client, seen) = cluster_client(false);
        let cache = ClientCache::new_many(vec![]);
        cache.preload("snapshot-ctx", client).await;
        let snapshot = ArgoSnapshot {
            apps: vec![argo_app("shop", "argocd", shop_history())],
            fetched_at: Some(Timestamp::now()),
            complete: true,
            error: None,
            hub: Some("tools".into()),
        };
        let report = fetch_changed_triage(
            &cache,
            "snapshot-ctx",
            Some("default"),
            Duration::from_secs(1800),
            TriageOptions::default(),
            ArgoSource::Snapshot(snapshot),
        )
        .await
        .unwrap();

        assert_eq!(report.argo.state, ArgoCoverageState::Complete);
        assert_eq!(report.argo.apps_loaded, 1);
        assert_eq!(report.argo.hub.as_deref(), Some("tools"));
        let seen = seen.lock().unwrap();
        assert!(seen.iter().any(|p| p.contains("deployments")), "{seen:?}");
        assert!(
            !seen.iter().any(|p| p.contains("argoproj.io")),
            "no Argo call with a snapshot: {seen:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_lookup_past_its_budget_still_returns_the_report() {
        let (client, _seen) = cluster_client(true);
        let cache = ClientCache::new_many(vec![]);
        cache.preload("budget-ctx", client).await;
        let report = fetch_changed_triage(
            &cache,
            "budget-ctx",
            Some("default"),
            Duration::from_secs(1800),
            TriageOptions::default(),
            ArgoSource::Lookup {
                lookup: ArgoLookup::default(),
                budget: Duration::from_secs(5),
            },
        )
        .await
        .expect("the report does not wait on Argo past the budget");

        assert_eq!(report.argo.state, ArgoCoverageState::Unavailable);
        assert_eq!(
            report.argo.error.as_deref(),
            Some("Argo lookup timed out after 5s")
        );
    }

    #[tokio::test]
    async fn lookup_snapshot_preserves_cached_fetched_at() {
        let _lock = crate::argo::CACHE_TEST_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::argo::invalidate_argo_applications_cache();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SRELENS_CACHE_DIR", dir.path());
        let ctx = "test-lookup-fetched-at";
        let cache = ClientCache::new_many(vec![]);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let target_ts = now - 500;
        let fresh_result = crate::argo::ArgoApplicationsFetchResult {
            all_apps: vec![],
            filtered_apps: vec![],
            is_remote_hub: false,
            truncated: false,
            fetched_at: Some(target_ts),
        };
        crate::argo::save_argo_apps_disk_cache(ctx, &fresh_result);

        let snap =
            lookup_snapshot(&cache, ctx, &ArgoLookup::default(), Duration::from_secs(5)).await;

        let expected_ts = Timestamp::from_second(target_ts as i64).unwrap();
        assert_eq!(
            snap.fetched_at,
            Some(expected_ts),
            "lookup_snapshot must preserve cached fetched_at instead of stamping now"
        );

        crate::argo::invalidate_argo_disk_cache(ctx);
        std::env::remove_var("SRELENS_CACHE_DIR");
    }

    #[test]
    fn successful_sync_message_is_not_treated_as_sync_error() {
        let app = ArgoApplication::from_json(&serde_json::json!({
            "metadata": { "name": "shop", "namespace": "argocd" },
            "spec": { "source": { "repoURL": "https://github.com/acme/deploy.git", "path": "apps/shop" } },
            "status": {
                "sync": { "status": "Synced", "revision": SHA_NOW },
                "health": { "status": "Healthy" },
                "operationState": {
                    "phase": "Succeeded",
                    "message": "successfully synced (all tasks run)"
                },
                "history": shop_history()
            }
        }));
        let dep = tracked_deployment(Some("shop:apps/Deployment:default/api"), None);
        let rs = [replicaset("api-new", "api", 300)];
        let report = triage(&[dep], &rs, &[], &[], &[app]);
        let gitops = report.deployments[0].gitops.as_ref().unwrap();
        assert_eq!(
            gitops.sync_message, None,
            "a successful sync message must not be recorded as sync_message"
        );
    }

    #[test]
    fn failed_sync_message_is_recorded_as_sync_error() {
        let app = ArgoApplication::from_json(&serde_json::json!({
            "metadata": { "name": "shop", "namespace": "argocd" },
            "spec": { "source": { "repoURL": "https://github.com/acme/deploy.git", "path": "apps/shop" } },
            "status": {
                "sync": { "status": "Failed", "revision": SHA_NOW },
                "health": { "status": "Degraded" },
                "operationState": {
                    "phase": "Failed",
                    "message": "one or more synchronization tasks are not valid"
                },
                "history": shop_history()
            }
        }));
        let dep = tracked_deployment(Some("shop:apps/Deployment:default/api"), None);
        let rs = [replicaset("api-new", "api", 300)];
        let report = triage(&[dep], &rs, &[], &[], &[app]);
        let gitops = report.deployments[0].gitops.as_ref().unwrap();
        assert_eq!(
            gitops.sync_message.as_deref(),
            Some("one or more synchronization tasks are not valid")
        );
        assert!(
            !gitops.is_health_message,
            "operation error must not be marked as health message"
        );
    }

    #[test]
    fn degraded_health_message_sets_is_health_message_flag() {
        let app = ArgoApplication::from_json(&serde_json::json!({
            "metadata": { "name": "shop", "namespace": "argocd" },
            "spec": { "source": { "repoURL": "https://github.com/acme/deploy.git", "path": "apps/shop" } },
            "status": {
                "sync": { "status": "OutOfSync", "revision": SHA_NOW },
                "health": { "status": "Degraded", "message": "Deployment has 0/2 ready pods" },
                "history": shop_history()
            }
        }));
        let dep = tracked_deployment(Some("shop:apps/Deployment:default/api"), None);
        let rs = [replicaset("api-new", "api", 300)];
        let report = triage(&[dep], &rs, &[], &[], &[app]);
        let gitops = report.deployments[0].gitops.as_ref().unwrap();
        assert_eq!(
            gitops.sync_message.as_deref(),
            Some("Deployment has 0/2 ready pods")
        );
        assert!(
            gitops.is_health_message,
            "health message must be marked as is_health_message"
        );
    }

    #[test]
    fn routine_cronjob_schedule_ticks_are_not_reported_as_rollouts() {
        use k8s_openapi::api::batch::v1::{CronJobSpec, CronJobStatus, JobSpec, JobTemplateSpec};
        let now = Timestamp::from_second(1_700_000_000).unwrap();
        let old_creation = Timestamp::from_second(1_700_000_000 - 86400 * 10).unwrap();
        let sched_time = Timestamp::from_second(1_700_000_000 - 300).unwrap();

        let cj = CronJob {
            metadata: ObjectMeta {
                name: Some("routine-sync".to_string()),
                namespace: Some("default".to_string()),
                creation_timestamp: Some(Time(old_creation)),
                ..Default::default()
            },
            spec: Some(CronJobSpec {
                schedule: "*/5 * * * *".to_string(),
                job_template: JobTemplateSpec {
                    spec: Some(JobSpec::default()),
                    ..Default::default()
                },
                ..Default::default()
            }),
            status: Some(CronJobStatus {
                last_schedule_time: Some(Time(sched_time)),
                ..Default::default()
            }),
        };

        let report = evaluate_changed_triage(
            &[],
            &[],
            &[cj],
            &[],
            &[],
            &[],
            &[],
            &[],
            Duration::from_secs(3600),
            now,
            None,
            TriageOptions::default(),
        );

        assert!(
            report.deployments.is_empty(),
            "Routine schedule ticks must not appear as rollouts in changed report"
        );
    }

    #[test]
    fn an_unchanged_workload_with_stale_warning_that_is_healthy_is_not_reported_as_failing() {
        let deps = [deployment("advertiser-hub", 2, 2)];
        let rs = [replicaset(
            "advertiser-hub-old",
            "advertiser-hub",
            86400 * 5,
        )];
        let events = [event(
            "ReplicaSet",
            "advertiser-hub-old",
            "Warning",
            "FailedCreate",
            300,
        )];

        let report = triage(&deps, &rs, &[], &events, &[]);
        assert!(
            report.deployments.is_empty(),
            "A healthy 2/2 workload with an old warning must not appear as FailingOnly"
        );
    }

    #[test]
    fn a_workload_with_restart_surge_or_probe_failures_is_reported_as_flapping() {
        use k8s_openapi::api::core::v1::ContainerStateRunning;
        let now = Timestamp::from_second(1_700_000_000).unwrap();
        let dep = deployment("api", 1, 1);
        let rs = [replicaset("api-rs", "api", 86400 * 3)];
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("api-rs-pod".to_string()),
                namespace: Some("default".to_string()),
                creation_timestamp: Some(Time(
                    Timestamp::from_second(1_700_000_000 - 300).unwrap(),
                )),
                owner_references: Some(vec![make_owner_ref("ReplicaSet", "api-rs")]),
                ..Default::default()
            },
            spec: Some(PodSpec::default()),
            status: Some(PodStatus {
                phase: Some("Running".to_string()),
                container_statuses: Some(vec![ContainerStatus {
                    name: "api".to_string(),
                    ready: true,
                    restart_count: 3,
                    state: Some(ContainerState {
                        running: Some(ContainerStateRunning {
                            started_at: Some(Time(
                                Timestamp::from_second(1_700_000_000 - 100).unwrap(),
                            )),
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
            &[],
            &[],
            &[],
            &rs,
            &[pod],
            &[],
            &[],
            Duration::from_secs(3600),
            now,
            None,
            TriageOptions::default(),
        );

        assert_eq!(report.deployments.len(), 1);
        assert_eq!(
            report.deployments[0].incident_status,
            IncidentStatus::Flapping
        );
        assert_eq!(report.summary.flapping_count, 1);
    }
}
