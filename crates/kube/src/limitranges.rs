//! The `k8s.listLimitRanges` capability.

use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use k8s_openapi::api::core::v1::LimitRange;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListLimitRangesIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct LimitRangeSummary {
    pub name: String,
    pub namespace: String,
    /// Number of limit entries (`spec.limits`).
    pub limits: i32,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListLimitRangesOut {
    pub limitranges: Vec<LimitRangeSummary>,
}

pub(crate) fn summarise(lr: LimitRange) -> LimitRangeSummary {
    let limits = lr.spec.as_ref().map_or(0, |s| s.limits.len());
    LimitRangeSummary {
        name: lr.metadata.name.clone().unwrap_or_default(),
        namespace: lr.metadata.namespace.clone().unwrap_or_default(),
        limits: limits as i32,
        created: crate::creation_rfc3339(lr.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(lr.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listLimitRanges` — list LimitRanges in a namespace.
pub fn list_limitranges_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListLimitRangesIn, ListLimitRangesOut, _, _>(
        "k8s.listLimitRanges",
        "list LimitRanges in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListLimitRangesIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<LimitRange> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list limitranges timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListLimitRangesOut {
                    limitranges: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{LimitRangeItem, LimitRangeSpec};
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_limitranges_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listLimitRanges");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn counts_limit_entries() {
        let lr = LimitRange {
            metadata: kube::core::ObjectMeta {
                name: Some("mem-limits".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            spec: Some(LimitRangeSpec {
                limits: vec![
                    LimitRangeItem { type_: "Container".into(), ..Default::default() },
                    LimitRangeItem { type_: "Pod".into(), ..Default::default() },
                ],
            }),
        };
        let s = summarise(lr);
        assert_eq!(s.name, "mem-limits");
        assert_eq!(s.namespace, "default");
        assert_eq!(s.limits, 2);
    }
}
