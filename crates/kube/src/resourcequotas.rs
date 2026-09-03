//! The `k8s.listResourceQuotas` capability.

use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use k8s_openapi::api::core::v1::ResourceQuota;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListResourceQuotasIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ResourceQuotaSummary {
    pub name: String,
    pub namespace: String,
    /// Number of resources the quota constrains (`spec.hard` entries).
    pub resources: i32,
    pub age: String,
    /// Raw ISO 8601 timestamp `age` derives from, so UIs can recompute the
    /// age live at render time. Empty when the resource carries none.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResourceQuotasOut {
    pub resourcequotas: Vec<ResourceQuotaSummary>,
}

pub(crate) fn summarise(rq: ResourceQuota) -> ResourceQuotaSummary {
    let resources = rq
        .spec
        .as_ref()
        .and_then(|s| s.hard.as_ref())
        .map_or(0, |h| h.len());
    ResourceQuotaSummary {
        name: rq.metadata.name.clone().unwrap_or_default(),
        namespace: rq.metadata.namespace.clone().unwrap_or_default(),
        resources: resources as i32,
        age: crate::humanize_age(rq.metadata.creation_timestamp.as_ref()),
        created_at: crate::creation_timestamp_iso(rq.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listResourceQuotas` — list ResourceQuotas in a namespace.
pub fn list_resourcequotas_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListResourceQuotasIn, ListResourceQuotasOut, _, _>(
        "k8s.listResourceQuotas",
        "list ResourceQuotas in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListResourceQuotasIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<ResourceQuota> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list resourcequotas timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListResourceQuotasOut {
                    resourcequotas: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::ResourceQuotaSpec;
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_resourcequotas_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listResourceQuotas");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn counts_constrained_resources() {
        let mut hard = BTreeMap::new();
        hard.insert("cpu".to_string(), Quantity("2".into()));
        hard.insert("memory".to_string(), Quantity("4Gi".into()));
        hard.insert("pods".to_string(), Quantity("10".into()));
        let rq = ResourceQuota {
            metadata: kube::core::ObjectMeta {
                name: Some("team-quota".into()),
                namespace: Some("team-a".into()),
                ..Default::default()
            },
            spec: Some(ResourceQuotaSpec {
                hard: Some(hard),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = summarise(rq);
        assert_eq!(s.name, "team-quota");
        assert_eq!(s.namespace, "team-a");
        assert_eq!(s.resources, 3);
    }
}
