//! The `k8s.listDeployments` capability.

use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use k8s_openapi::api::apps::v1::{Deployment, ReplicaSet};
use k8s_openapi::api::core::v1::PodTemplateSpec;
use kube::api::{ListParams, PostParams};
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListDeploymentsIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct DeploymentSummary {
    pub name: String,
    pub namespace: String,
    pub ready: String,
    #[serde(rename = "upToDate")]
    pub up_to_date: i32,
    pub available: i32,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
    /// Raw ISO 8601 timestamp `age` derives from, so UIs can recompute the
    /// age live at render time. Empty when the resource carries none.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListDeploymentsOut {
    pub deployments: Vec<DeploymentSummary>,
}

pub(crate) fn summarise(dep: Deployment) -> DeploymentSummary {
    let name = dep.metadata.name.clone().unwrap_or_default();
    let namespace = dep.metadata.namespace.clone().unwrap_or_default();
    let desired = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0);
    let status = dep.status.as_ref();
    let ready = status.and_then(|s| s.ready_replicas).unwrap_or(0);
    let up_to_date = status.and_then(|s| s.updated_replicas).unwrap_or(0);
    let available = status.and_then(|s| s.available_replicas).unwrap_or(0);
    DeploymentSummary {
        name,
        namespace,
        ready: format!("{ready}/{desired}"),
        up_to_date,
        available,
        created: crate::creation_rfc3339(dep.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(dep.metadata.creation_timestamp.as_ref()),
        created_at: crate::creation_timestamp_iso(dep.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listDeployments` — list deployments in a namespace.
pub fn list_deployments_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListDeploymentsIn, ListDeploymentsOut, _, _>(
        "k8s.listDeployments",
        "list deployments in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListDeploymentsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Deployment> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list deployments timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListDeploymentsOut {
                    deployments: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListReplicaSetsIn {
    pub context: String,
    pub namespace: String,
    /// Name of the owning Deployment; only ReplicaSets it owns are returned.
    #[serde(rename = "ownerName")]
    pub owner_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ReplicaSetSummary {
    pub name: String,
    /// `deployment.kubernetes.io/revision`, or "" if unset.
    pub revision: String,
    pub desired: i32,
    pub ready: i32,
    pub current: i32,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
    /// Raw ISO 8601 timestamp `age` derives from, so UIs can recompute the
    /// age live at render time. Empty when the resource carries none.
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// The pod template's container images, in order (#389): what tells two
    /// revisions apart when choosing one to roll back to.
    pub images: Vec<String>,
    /// `kubernetes.io/change-cause`, when the revision was recorded with one.
    #[serde(rename = "changeCause", skip_serializing_if = "Option::is_none")]
    pub change_cause: Option<String>,
    /// Whether this revision's pod template is the one the Deployment runs now
    /// (PR #810 review) — the revision `k8s.rolloutUndo` refuses as already
    /// running. The numbering alone does not say: right after a template
    /// change the newest ReplicaSet can still be the previous template. False
    /// for every row when the Deployment is gone (a failed read of it fails
    /// the listing instead — see `owner_read`).
    #[serde(rename = "currentTemplate")]
    pub current_template: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListReplicaSetsOut {
    pub replicasets: Vec<ReplicaSetSummary>,
}

fn owned_by(rs: &ReplicaSet, deployment: &str) -> bool {
    rs.metadata
        .owner_references
        .iter()
        .flatten()
        .any(|o| o.kind == "Deployment" && o.name == deployment)
}

pub(crate) fn summarise_rs(rs: ReplicaSet) -> ReplicaSetSummary {
    let revision = rs
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get("deployment.kubernetes.io/revision").cloned())
        .unwrap_or_default();
    let desired = rs.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0);
    let status = rs.status.as_ref();
    let ready = status.and_then(|s| s.ready_replicas).unwrap_or(0);
    let current = status.map(|s| s.replicas).unwrap_or(0);
    let images = rs
        .spec
        .as_ref()
        .and_then(|s| s.template.as_ref())
        .and_then(|t| t.spec.as_ref())
        .map(|p| p.containers.iter().filter_map(|c| c.image.clone()).collect())
        .unwrap_or_default();
    let change_cause = rs
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get("kubernetes.io/change-cause").cloned());
    ReplicaSetSummary {
        name: rs.metadata.name.clone().unwrap_or_default(),
        revision,
        desired,
        ready,
        current,
        created: crate::creation_rfc3339(rs.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(rs.metadata.creation_timestamp.as_ref()),
        created_at: crate::creation_timestamp_iso(rs.metadata.creation_timestamp.as_ref()),
        images,
        change_cause,
        current_template: false,
    }
}

/// Whether `rs` is one of `dep`'s revisions — by its owner's name and, when
/// the Deployment was read with one, its UID (PR #810 review). A Deployment
/// deleted and recreated under the same name leaves the old one's ReplicaSets
/// until they are collected, and a revision 1 of those is not this one's.
pub(crate) fn owned_by_deployment(rs: &ReplicaSet, dep: &Deployment) -> bool {
    let name = dep.metadata.name.as_deref().unwrap_or_default();
    let uid = dep.metadata.uid.as_deref();
    rs.metadata
        .owner_references
        .iter()
        .flatten()
        .any(|o| o.kind == "Deployment" && o.name == name && uid.is_none_or(|uid| o.uid == uid))
}

/// The owner Deployment as `k8s.listReplicaSets` takes it, from its read
/// (`None` when the read timed out): the Deployment, or `None` when it is gone
/// — both answers — or the failure, which is the listing's failure too (PR #810
/// review). A list quietly fallen back to names would mark no revision as the
/// one the Deployment runs, and present a failed call as a fact about it.
pub(crate) fn owner_read(read: Option<Result<Option<Deployment>, kube::Error>>) -> Result<Option<Deployment>, String> {
    match read {
        None => Err("reading the Deployment timed out".into()),
        Some(Err(e)) => Err(e.to_string()),
        Some(Ok(owner)) => Ok(owner),
    }
}

/// A Deployment's revisions, newest first, as `k8s.listReplicaSets` lists
/// them. With the Deployment read (`owner`), only its own ReplicaSets, and the
/// one whose template it runs marked `current_template`; without it, by name,
/// none marked.
pub(crate) fn revision_rows(owner: Option<&Deployment>, owner_name: &str, rss: Vec<ReplicaSet>) -> Vec<ReplicaSetSummary> {
    let running = owner.and_then(|dep| dep.spec.as_ref()).map(|spec| &spec.template);
    let mut rows: Vec<ReplicaSetSummary> = rss
        .into_iter()
        .filter(|rs| match owner {
            Some(dep) => owned_by_deployment(rs, dep),
            None => owned_by(rs, owner_name),
        })
        .map(|rs| {
            let current = running.is_some_and(|template| *template == template_of(&rs));
            let mut row = summarise_rs(rs);
            row.current_template = current;
            row
        })
        .collect();
    rows.sort_by(|a, b| {
        let pa = a.revision.parse::<i64>().unwrap_or(0);
        let pb = b.revision.parse::<i64>().unwrap_or(0);
        pb.cmp(&pa)
    });
    rows
}

const REVISION_ANNOTATION: &str = "deployment.kubernetes.io/revision";

/// The annotations `kubectl rollout undo` keeps from the Deployment rather
/// than taking from the target revision's ReplicaSet: bookkeeping, not intent.
pub(crate) const ROLLBACK_SKIPPED_ANNOTATIONS: &[&str] = &[
    "kubectl.kubernetes.io/last-applied-configuration",
    REVISION_ANNOTATION,
    "deployment.kubernetes.io/revision-history",
    "deployment.kubernetes.io/desired-replicas",
    "deployment.kubernetes.io/max-replicas",
    "deprecated.deployment.rollback.to",
];

/// The revision a ReplicaSet was rolled out as, when it says and says a number.
pub(crate) fn revision_of(rs: &ReplicaSet) -> Option<i64> {
    rs.metadata.annotations.as_ref()?.get(REVISION_ANNOTATION)?.parse().ok()
}

/// The ReplicaSet carrying `revision` (#389), or the refusal naming the
/// revisions there are, newest first — the list the caller should pick from.
pub(crate) fn pick_revision<'a>(
    rss: &'a [ReplicaSet],
    namespace: &str,
    name: &str,
    revision: i64,
) -> Result<&'a ReplicaSet, String> {
    if let Some(rs) = rss.iter().find(|rs| revision_of(rs) == Some(revision)) {
        return Ok(rs);
    }
    let mut have: Vec<i64> = rss.iter().filter_map(revision_of).collect();
    have.sort_unstable_by(|a, b| b.cmp(a));
    let listed = if have.is_empty() {
        "it has no revisions".to_string()
    } else {
        format!("revisions: {}", have.iter().map(i64::to_string).collect::<Vec<_>>().join(", "))
    };
    Err(format!("Deployment {namespace}/{name} has no revision {revision} ({listed})"))
}

/// A revision's pod template as its Deployment would hold it: without the
/// `pod-template-hash` label the controller adds to the ReplicaSet's copy.
pub(crate) fn template_of(rs: &ReplicaSet) -> PodTemplateSpec {
    let mut template = rs.spec.as_ref().and_then(|s| s.template.clone()).unwrap_or_default();
    if let Some(labels) = template.metadata.as_mut().and_then(|m| m.labels.as_mut()) {
        labels.remove("pod-template-hash");
    }
    template
}

/// Why a rollback to `revision` must not be written, if it must not: a paused
/// Deployment would take the template and roll nothing out (kubectl refuses
/// it too), and one already running that template would be told it was
/// rolled back when nothing changed.
pub(crate) fn undo_refusal(dep: &Deployment, target: &ReplicaSet, revision: i64) -> Option<String> {
    let namespace = dep.metadata.namespace.as_deref().unwrap_or_default();
    let name = dep.metadata.name.as_deref().unwrap_or_default();
    let spec = dep.spec.as_ref()?;
    if spec.paused == Some(true) {
        return Some(format!("Deployment {namespace}/{name} is paused; resume it before rolling back"));
    }
    if spec.template == template_of(target) {
        return Some(format!("Deployment {namespace}/{name} already runs revision {revision}"));
    }
    None
}

/// The Deployment as the rollback leaves it — exactly `kubectl rollout undo`:
/// the revision's template, the Deployment's own values for the bookkeeping
/// annotations, and every other annotation taken from the revision (so the
/// new rollout carries that revision's change-cause).
pub(crate) fn rolled_back(mut dep: Deployment, target: &ReplicaSet) -> Deployment {
    if let Some(spec) = dep.spec.as_mut() {
        spec.template = template_of(target);
    }
    let skipped = |k: &String| ROLLBACK_SKIPPED_ANNOTATIONS.contains(&k.as_str());
    let mut annotations = std::collections::BTreeMap::new();
    for (k, v) in dep.metadata.annotations.iter().flatten().filter(|(k, _)| skipped(k)) {
        annotations.insert(k.clone(), v.clone());
    }
    for (k, v) in target.metadata.annotations.iter().flatten().filter(|(k, _)| !skipped(k)) {
        annotations.insert(k.clone(), v.clone());
    }
    dep.metadata.annotations = Some(annotations);
    dep
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RolloutUndoIn {
    pub context: String,
    pub namespace: String,
    /// The Deployment to roll back.
    pub name: String,
    /// The revision to roll back to — `deployment.kubernetes.io/revision` of
    /// one of its ReplicaSets, as `k8s.listReplicaSets` lists them. Required:
    /// the confirmation shows which revision the cluster is about to run.
    pub revision: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RolloutUndoOut {
    pub name: String,
    /// The revision the Deployment was rolled back to.
    pub revision: i64,
}

/// How many times a rollback re-reads and writes again on a 409 before it
/// gives up.
const UNDO_ATTEMPTS: usize = 5;

/// `k8s.rolloutUndo` — roll a Deployment back to an earlier revision (#389),
/// the `kubectl rollout undo --to-revision` mechanism. Requires confirmation.
pub fn rollout_undo_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<RolloutUndoIn, RolloutUndoOut, _, _>(
        "k8s.rolloutUndo",
        "roll a Deployment back to an earlier revision (kubectl rollout undo --to-revision)",
        Annotations::MUTATING
            .with_confirm("Roll back Deployment[ {namespace}/{name}][ in cluster {cluster}] to an earlier revision?"),
        move |input: RolloutUndoIn| {
            let cache = cache.clone();
            async move {
                if input.revision < 1 {
                    return Err(CapabilityError::Handler(format!(
                        "revision must be 1 or more, not {}",
                        input.revision
                    )));
                }
                let client = cache.get(&input.context).await.map_err(CapabilityError::Handler)?;
                let deployments: Api<Deployment> = crate::scoped_api(client.clone(), &input.namespace);
                let replicasets: Api<ReplicaSet> = crate::scoped_api(client, &input.namespace);
                let timed_out = |what: &str| CapabilityError::Handler(format!("rollback timed out while it {what}"));
                // The controller writes the Deployment's status throughout a
                // rollout — exactly when a rollback is wanted — so a replace
                // on a stale read conflicts. Read, check and write again, as
                // client-go's RetryOnConflict does: the change is the target
                // revision's, so writing it again is the same write, and the
                // refusals are checked against the fresh read every time.
                for attempt in 1..=UNDO_ATTEMPTS {
                    let dep = tokio::time::timeout(request_timeout(), deployments.get(&input.name))
                        .await
                        .map_err(|_| timed_out("read the Deployment"))?
                        .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                    let list = tokio::time::timeout(request_timeout(), replicasets.list(&ListParams::default()))
                        .await
                        .map_err(|_| timed_out("listed its revisions"))?
                        .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                    let owned: Vec<ReplicaSet> =
                        list.items.into_iter().filter(|rs| owned_by_deployment(rs, &dep)).collect();
                    let target = pick_revision(&owned, &input.namespace, &input.name, input.revision)
                        .map_err(CapabilityError::Handler)?;
                    if let Some(refusal) = undo_refusal(&dep, target, input.revision) {
                        return Err(CapabilityError::Handler(refusal));
                    }
                    let next = rolled_back(dep, target);
                    match tokio::time::timeout(
                        request_timeout(),
                        deployments.replace(&input.name, &PostParams::default(), &next),
                    )
                    .await
                    {
                        Err(_) => return Err(timed_out("wrote the Deployment")),
                        Ok(Ok(_)) => return Ok(RolloutUndoOut { name: input.name.clone(), revision: input.revision }),
                        Ok(Err(kube::Error::Api(status))) if status.code == 409 && attempt < UNDO_ATTEMPTS => continue,
                        Ok(Err(e)) => return Err(CapabilityError::Handler(e.to_string())),
                    }
                }
                unreachable!("the last attempt returns its result")
            }
        },
    )
}

/// `k8s.listReplicaSets` — ReplicaSets owned by a Deployment, newest revision
/// first. Powers the "Deploy Revisions" section of the deployment detail.
pub fn list_replicasets_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListReplicaSetsIn, ListReplicaSetsOut, _, _>(
        "k8s.listReplicaSets",
        "list the ReplicaSets owned by a Deployment (its rollout revisions)",
        Annotations::READ_ONLY,
        move |input: ListReplicaSetsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let deployments: Api<Deployment> = crate::scoped_api(client.clone(), &input.namespace);
                let api: Api<ReplicaSet> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list replicasets timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                // The owner itself: it is what scopes the list to this
                // Deployment's UID and says which revision it runs (PR #810
                // review). Gone, the list is by name and marks none; a failed
                // read is the listing's failure — see `owner_read`.
                let read = tokio::time::timeout(request_timeout(), deployments.get_opt(&input.owner_name))
                    .await
                    .ok();
                let owner = owner_read(read).map_err(CapabilityError::Handler)?;
                Ok(ListReplicaSetsOut { replicasets: revision_rows(owner.as_ref(), &input.owner_name, list.items) })
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
    use k8s_openapi::api::core::v1::{Container, PodSpec, PodTemplateSpec};
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_deployments_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listDeployments");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn summarises_ready_ratio() {
        let dep = Deployment {
            metadata: kube::core::ObjectMeta {
                name: Some("web".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                replicas: Some(3),
                ..Default::default()
            }),
            status: Some(DeploymentStatus {
                ready_replicas: Some(2),
                updated_replicas: Some(3),
                available_replicas: Some(2),
                ..Default::default()
            }),
        };
        let s = summarise(dep);
        assert_eq!(s.ready, "2/3");
        assert_eq!(s.up_to_date, 3);
        assert_eq!(s.available, 2);
    }

    fn replicaset(name: &str, owner: &str, revision: &str) -> ReplicaSet {
        let mut annotations = std::collections::BTreeMap::new();
        annotations.insert("deployment.kubernetes.io/revision".to_string(), revision.to_string());
        ReplicaSet {
            metadata: kube::core::ObjectMeta {
                name: Some(name.into()),
                annotations: Some(annotations),
                owner_references: Some(vec![k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference {
                    kind: "Deployment".into(),
                    name: owner.into(),
                    ..Default::default()
                }]),
                ..Default::default()
            },
            spec: Some(ReplicaSetSpec {
                replicas: Some(1),
                ..Default::default()
            }),
            status: Some(ReplicaSetStatus {
                replicas: 1,
                ready_replicas: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn replicaset_capability_has_expected_id() {
        let cap = list_replicasets_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listReplicaSets");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn ownership_matches_only_named_deployment() {
        let rs = replicaset("web-abc", "web", "3");
        assert!(owned_by(&rs, "web"));
        assert!(!owned_by(&rs, "other"));
    }

    #[test]
    fn summarises_a_revisions_images_and_change_cause() {
        let mut rs = replicaset("web-abc", "web", "5");
        rs.metadata
            .annotations
            .as_mut()
            .unwrap()
            .insert("kubernetes.io/change-cause".into(), "bump api".into());
        rs.spec.as_mut().unwrap().template = Some(PodTemplateSpec {
            spec: Some(PodSpec {
                containers: vec![
                    Container { name: "api".into(), image: Some("api:1.4.2".into()), ..Default::default() },
                    Container { name: "proxy".into(), image: Some("envoy:1.30".into()), ..Default::default() },
                ],
                ..Default::default()
            }),
            ..Default::default()
        });
        let s = summarise_rs(rs);
        assert_eq!(s.images, vec!["api:1.4.2".to_string(), "envoy:1.30".to_string()]);
        assert_eq!(s.change_cause.as_deref(), Some("bump api"));

        let plain = summarise_rs(replicaset("web-def", "web", "6"));
        assert!(plain.images.is_empty());
        assert_eq!(plain.change_cause, None);
        let json = serde_json::to_value(&plain).unwrap();
        assert!(json.get("changeCause").is_none(), "an unset change-cause is omitted: {json}");
    }

    fn annotations(pairs: &[(&str, &str)]) -> Option<std::collections::BTreeMap<String, String>> {
        Some(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
    }

    fn template(image: &str, hash: Option<&str>) -> PodTemplateSpec {
        let mut labels = std::collections::BTreeMap::from([("app".to_string(), "web".to_string())]);
        if let Some(hash) = hash {
            labels.insert("pod-template-hash".into(), hash.into());
        }
        PodTemplateSpec {
            metadata: Some(kube::core::ObjectMeta { labels: Some(labels), ..Default::default() }),
            spec: Some(PodSpec {
                containers: vec![Container { name: "api".into(), image: Some(image.into()), ..Default::default() }],
                ..Default::default()
            }),
        }
    }

    /// The Deployment `shop/web`, running `image`.
    fn deploy_at(image: &str, notes: &[(&str, &str)]) -> Deployment {
        Deployment {
            metadata: kube::core::ObjectMeta {
                name: Some("web".into()),
                namespace: Some("shop".into()),
                annotations: annotations(notes),
                ..Default::default()
            },
            spec: Some(DeploymentSpec { template: template(image, None), ..Default::default() }),
            status: None,
        }
    }

    /// `web`'s ReplicaSet for `revision`, whose template runs `image`.
    fn rs_at(revision: i64, image: &str, notes: &[(&str, &str)]) -> ReplicaSet {
        let mut rs = replicaset(&format!("web-h{revision}"), "web", &revision.to_string());
        let ann = rs.metadata.annotations.get_or_insert_with(Default::default);
        for (k, v) in notes {
            ann.insert(k.to_string(), v.to_string());
        }
        rs.spec.as_mut().unwrap().template = Some(template(image, Some(&format!("h{revision}"))));
        rs
    }

    fn with_owner_uid(mut rs: ReplicaSet, uid: &str) -> ReplicaSet {
        rs.metadata.owner_references.as_mut().unwrap()[0].uid = uid.into();
        rs
    }

    fn deploy_with_uid(image: &str, uid: &str) -> Deployment {
        let mut dep = deploy_at(image, &[]);
        dep.metadata.uid = Some(uid.into());
        dep
    }

    /// PR #810 review: a Deployment deleted and recreated under the same name
    /// leaves the old one's ReplicaSets until they are collected — revisions
    /// that are not this Deployment's.
    #[test]
    fn a_deployments_revisions_are_its_own_not_a_deleted_namesakes() {
        let dep = deploy_with_uid("api:3", "u-new");
        assert!(owned_by_deployment(&with_owner_uid(rs_at(1, "api:1", &[]), "u-new"), &dep));
        assert!(!owned_by_deployment(&with_owner_uid(rs_at(1, "api:0", &[]), "u-old"), &dep));
        // Read without a uid, a Deployment falls back to its name.
        assert!(owned_by_deployment(&rs_at(1, "api:1", &[]), &deploy_at("api:3", &[])));
    }

    /// PR #810 review: the newest ReplicaSet is not always the template the
    /// Deployment runs — right after a template change it can still be the
    /// previous one — so the row that runs it says so, by template.
    #[test]
    fn revision_rows_mark_the_template_the_deployment_runs_and_drop_a_namesakes() {
        let dep = deploy_with_uid("api:2", "u-new");
        let rows = revision_rows(
            Some(&dep),
            "web",
            vec![
                with_owner_uid(rs_at(1, "api:1", &[]), "u-new"),
                with_owner_uid(rs_at(3, "api:3", &[]), "u-new"),
                with_owner_uid(rs_at(2, "api:2", &[]), "u-new"),
                with_owner_uid(rs_at(9, "api:9", &[]), "u-old"),
            ],
        );
        let seen: Vec<(&str, bool)> = rows.iter().map(|r| (r.revision.as_str(), r.current_template)).collect();
        assert_eq!(seen, vec![("3", false), ("2", true), ("1", false)]);
    }

    /// PR #810 review: a failed read of the owner is the listing's failure. A
    /// list quietly fallen back to names would mark no revision current and
    /// present that as a fact about the cluster; only a Deployment that is
    /// really gone (`None`, an answer) lists by name.
    #[test]
    fn a_failed_owner_read_fails_the_listing_and_only_a_missing_owner_falls_back() {
        assert!(owner_read(Some(Ok(Some(deploy_at("api:1", &[]))))).unwrap().is_some());
        assert!(owner_read(Some(Ok(None))).unwrap().is_none());
        let forbidden = kube::Error::Api(Box::new(
            kube::core::Status::failure("deployments.apps \"web\" is forbidden", "Forbidden").with_code(403),
        ));
        let refused = owner_read(Some(Err(forbidden))).unwrap_err();
        assert!(refused.to_lowercase().contains("forbidden"), "{refused}");
        assert!(owner_read(None).unwrap_err().contains("timed out"));
    }

    #[test]
    fn revision_rows_without_the_deployment_fall_back_to_its_name_and_mark_none() {
        let rows = revision_rows(None, "web", vec![rs_at(1, "api:1", &[]), rs_at(2, "api:2", &[])]);
        let seen: Vec<(&str, bool)> = rows.iter().map(|r| (r.revision.as_str(), r.current_template)).collect();
        assert_eq!(seen, vec![("2", false), ("1", false)]);
    }

    #[test]
    fn picks_the_replicaset_carrying_the_revision_or_names_the_ones_there_are() {
        let rss = vec![rs_at(3, "api:3", &[]), rs_at(2, "api:2", &[]), rs_at(1, "api:1", &[])];
        assert_eq!(pick_revision(&rss, "shop", "web", 2).unwrap().metadata.name.as_deref(), Some("web-h2"));
        assert_eq!(
            pick_revision(&rss, "shop", "web", 7).unwrap_err(),
            "Deployment shop/web has no revision 7 (revisions: 3, 2, 1)"
        );
        assert_eq!(
            pick_revision(&[], "shop", "web", 1).unwrap_err(),
            "Deployment shop/web has no revision 1 (it has no revisions)"
        );
    }

    #[test]
    fn a_revisions_template_is_its_deployments_without_the_hash_label() {
        let t = template_of(&rs_at(2, "api:2", &[]));
        let labels = t.metadata.unwrap().labels.unwrap();
        assert!(!labels.contains_key("pod-template-hash"));
        assert_eq!(labels.get("app").map(String::as_str), Some("web"));
    }

    #[test]
    fn refuses_a_paused_deployment_and_one_already_running_the_revision() {
        let target = rs_at(2, "api:2", &[]);
        let mut paused = deploy_at("api:3", &[]);
        paused.spec.as_mut().unwrap().paused = Some(true);
        assert_eq!(
            undo_refusal(&paused, &target, 2).as_deref(),
            Some("Deployment shop/web is paused; resume it before rolling back")
        );
        assert_eq!(
            undo_refusal(&deploy_at("api:2", &[]), &target, 2).as_deref(),
            Some("Deployment shop/web already runs revision 2")
        );
        assert_eq!(undo_refusal(&deploy_at("api:3", &[]), &target, 2), None);
    }

    #[test]
    fn a_rollback_takes_the_revisions_template_and_annotations_as_kubectl_does() {
        let dep = deploy_at("api:3", &[("deployment.kubernetes.io/revision", "3"), ("team", "a")]);
        let target = rs_at(2, "api:2", &[("kubernetes.io/change-cause", "bump api")]);
        let next = rolled_back(dep, &target);

        assert_eq!(next.spec.as_ref().unwrap().template, template("api:2", None));
        let ann = next.metadata.annotations.unwrap();
        // Bookkeeping stays the Deployment's own; intent comes from the revision.
        assert_eq!(ann.get("deployment.kubernetes.io/revision").map(String::as_str), Some("3"));
        assert_eq!(ann.get("kubernetes.io/change-cause").map(String::as_str), Some("bump api"));
        assert!(!ann.contains_key("team"), "the Deployment's own non-bookkeeping annotation is replaced: {ann:?}");
    }

    #[test]
    fn rollout_undo_is_a_gated_write_that_names_its_deployment() {
        let cap = rollout_undo_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.rolloutUndo");
        assert!(cap.annotations.requires_confirm);
        assert!(!cap.annotations.read_only);
        assert!(!cap.annotations.destructive);
        assert_eq!(
            cap.annotations
                .confirm_text(&serde_json::json!({"context": "prod", "namespace": "shop", "name": "web", "revision": 2}))
                .as_deref(),
            Some("Roll back Deployment shop/web in cluster prod to an earlier revision?")
        );
    }

    #[tokio::test]
    async fn rollout_undo_refuses_a_revision_below_one_before_any_request() {
        let cap = rollout_undo_capability(ClientCache::new(PathBuf::from("/nowhere")));
        let err = (cap.handler)(serde_json::json!({"context": "c", "namespace": "shop", "name": "web", "revision": 0}))
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("revision must be 1 or more, not 0"), "{err:?}");
    }

    #[test]
    fn summarises_replicaset_revision_and_pods() {
        let s = summarise_rs(replicaset("web-abc", "web", "5"));
        assert_eq!(s.name, "web-abc");
        assert_eq!(s.revision, "5");
        assert_eq!(s.desired, 1);
        assert_eq!(s.ready, 1);
        assert_eq!(s.current, 1);
    }
}
