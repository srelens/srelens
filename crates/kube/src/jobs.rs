//! The `k8s.listJobs` capability.

use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use k8s_openapi::api::batch::v1::Job;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListJobsIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct JobSummary {
    pub name: String,
    pub namespace: String,
    /// "succeeded/completions", e.g. "1/1"; completions "" -> "succeeded/1".
    pub completions: String,
    pub active: i32,
    pub failed: i32,
    /// Run duration (start→completion) humanized, or "" while running/pending.
    pub duration: String,
    /// Owning CronJob name from ownerReferences, or "".
    pub owner: String,
    pub age: String,
    /// Raw ISO 8601 timestamp `age` derives from, so UIs can recompute the
    /// age live at render time. Empty when the resource carries none.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListJobsOut {
    pub jobs: Vec<JobSummary>,
}

pub(crate) fn summarise(job: Job) -> JobSummary {
    let status = job.status.as_ref();
    let succeeded = status.and_then(|s| s.succeeded).unwrap_or(0);
    let completions = job.spec.as_ref().and_then(|s| s.completions).unwrap_or(1);
    let owner = job
        .metadata
        .owner_references
        .iter()
        .flatten()
        .find(|o| o.kind == "CronJob")
        .map(|o| o.name.clone())
        .unwrap_or_default();
    // Duration only once the run has both started and completed.
    let duration = match (
        status.and_then(|s| s.start_time.as_ref()),
        status.and_then(|s| s.completion_time.as_ref()),
    ) {
        (Some(start), Some(end)) => crate::format_age(end.0.duration_since(start.0).as_secs()),
        _ => String::new(),
    };
    JobSummary {
        name: job.metadata.name.clone().unwrap_or_default(),
        namespace: job.metadata.namespace.clone().unwrap_or_default(),
        completions: format!("{succeeded}/{completions}"),
        active: status.and_then(|s| s.active).unwrap_or(0),
        failed: status.and_then(|s| s.failed).unwrap_or(0),
        duration,
        owner,
        age: crate::humanize_age(job.metadata.creation_timestamp.as_ref()),
        created_at: crate::creation_timestamp_iso(job.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listJobs` — list Jobs in a namespace.
pub fn list_jobs_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListJobsIn, ListJobsOut, _, _>(
        "k8s.listJobs",
        "list Jobs in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListJobsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Job> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list jobs timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListJobsOut {
                    jobs: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::batch::v1::{JobSpec, JobStatus};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{OwnerReference, Time};
    use k8s_openapi::jiff::Timestamp;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_jobs_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listJobs");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn summarises_completions_owner_and_duration() {
        let start: Timestamp = "2026-01-01T10:00:00Z".parse().unwrap();
        let end: Timestamp = "2026-01-01T10:02:30Z".parse().unwrap();
        let job = Job {
            metadata: kube::core::ObjectMeta {
                name: Some("backup-123".into()),
                namespace: Some("ops".into()),
                owner_references: Some(vec![OwnerReference {
                    kind: "CronJob".into(),
                    name: "backup".into(),
                    ..Default::default()
                }]),
                ..Default::default()
            },
            spec: Some(JobSpec {
                completions: Some(1),
                ..Default::default()
            }),
            status: Some(JobStatus {
                succeeded: Some(1),
                active: Some(0),
                failed: Some(0),
                start_time: Some(Time(start)),
                completion_time: Some(Time(end)),
                ..Default::default()
            }),
        };
        let s = summarise(job);
        assert_eq!(s.name, "backup-123");
        assert_eq!(s.namespace, "ops");
        assert_eq!(s.completions, "1/1");
        assert_eq!(s.owner, "backup");
        assert_eq!(s.duration, "2m"); // 150s -> "2m"
    }

    #[test]
    fn running_job_has_no_duration_and_owner_absent() {
        let job = Job {
            metadata: kube::core::ObjectMeta {
                name: Some("adhoc".into()),
                ..Default::default()
            },
            spec: Some(JobSpec::default()),
            status: Some(JobStatus {
                active: Some(2),
                ..Default::default()
            }),
        };
        let s = summarise(job);
        // No spec.completions -> denominator defaults to 1 (parallelism-agnostic display).
        assert_eq!(s.completions, "0/1");
        assert_eq!(s.active, 2);
        assert_eq!(s.duration, "");
        assert_eq!(s.owner, "");
    }
}
