//! The `k8s.listPersistentVolumeClaims` capability.

use std::sync::Arc;

use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod};
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;
use crate::workloads::{summarise_pod, ListPodsOut};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListPvcsIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PvcSummary {
    pub name: String,
    pub namespace: String,
    /// Bind phase: Bound / Pending / Lost.
    pub status: String,
    /// Bound capacity (e.g. "10Gi"), empty until bound.
    pub capacity: String,
    #[serde(rename = "accessModes")]
    pub access_modes: String,
    #[serde(rename = "storageClass")]
    pub storage_class: String,
    /// Bound PersistentVolume name, empty until bound.
    pub volume: String,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListPvcsOut {
    pub persistentvolumeclaims: Vec<PvcSummary>,
}

pub(crate) fn summarise(pvc: PersistentVolumeClaim) -> PvcSummary {
    let name = pvc.metadata.name.clone().unwrap_or_default();
    let namespace = pvc.metadata.namespace.clone().unwrap_or_default();
    let spec = pvc.spec.as_ref();
    let status = pvc
        .status
        .as_ref()
        .and_then(|s| s.phase.clone())
        .unwrap_or_default();
    let capacity = pvc
        .status
        .as_ref()
        .and_then(|s| s.capacity.as_ref())
        .and_then(|c| c.get("storage"))
        .map(|q| q.0.clone())
        .unwrap_or_default();
    let access_modes = crate::abbreviate_access_modes(spec.and_then(|s| s.access_modes.as_ref()));
    let storage_class = spec
        .and_then(|s| s.storage_class_name.clone())
        .unwrap_or_default();
    let volume = spec.and_then(|s| s.volume_name.clone()).unwrap_or_default();
    PvcSummary {
        name,
        namespace,
        status,
        capacity,
        access_modes,
        storage_class,
        volume,
        created: crate::creation_rfc3339(pvc.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(pvc.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listPersistentVolumeClaims` — list PVCs in a namespace.
pub fn list_pvcs_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListPvcsIn, ListPvcsOut, _, _>(
        "k8s.listPersistentVolumeClaims",
        "list PersistentVolumeClaims in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListPvcsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<PersistentVolumeClaim> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list pvcs timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListPvcsOut {
                    persistentvolumeclaims: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PodsForPvcIn {
    pub context: String,
    pub namespace: String,
    /// The PersistentVolumeClaim name whose consumers we want.
    pub pvc: String,
}

/// Whether a pod mounts `pvc` via one of its `spec.volumes`.
fn pod_uses_pvc(pod: &Pod, pvc: &str) -> bool {
    pod.spec
        .as_ref()
        .and_then(|s| s.volumes.as_ref())
        .is_some_and(|volumes| {
            volumes.iter().any(|v| {
                v.persistent_volume_claim
                    .as_ref()
                    .is_some_and(|c| c.claim_name == pvc)
            })
        })
}

/// `k8s.podsForPvc` — pods in a namespace that mount a given PVC, powering the
/// "consumed by" link on a claim's detail view.
pub fn pods_for_pvc_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<PodsForPvcIn, ListPodsOut, _, _>(
        "k8s.podsForPvc",
        "list pods in a namespace that mount a given PersistentVolumeClaim",
        Annotations::READ_ONLY,
        move |input: PodsForPvcIn| {
            let cache = cache.clone();
            async move {
                if input.pvc.is_empty() {
                    return Ok(ListPodsOut { pods: vec![] });
                }
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Pod> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list pods timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let pods = list
                    .items
                    .into_iter()
                    .filter(|p| pod_uses_pvc(p, &input.pvc))
                    .map(summarise_pod)
                    .collect();
                Ok(ListPodsOut { pods })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        PersistentVolumeClaimSpec, PersistentVolumeClaimStatus, PersistentVolumeClaimVolumeSource,
        PodSpec, Volume,
    };
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_pvcs_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listPersistentVolumeClaims");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn summarises_bound_claim() {
        let mut capacity = BTreeMap::new();
        capacity.insert("storage".to_string(), Quantity("10Gi".into()));
        let pvc = PersistentVolumeClaim {
            metadata: kube::core::ObjectMeta {
                name: Some("data".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            spec: Some(PersistentVolumeClaimSpec {
                access_modes: Some(vec!["ReadWriteOnce".into()]),
                storage_class_name: Some("standard".into()),
                volume_name: Some("pv-123".into()),
                ..Default::default()
            }),
            status: Some(PersistentVolumeClaimStatus {
                phase: Some("Bound".into()),
                capacity: Some(capacity),
                ..Default::default()
            }),
        };
        let s = summarise(pvc);
        assert_eq!(s.status, "Bound");
        assert_eq!(s.capacity, "10Gi");
        assert_eq!(s.access_modes, "RWO");
        assert_eq!(s.storage_class, "standard");
        assert_eq!(s.volume, "pv-123");
    }

    #[test]
    fn pods_for_pvc_capability_has_expected_id() {
        let cap = pods_for_pvc_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.podsForPvc");
        assert!(cap.annotations.read_only);
    }

    fn pod_with_pvc(claim: Option<&str>) -> Pod {
        Pod {
            spec: Some(PodSpec {
                volumes: claim.map(|c| {
                    vec![Volume {
                        persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                            claim_name: c.into(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }]
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn detects_pods_mounting_the_claim() {
        assert!(pod_uses_pvc(&pod_with_pvc(Some("data")), "data"));
        assert!(!pod_uses_pvc(&pod_with_pvc(Some("other")), "data"));
        assert!(!pod_uses_pvc(&pod_with_pvc(None), "data"));
    }
}
