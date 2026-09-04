//! The `k8s.listPersistentVolumes` capability (cluster-scoped).

use std::sync::Arc;

use k8s_openapi::api::core::v1::PersistentVolume;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListPvsIn {
    pub context: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PvSummary {
    pub name: String,
    /// Declared capacity (e.g. "10Gi").
    pub capacity: String,
    #[serde(rename = "accessModes")]
    pub access_modes: String,
    #[serde(rename = "reclaimPolicy")]
    pub reclaim_policy: String,
    /// Bind phase: Available / Bound / Released / Failed.
    pub status: String,
    /// Bound claim as "namespace/name", empty when unbound.
    pub claim: String,
    #[serde(rename = "storageClass")]
    pub storage_class: String,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListPvsOut {
    pub persistentvolumes: Vec<PvSummary>,
}

pub(crate) fn summarise(pv: PersistentVolume) -> PvSummary {
    let name = pv.metadata.name.clone().unwrap_or_default();
    let spec = pv.spec.as_ref();
    let capacity = spec
        .and_then(|s| s.capacity.as_ref())
        .and_then(|c| c.get("storage"))
        .map(|q| q.0.clone())
        .unwrap_or_default();
    let access_modes = crate::abbreviate_access_modes(spec.and_then(|s| s.access_modes.as_ref()));
    let reclaim_policy = spec
        .and_then(|s| s.persistent_volume_reclaim_policy.clone())
        .unwrap_or_default();
    let status = pv
        .status
        .as_ref()
        .and_then(|s| s.phase.clone())
        .unwrap_or_default();
    let claim = spec
        .and_then(|s| s.claim_ref.as_ref())
        .map(|r| {
            let ns = r.namespace.clone().unwrap_or_default();
            let n = r.name.clone().unwrap_or_default();
            if ns.is_empty() {
                n
            } else {
                format!("{ns}/{n}")
            }
        })
        .unwrap_or_default();
    let storage_class = spec
        .and_then(|s| s.storage_class_name.clone())
        .unwrap_or_default();
    PvSummary {
        name,
        capacity,
        access_modes,
        reclaim_policy,
        status,
        claim,
        storage_class,
        created: crate::creation_rfc3339(pv.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(pv.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listPersistentVolumes` — list cluster PersistentVolumes.
pub fn list_pvs_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListPvsIn, ListPvsOut, _, _>(
        "k8s.listPersistentVolumes",
        "list PersistentVolumes of a connected kube context (cluster-scoped)",
        Annotations::READ_ONLY,
        move |input: ListPvsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<PersistentVolume> = Api::all(client);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list pvs timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListPvsOut {
                    persistentvolumes: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{ObjectReference, PersistentVolumeSpec, PersistentVolumeStatus};
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_pvs_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listPersistentVolumes");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn summarises_bound_volume_with_claim() {
        let mut capacity = BTreeMap::new();
        capacity.insert("storage".to_string(), Quantity("20Gi".into()));
        let pv = PersistentVolume {
            metadata: kube::core::ObjectMeta {
                name: Some("pv-123".into()),
                ..Default::default()
            },
            spec: Some(PersistentVolumeSpec {
                capacity: Some(capacity),
                access_modes: Some(vec!["ReadWriteOnce".into()]),
                persistent_volume_reclaim_policy: Some("Retain".into()),
                storage_class_name: Some("standard".into()),
                claim_ref: Some(ObjectReference {
                    namespace: Some("default".into()),
                    name: Some("data".into()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            status: Some(PersistentVolumeStatus {
                phase: Some("Bound".into()),
                ..Default::default()
            }),
        };
        let s = summarise(pv);
        assert_eq!(s.capacity, "20Gi");
        assert_eq!(s.access_modes, "RWO");
        assert_eq!(s.reclaim_policy, "Retain");
        assert_eq!(s.status, "Bound");
        assert_eq!(s.claim, "default/data");
        assert_eq!(s.storage_class, "standard");
    }
}
