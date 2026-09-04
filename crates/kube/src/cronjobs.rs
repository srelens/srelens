//! The `k8s.listCronJobs` capability.

use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use kube::api::{ListParams, Patch, PatchParams, PostParams};
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

/// Build a one-off Job from a CronJob's `jobTemplate` (the `kubectl create job
/// --from=cronjob/x` mechanism). The Job is owned by the CronJob so it is
/// garbage-collected with it and discoverable in the CronJob's relations, but
/// `controller: false` leaves the CronJob controller's own scheduling alone.
/// `suffix` (a timestamp) makes the name unique; the whole name is truncated to
/// the 63-char Kubernetes limit.
pub(crate) fn build_manual_job(cj: &CronJob, suffix: &str) -> Result<Job, String> {
    let cj_name = cj.metadata.name.clone().ok_or("CronJob has no name")?;
    let template = cj
        .spec
        .as_ref()
        .map(|s| s.job_template.clone())
        .ok_or("CronJob has no jobTemplate")?;

    let mut name = format!("{cj_name}-{suffix}");
    if name.len() > 63 {
        name.truncate(63);
        while name.ends_with('-') {
            name.pop();
        }
    }

    let owner = cj.metadata.uid.clone().map(|uid| OwnerReference {
        api_version: "batch/v1".into(),
        kind: "CronJob".into(),
        name: cj_name,
        uid,
        controller: Some(false),
        block_owner_deletion: Some(false),
    });

    Ok(Job {
        metadata: kube::core::ObjectMeta {
            name: Some(name),
            namespace: cj.metadata.namespace.clone(),
            labels: template.metadata.and_then(|m| m.labels),
            owner_references: owner.map(|o| vec![o]),
            ..Default::default()
        },
        spec: template.spec,
        status: None,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListCronJobsIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CronJobSummary {
    pub name: String,
    pub namespace: String,
    pub schedule: String,
    pub suspended: bool,
    pub active: i32,
    /// Humanized age of the last scheduled run, or "" if never scheduled.
    #[serde(rename = "lastSchedule")]
    pub last_schedule: String,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListCronJobsOut {
    pub cronjobs: Vec<CronJobSummary>,
}

pub(crate) fn summarise(cj: CronJob) -> CronJobSummary {
    let spec = cj.spec.as_ref();
    let status = cj.status.as_ref();
    let last_schedule = status
        .and_then(|s| s.last_schedule_time.as_ref())
        .map(|t| crate::humanize_age(Some(t)))
        .unwrap_or_default();
    CronJobSummary {
        name: cj.metadata.name.clone().unwrap_or_default(),
        namespace: cj.metadata.namespace.clone().unwrap_or_default(),
        schedule: spec.map(|s| s.schedule.clone()).unwrap_or_default(),
        suspended: spec.and_then(|s| s.suspend).unwrap_or(false),
        active: status.map(|s| s.active.as_ref().map_or(0, |a| a.len() as i32)).unwrap_or(0),
        last_schedule,
        created: crate::creation_rfc3339(cj.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(cj.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listCronJobs` — list CronJobs in a namespace.
pub fn list_cronjobs_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListCronJobsIn, ListCronJobsOut, _, _>(
        "k8s.listCronJobs",
        "list CronJobs in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListCronJobsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<CronJob> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list cronjobs timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListCronJobsOut {
                    cronjobs: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CronJobActionOut {
    pub name: String,
    pub ok: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetSuspendIn {
    pub context: String,
    pub namespace: String,
    pub name: String,
    /// true = suspend scheduling, false = resume.
    pub suspend: bool,
}

/// `k8s.cronjobSetSuspend` — suspend or resume a CronJob (`spec.suspend`).
/// Reversible, so it requires confirmation but is not flagged destructive.
pub fn cronjob_set_suspend_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<SetSuspendIn, CronJobActionOut, _, _>(
        "k8s.cronjobSetSuspend",
        "suspend or resume a CronJob (set spec.suspend)",
        Annotations {
            read_only: false,
            destructive: false,
            requires_confirm: true,
            sensitive: false,
        },
        move |input: SetSuspendIn| {
            let cache = cache.clone();
            async move {
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let api: Api<CronJob> = crate::scoped_api(client, &input.namespace);
                let patch = json!({ "spec": { "suspend": input.suspend } });
                tokio::time::timeout(
                    request_timeout(),
                    api.patch(&input.name, &PatchParams::default(), &Patch::Merge(&patch)),
                )
                .await
                .map_err(|_| CapabilityError::Handler("set suspend timed out".into()))?
                .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(CronJobActionOut { name: input.name, ok: true })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TriggerNowIn {
    pub context: String,
    pub namespace: String,
    pub name: String,
    /// A unique suffix for the created Job name (a timestamp from the caller,
    /// so the handler stays deterministic and testable).
    pub suffix: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TriggerNowOut {
    /// The name of the one-off Job that was created.
    #[serde(rename = "jobName")]
    pub job_name: String,
    pub ok: bool,
}

/// `k8s.cronjobTriggerNow` — run a CronJob immediately by creating a one-off Job
/// from its `jobTemplate` (like `kubectl create job --from`). Requires
/// confirmation.
pub fn cronjob_trigger_now_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<TriggerNowIn, TriggerNowOut, _, _>(
        "k8s.cronjobTriggerNow",
        "run a CronJob immediately by creating a Job from its jobTemplate",
        Annotations {
            read_only: false,
            destructive: false,
            requires_confirm: true,
            sensitive: false,
        },
        move |input: TriggerNowIn| {
            let cache = cache.clone();
            async move {
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let api: Api<CronJob> = crate::scoped_api(client.clone(), &input.namespace);
                let cj = tokio::time::timeout(request_timeout(), api.get(&input.name))
                    .await
                    .map_err(|_| CapabilityError::Handler("get cronjob timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let job = build_manual_job(&cj, &input.suffix).map_err(CapabilityError::Handler)?;
                let job_name = job.metadata.name.clone().unwrap_or_default();
                let jobs: Api<Job> = crate::scoped_api(client, &input.namespace);
                tokio::time::timeout(request_timeout(), jobs.create(&PostParams::default(), &job))
                    .await
                    .map_err(|_| CapabilityError::Handler("create job timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(TriggerNowOut { job_name, ok: true })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::batch::v1::{CronJobSpec, CronJobStatus};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
    use k8s_openapi::jiff::Timestamp;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_cronjobs_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listCronJobs");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn suspend_and_trigger_capabilities_require_confirm() {
        let cache = ClientCache::new(PathBuf::from("/x"));
        let suspend = cronjob_set_suspend_capability(cache.clone());
        assert_eq!(suspend.id, "k8s.cronjobSetSuspend");
        assert!(suspend.annotations.requires_confirm);
        assert!(!suspend.annotations.read_only);
        // Suspend/resume is reversible, so not flagged destructive.
        assert!(!suspend.annotations.destructive);

        let trigger = cronjob_trigger_now_capability(cache);
        assert_eq!(trigger.id, "k8s.cronjobTriggerNow");
        assert!(trigger.annotations.requires_confirm);
        assert!(!trigger.annotations.read_only);
    }

    #[test]
    fn summarises_schedule_suspend_and_active() {
        let cj = CronJob {
            metadata: kube::core::ObjectMeta {
                name: Some("nightly".into()),
                namespace: Some("ops".into()),
                ..Default::default()
            },
            spec: Some(CronJobSpec {
                schedule: "0 2 * * *".into(),
                suspend: Some(true),
                ..Default::default()
            }),
            status: Some(CronJobStatus {
                active: Some(vec![Default::default(), Default::default()]),
                last_schedule_time: Some(Time(Timestamp::now())),
                ..Default::default()
            }),
        };
        let s = summarise(cj);
        assert_eq!(s.name, "nightly");
        assert_eq!(s.namespace, "ops");
        assert_eq!(s.schedule, "0 2 * * *");
        assert!(s.suspended);
        assert_eq!(s.active, 2);
        assert_eq!(s.last_schedule, "0s");
    }

    fn cronjob_with_template(name: &str) -> CronJob {
        use k8s_openapi::api::batch::v1::{JobSpec, JobTemplateSpec};
        let mut labels = std::collections::BTreeMap::new();
        labels.insert("app".to_string(), "backup".to_string());
        CronJob {
            metadata: kube::core::ObjectMeta {
                name: Some(name.into()),
                namespace: Some("ops".into()),
                uid: Some("cj-uid-123".into()),
                ..Default::default()
            },
            spec: Some(CronJobSpec {
                schedule: "0 2 * * *".into(),
                job_template: JobTemplateSpec {
                    metadata: Some(kube::core::ObjectMeta {
                        labels: Some(labels),
                        ..Default::default()
                    }),
                    spec: Some(JobSpec {
                        completions: Some(1),
                        ..Default::default()
                    }),
                },
                ..Default::default()
            }),
            status: None,
        }
    }

    #[test]
    fn builds_manual_job_with_owner_labels_and_spec() {
        let cj = cronjob_with_template("backup");
        let job = build_manual_job(&cj, "1700000000").unwrap();
        assert_eq!(job.metadata.name.as_deref(), Some("backup-1700000000"));
        assert_eq!(job.metadata.namespace.as_deref(), Some("ops"));
        assert_eq!(job.metadata.labels.as_ref().unwrap().get("app").map(String::as_str), Some("backup"));
        let owner = &job.metadata.owner_references.as_ref().unwrap()[0];
        assert_eq!(owner.kind, "CronJob");
        assert_eq!(owner.name, "backup");
        assert_eq!(owner.uid, "cj-uid-123");
        assert_eq!(owner.controller, Some(false));
        assert_eq!(job.spec.as_ref().unwrap().completions, Some(1));
    }

    #[test]
    fn truncates_manual_job_name_to_63_chars() {
        let long = "a".repeat(60);
        let cj = cronjob_with_template(&long);
        let job = build_manual_job(&cj, "1700000000").unwrap();
        let name = job.metadata.name.unwrap();
        assert!(name.len() <= 63, "name was {} chars", name.len());
        assert!(!name.ends_with('-'), "name should not end with a dash: {name}");
    }

    #[test]
    fn never_scheduled_cronjob_defaults() {
        let cj = CronJob {
            metadata: kube::core::ObjectMeta {
                name: Some("weekly".into()),
                ..Default::default()
            },
            spec: Some(CronJobSpec {
                schedule: "0 0 * * 0".into(),
                ..Default::default()
            }),
            status: Some(CronJobStatus::default()),
        };
        let s = summarise(cj);
        assert!(!s.suspended);
        assert_eq!(s.active, 0);
        assert_eq!(s.last_schedule, "");
    }
}
