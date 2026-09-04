//! The `k8s.listStatefulSets` capability.

use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use k8s_openapi::api::apps::v1::StatefulSet;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListStatefulSetsIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct StatefulSetSummary {
    pub name: String,
    pub namespace: String,
    /// "ready/desired", e.g. "2/3".
    pub ready: String,
    pub updated: i32,
    /// The governing headless Service name (`spec.serviceName`), or "".
    pub service: String,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListStatefulSetsOut {
    #[serde(rename = "statefulsets")]
    pub statefulsets: Vec<StatefulSetSummary>,
}

pub(crate) fn summarise(sts: StatefulSet) -> StatefulSetSummary {
    let desired = sts.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0);
    let service = sts
        .spec
        .as_ref()
        .and_then(|s| s.service_name.clone())
        .unwrap_or_default();
    let status = sts.status.as_ref();
    let ready = status.and_then(|s| s.ready_replicas).unwrap_or(0);
    let updated = status.and_then(|s| s.updated_replicas).unwrap_or(0);
    StatefulSetSummary {
        name: sts.metadata.name.clone().unwrap_or_default(),
        namespace: sts.metadata.namespace.clone().unwrap_or_default(),
        ready: format!("{ready}/{desired}"),
        updated,
        service,
        created: crate::creation_rfc3339(sts.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(sts.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listStatefulSets` — list StatefulSets in a namespace.
pub fn list_statefulsets_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListStatefulSetsIn, ListStatefulSetsOut, _, _>(
        "k8s.listStatefulSets",
        "list StatefulSets in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListStatefulSetsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<StatefulSet> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list statefulsets timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListStatefulSetsOut {
                    statefulsets: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::apps::v1::{StatefulSetSpec, StatefulSetStatus};
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_statefulsets_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listStatefulSets");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn summarises_ready_ratio_service_and_updated() {
        let sts = StatefulSet {
            metadata: kube::core::ObjectMeta {
                name: Some("pg".into()),
                namespace: Some("data".into()),
                ..Default::default()
            },
            spec: Some(StatefulSetSpec {
                replicas: Some(3),
                service_name: Some("pg-headless".into()),
                ..Default::default()
            }),
            status: Some(StatefulSetStatus {
                ready_replicas: Some(2),
                updated_replicas: Some(3),
                ..Default::default()
            }),
        };
        let s = summarise(sts);
        assert_eq!(s.name, "pg");
        assert_eq!(s.namespace, "data");
        assert_eq!(s.ready, "2/3");
        assert_eq!(s.updated, 3);
        assert_eq!(s.service, "pg-headless");
    }
}
