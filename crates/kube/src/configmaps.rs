//! The `k8s.listConfigMaps` capability.

use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use k8s_openapi::api::core::v1::ConfigMap;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListConfigMapsIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ConfigMapSummary {
    pub name: String,
    pub namespace: String,
    /// Number of keys (`data` + `binaryData`).
    pub keys: i32,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and a summary
    /// is only rebuilt when a watch event arrives for the object — so it goes
    /// stale (#405). Prefer this; `age` stays for callers that have no clock.
    pub created: Option<String>,
    pub age: String,
    /// Raw ISO 8601 timestamp `age` derives from, so UIs can recompute the
    /// age live at render time. Empty when the resource carries none.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListConfigMapsOut {
    pub configmaps: Vec<ConfigMapSummary>,
}

pub(crate) fn summarise(cm: ConfigMap) -> ConfigMapSummary {
    let data_keys = cm.data.as_ref().map_or(0, |d| d.len());
    let binary_keys = cm.binary_data.as_ref().map_or(0, |d| d.len());
    ConfigMapSummary {
        name: cm.metadata.name.clone().unwrap_or_default(),
        namespace: cm.metadata.namespace.clone().unwrap_or_default(),
        keys: (data_keys + binary_keys) as i32,
        created: crate::creation_rfc3339(cm.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(cm.metadata.creation_timestamp.as_ref()),
        created_at: crate::creation_timestamp_iso(cm.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listConfigMaps` — list ConfigMaps in a namespace.
pub fn list_configmaps_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListConfigMapsIn, ListConfigMapsOut, _, _>(
        "k8s.listConfigMaps",
        "list ConfigMaps in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListConfigMapsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<ConfigMap> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list configmaps timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListConfigMapsOut {
                    configmaps: list.items.into_iter().map(summarise).collect(),
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
        let cap = list_configmaps_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listConfigMaps");
        assert!(cap.annotations.read_only);
    }

    /// #405: the summary must carry the raw `creationTimestamp`, not only the
    /// rendered `age`. `age` is resolved against `now` when the summary is
    /// built and a summary is rebuilt only on a watch event, so a list showing
    /// it freezes; `created` is what lets the frontend re-derive a live age.
    #[test]
    fn summary_carries_creation_timestamp_for_a_live_age() {
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
        let created = "2026-09-01T12:59:22Z".parse::<k8s_openapi::jiff::Timestamp>().unwrap();
        let cm = ConfigMap {
            metadata: kube::core::ObjectMeta {
                name: Some("demo-cm".into()),
                namespace: Some("default".into()),
                creation_timestamp: Some(Time(created)),
                ..Default::default()
            },
            ..Default::default()
        };
        let out = summarise(cm);
        assert_eq!(out.created.as_deref(), Some("2026-09-01T12:59:22Z"));

        // No timestamp => None, so the UI renders nothing rather than a wrong age.
        let bare = ConfigMap {
            metadata: kube::core::ObjectMeta { name: Some("x".into()), ..Default::default() },
            ..Default::default()
        };
        assert_eq!(summarise(bare).created, None);
    }

    #[test]
    fn counts_data_and_binary_keys() {
        let mut data = BTreeMap::new();
        data.insert("app.conf".to_string(), "level=info".to_string());
        data.insert("log.conf".to_string(), "json".to_string());
        let mut binary = BTreeMap::new();
        binary.insert("cert.der".to_string(), k8s_openapi::ByteString(vec![1, 2, 3]));
        let cm = ConfigMap {
            metadata: kube::core::ObjectMeta {
                name: Some("web-config".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            data: Some(data),
            binary_data: Some(binary),
            ..Default::default()
        };
        let s = summarise(cm);
        assert_eq!(s.name, "web-config");
        assert_eq!(s.namespace, "default");
        assert_eq!(s.keys, 3);
    }
}
