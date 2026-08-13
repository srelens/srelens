//! The `k8s.listEndpointSlices` capability.

use std::sync::Arc;

use k8s_openapi::api::discovery::v1::EndpointSlice;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

/// Label the EndpointSlice controller sets to name the owning Service.
const SERVICE_NAME_LABEL: &str = "kubernetes.io/service-name";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListEndpointSlicesIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct EndpointSliceSummary {
    pub name: String,
    pub namespace: String,
    #[serde(rename = "addressType")]
    pub address_type: String,
    /// Ready-over-total endpoint count, e.g. "3/3".
    pub endpoints: String,
    /// Comma-joined port numbers.
    pub ports: String,
    /// Owning Service (from the `kubernetes.io/service-name` label).
    pub service: String,
    pub age: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListEndpointSlicesOut {
    pub endpointslices: Vec<EndpointSliceSummary>,
}

pub(crate) fn summarise(slice: EndpointSlice) -> EndpointSliceSummary {
    let name = slice.metadata.name.clone().unwrap_or_default();
    let namespace = slice.metadata.namespace.clone().unwrap_or_default();
    let total = slice.endpoints.len();
    let ready = slice
        .endpoints
        .iter()
        // The EndpointSlice API treats an ABSENT `conditions.ready` as ready
        // ("consumers should interpret this unknown state as ready" — the
        // field is only meaningful when explicitly set). Absent conditions
        // mostly appear on manually managed slices (selector-less Services);
        // kube-controller-manager always sets `ready` on slices it owns.
        .filter(|e| e.conditions.as_ref().and_then(|c| c.ready).unwrap_or(true))
        .count();
    let ports = slice
        .ports
        .as_ref()
        .map(|ports| {
            ports
                .iter()
                .filter_map(|p| p.port.map(|n| n.to_string()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let service = slice
        .metadata
        .labels
        .as_ref()
        .and_then(|l| l.get(SERVICE_NAME_LABEL).cloned())
        .unwrap_or_default();
    EndpointSliceSummary {
        name,
        namespace,
        address_type: slice.address_type,
        endpoints: format!("{ready}/{total}"),
        ports,
        service,
        age: crate::humanize_age(slice.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listEndpointSlices` — list EndpointSlices in a namespace.
pub fn list_endpointslices_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListEndpointSlicesIn, ListEndpointSlicesOut, _, _>(
        "k8s.listEndpointSlices",
        "list EndpointSlices in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListEndpointSlicesIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<EndpointSlice> = crate::scoped_api(client, &input.namespace);
                let list =
                    tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                        .await
                        .map_err(|_| {
                            CapabilityError::Handler("list endpointslices timed out".into())
                        })?
                        .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListEndpointSlicesOut {
                    endpointslices: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::discovery::v1::{Endpoint, EndpointConditions, EndpointPort};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_endpointslices_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listEndpointSlices");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn counts_ready_endpoints_and_maps_ports_and_service() {
        let mut labels = BTreeMap::new();
        labels.insert(SERVICE_NAME_LABEL.to_string(), "web".to_string());
        let ready = Endpoint {
            conditions: Some(EndpointConditions {
                ready: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };
        let not_ready = Endpoint {
            conditions: Some(EndpointConditions {
                ready: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        };
        let slice = EndpointSlice {
            metadata: kube::core::ObjectMeta {
                name: Some("web-abc".into()),
                namespace: Some("default".into()),
                labels: Some(labels),
                ..Default::default()
            },
            address_type: "IPv4".into(),
            endpoints: vec![ready.clone(), ready, not_ready],
            ports: Some(vec![EndpointPort {
                port: Some(8080),
                ..Default::default()
            }]),
        };
        let s = summarise(slice);
        assert_eq!(s.address_type, "IPv4");
        assert_eq!(s.endpoints, "2/3");
        assert_eq!(s.ports, "8080");
        assert_eq!(s.service, "web");
    }

    #[test]
    fn an_absent_ready_condition_counts_as_ready_but_an_explicit_false_does_not() {
        // API convention (issue #191): `conditions.ready` is only meaningful
        // when set — absent conditions, or conditions without `ready`, mean
        // ready. Only an explicit `Some(false)` is not-ready. Manually
        // managed slices (selector-less Services) commonly omit conditions
        // entirely and used to show as 0/N.
        let no_conditions = Endpoint::default();
        let empty_conditions = Endpoint {
            conditions: Some(EndpointConditions::default()),
            ..Default::default()
        };
        let explicitly_not_ready = Endpoint {
            conditions: Some(EndpointConditions { ready: Some(false), ..Default::default() }),
            ..Default::default()
        };
        let slice = EndpointSlice {
            metadata: kube::core::ObjectMeta {
                name: Some("manual-abc".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            address_type: "IPv4".into(),
            endpoints: vec![no_conditions, empty_conditions, explicitly_not_ready],
            ports: None,
        };
        assert_eq!(summarise(slice).endpoints, "2/3");
    }
}
