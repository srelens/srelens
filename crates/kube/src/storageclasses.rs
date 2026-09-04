//! The `k8s.listStorageClasses` capability (cluster-scoped).

use std::sync::Arc;

use k8s_openapi::api::storage::v1::StorageClass;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

/// Annotation marking the cluster's default StorageClass.
const DEFAULT_CLASS_ANNOTATION: &str = "storageclass.kubernetes.io/is-default-class";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListStorageClassesIn {
    pub context: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct StorageClassSummary {
    pub name: String,
    pub provisioner: String,
    #[serde(rename = "reclaimPolicy")]
    pub reclaim_policy: String,
    #[serde(rename = "volumeBindingMode")]
    pub volume_binding_mode: String,
    /// Whether this is the cluster's default StorageClass.
    pub default: bool,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListStorageClassesOut {
    pub storageclasses: Vec<StorageClassSummary>,
}

pub(crate) fn summarise(sc: StorageClass) -> StorageClassSummary {
    let name = sc.metadata.name.clone().unwrap_or_default();
    let default = sc
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(DEFAULT_CLASS_ANNOTATION))
        .map(|v| v == "true")
        .unwrap_or(false);
    StorageClassSummary {
        name,
        provisioner: sc.provisioner,
        reclaim_policy: sc.reclaim_policy.unwrap_or_default(),
        volume_binding_mode: sc.volume_binding_mode.unwrap_or_default(),
        default,
        created: crate::creation_rfc3339(sc.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(sc.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listStorageClasses` — list cluster StorageClasses.
pub fn list_storageclasses_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListStorageClassesIn, ListStorageClassesOut, _, _>(
        "k8s.listStorageClasses",
        "list StorageClasses of a connected kube context (cluster-scoped)",
        Annotations::READ_ONLY,
        move |input: ListStorageClassesIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<StorageClass> = Api::all(client);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list storageclasses timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListStorageClassesOut {
                    storageclasses: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_storageclasses_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listStorageClasses");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn summarises_default_class() {
        let mut annotations = BTreeMap::new();
        annotations.insert(DEFAULT_CLASS_ANNOTATION.to_string(), "true".to_string());
        let sc = StorageClass {
            metadata: kube::core::ObjectMeta {
                name: Some("standard".into()),
                annotations: Some(annotations),
                ..Default::default()
            },
            provisioner: "kubernetes.io/aws-ebs".into(),
            reclaim_policy: Some("Delete".into()),
            volume_binding_mode: Some("WaitForFirstConsumer".into()),
            ..Default::default()
        };
        let s = summarise(sc);
        assert_eq!(s.provisioner, "kubernetes.io/aws-ebs");
        assert_eq!(s.reclaim_policy, "Delete");
        assert_eq!(s.volume_binding_mode, "WaitForFirstConsumer");
        assert!(s.default);
    }

    #[test]
    fn non_default_when_annotation_absent() {
        let sc = StorageClass {
            metadata: kube::core::ObjectMeta {
                name: Some("slow".into()),
                ..Default::default()
            },
            provisioner: "kubernetes.io/no-provisioner".into(),
            ..Default::default()
        };
        assert!(!summarise(sc).default);
    }
}
