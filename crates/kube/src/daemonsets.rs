//! The `k8s.listDaemonSets` capability.

use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use k8s_openapi::api::apps::v1::DaemonSet;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListDaemonSetsIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct DaemonSetSummary {
    pub name: String,
    pub namespace: String,
    pub desired: i32,
    pub current: i32,
    pub ready: i32,
    #[serde(rename = "upToDate")]
    pub up_to_date: i32,
    pub available: i32,
    pub age: String,
    /// Raw ISO 8601 timestamp `age` derives from, so UIs can recompute the
    /// age live at render time. Empty when the resource carries none.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListDaemonSetsOut {
    pub daemonsets: Vec<DaemonSetSummary>,
}

pub(crate) fn summarise(ds: DaemonSet) -> DaemonSetSummary {
    let status = ds.status.as_ref();
    DaemonSetSummary {
        name: ds.metadata.name.clone().unwrap_or_default(),
        namespace: ds.metadata.namespace.clone().unwrap_or_default(),
        desired: status.map(|s| s.desired_number_scheduled).unwrap_or(0),
        current: status.map(|s| s.current_number_scheduled).unwrap_or(0),
        ready: status.map(|s| s.number_ready).unwrap_or(0),
        up_to_date: status.and_then(|s| s.updated_number_scheduled).unwrap_or(0),
        available: status.and_then(|s| s.number_available).unwrap_or(0),
        age: crate::humanize_age(ds.metadata.creation_timestamp.as_ref()),
        created_at: crate::creation_timestamp_iso(ds.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listDaemonSets` — list DaemonSets in a namespace.
pub fn list_daemonsets_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListDaemonSetsIn, ListDaemonSetsOut, _, _>(
        "k8s.listDaemonSets",
        "list DaemonSets in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListDaemonSetsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<DaemonSet> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list daemonsets timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListDaemonSetsOut {
                    daemonsets: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::apps::v1::DaemonSetStatus;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_daemonsets_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listDaemonSets");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn summarises_node_coverage_counts() {
        let ds = DaemonSet {
            metadata: kube::core::ObjectMeta {
                name: Some("fluentd".into()),
                namespace: Some("logging".into()),
                ..Default::default()
            },
            status: Some(DaemonSetStatus {
                desired_number_scheduled: 5,
                current_number_scheduled: 5,
                number_ready: 4,
                updated_number_scheduled: Some(5),
                number_available: Some(4),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = summarise(ds);
        assert_eq!(s.name, "fluentd");
        assert_eq!(s.namespace, "logging");
        assert_eq!(s.desired, 5);
        assert_eq!(s.current, 5);
        assert_eq!(s.ready, 4);
        assert_eq!(s.up_to_date, 5);
        assert_eq!(s.available, 4);
    }
}
