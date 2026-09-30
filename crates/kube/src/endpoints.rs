//! The `k8s.listEndpoints` capability.

use std::sync::Arc;

use k8s_openapi::api::core::v1::Endpoints;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListEndpointsIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EndpointItem {
    pub ip: String,
    pub port: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "portName")]
    pub port_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    pub ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "nodeName")]
    pub node_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "podName")]
    pub pod_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EndpointSummary {
    pub name: String,
    pub namespace: String,
    /// Comma-separated list of ready endpoint IP:port addresses, following `kubectl get ep`.
    /// E.g. "10.254.98.20:15012, 10.254.98.23:15012" or "<none>".
    pub endpoints: String,
    /// Ready count vs total count, e.g. "3/3".
    #[serde(rename = "readyCount")]
    pub ready_count: String,
    /// Comma-joined port descriptions, e.g. "15012/TCP, 15010/TCP".
    pub ports: String,
    /// Structured list of individual endpoints for deep programmatic inspection.
    pub addresses: Vec<EndpointItem>,
    /// `creationTimestamp` (RFC 3339), so callers can derive a live age.
    pub created: Option<String>,
    pub age: String,
    /// Raw ISO 8601 timestamp `age` derives from.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListEndpointsOut {
    pub endpoints: Vec<EndpointSummary>,
}

pub(crate) fn summarise(ep: Endpoints) -> EndpointSummary {
    let name = ep.metadata.name.clone().unwrap_or_default();
    let namespace = ep.metadata.namespace.clone().unwrap_or_default();

    let mut ready_count = 0;
    let mut not_ready_count = 0;
    let mut ep_strings: Vec<String> = Vec::new();
    let mut port_strings: Vec<String> = Vec::new();
    let mut addresses: Vec<EndpointItem> = Vec::new();

    if let Some(subsets) = &ep.subsets {
        for subset in subsets {
            let ports = subset.ports.as_deref().unwrap_or(&[]);
            for p in ports {
                let proto = p.protocol.as_deref().unwrap_or("TCP");
                let port_desc = if let Some(ref pname) = p.name {
                    format!("{}:{}/{}", pname, p.port, proto)
                } else {
                    format!("{}/{}", p.port, proto)
                };
                if !port_strings.contains(&port_desc) {
                    port_strings.push(port_desc);
                }
            }

            if let Some(ready_addrs) = &subset.addresses {
                for addr in ready_addrs {
                    ready_count += 1;
                    let pod_name = addr.target_ref.as_ref().and_then(|tr| {
                        if tr.kind.as_deref() == Some("Pod") {
                            tr.name.clone()
                        } else {
                            None
                        }
                    });

                    if ports.is_empty() {
                        ep_strings.push(addr.ip.clone());
                        addresses.push(EndpointItem {
                            ip: addr.ip.clone(),
                            port: None,
                            port_name: None,
                            protocol: None,
                            ready: true,
                            node_name: addr.node_name.clone(),
                            pod_name,
                        });
                    } else {
                        for p in ports {
                            ep_strings.push(format!("{}:{}", addr.ip, p.port));
                            addresses.push(EndpointItem {
                                ip: addr.ip.clone(),
                                port: Some(p.port),
                                port_name: p.name.clone(),
                                protocol: p.protocol.clone(),
                                ready: true,
                                node_name: addr.node_name.clone(),
                                pod_name: pod_name.clone(),
                            });
                        }
                    }
                }
            }

            if let Some(not_ready_addrs) = &subset.not_ready_addresses {
                for addr in not_ready_addrs {
                    not_ready_count += 1;
                    let pod_name = addr.target_ref.as_ref().and_then(|tr| {
                        if tr.kind.as_deref() == Some("Pod") {
                            tr.name.clone()
                        } else {
                            None
                        }
                    });

                    if ports.is_empty() {
                        addresses.push(EndpointItem {
                            ip: addr.ip.clone(),
                            port: None,
                            port_name: None,
                            protocol: None,
                            ready: false,
                            node_name: addr.node_name.clone(),
                            pod_name,
                        });
                    } else {
                        for p in ports {
                            addresses.push(EndpointItem {
                                ip: addr.ip.clone(),
                                port: Some(p.port),
                                port_name: p.name.clone(),
                                protocol: p.protocol.clone(),
                                ready: false,
                                node_name: addr.node_name.clone(),
                                pod_name: pod_name.clone(),
                            });
                        }
                    }
                }
            }
        }
    }

    let total = ready_count + not_ready_count;
    let ready_count_str = if total == 0 {
        "0/0".to_string()
    } else {
        format!("{ready_count}/{total}")
    };

    let endpoints_str = if ep_strings.is_empty() {
        "<none>".to_string()
    } else {
        ep_strings.join(", ")
    };

    let ports_str = port_strings.join(", ");

    EndpointSummary {
        name,
        namespace,
        endpoints: endpoints_str,
        ready_count: ready_count_str,
        ports: ports_str,
        addresses,
        created: crate::creation_rfc3339(ep.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(ep.metadata.creation_timestamp.as_ref()),
        created_at: crate::creation_timestamp_iso(ep.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listEndpoints` — list Endpoints in a namespace.
pub fn list_endpoints_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListEndpointsIn, ListEndpointsOut, _, _>(
        "k8s.listEndpoints",
        "list Endpoints in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListEndpointsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Endpoints> = crate::scoped_api(client, &input.namespace);
                let list =
                    tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                        .await
                        .map_err(|_| CapabilityError::Handler("list endpoints timed out".into()))?
                        .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListEndpointsOut {
                    endpoints: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        EndpointAddress, EndpointPort, EndpointSubset, ObjectReference,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::path::PathBuf;

    #[test]
    fn capability_metadata() {
        let cap = list_endpoints_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listEndpoints");
        assert!(cap.annotations.read_only);
        assert!(!cap.annotations.destructive);
    }

    #[test]
    fn deserializes_caller_payload() {
        let raw = serde_json::json!({
            "context": "prod",
            "namespace": "istio-system"
        });
        let parsed: ListEndpointsIn =
            serde_json::from_value(raw).expect("deserializes clean input");
        assert_eq!(parsed.context, "prod");
        assert_eq!(parsed.namespace, "istio-system");
    }

    #[test]
    fn summarises_empty_endpoints() {
        let ep = Endpoints {
            metadata: ObjectMeta {
                name: Some("no-backends".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            subsets: None,
        };
        let summary = summarise(ep);
        assert_eq!(summary.name, "no-backends");
        assert_eq!(summary.namespace, "default");
        assert_eq!(summary.endpoints, "<none>");
        assert_eq!(summary.ready_count, "0/0");
        assert_eq!(summary.ports, "");
        assert!(summary.addresses.is_empty());
    }

    #[test]
    fn summarises_ready_and_not_ready_endpoints_with_ports() {
        let ep = Endpoints {
            metadata: ObjectMeta {
                name: Some("istiod".into()),
                namespace: Some("istio-system".into()),
                ..Default::default()
            },
            subsets: Some(vec![EndpointSubset {
                addresses: Some(vec![
                    EndpointAddress {
                        ip: "10.254.98.20".into(),
                        node_name: Some("node-1".into()),
                        target_ref: Some(ObjectReference {
                            kind: Some("Pod".into()),
                            name: Some("istiod-pod-1".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    EndpointAddress {
                        ip: "10.254.98.23".into(),
                        node_name: Some("node-1".into()),
                        target_ref: Some(ObjectReference {
                            kind: Some("Pod".into()),
                            name: Some("istiod-pod-2".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ]),
                not_ready_addresses: Some(vec![EndpointAddress {
                    ip: "10.254.98.99".into(),
                    node_name: Some("node-2".into()),
                    target_ref: Some(ObjectReference {
                        kind: Some("Pod".into()),
                        name: Some("istiod-pod-starting".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ports: Some(vec![
                    EndpointPort {
                        name: Some("https-dns".into()),
                        port: 15012,
                        protocol: Some("TCP".into()),
                        ..Default::default()
                    },
                    EndpointPort {
                        name: Some("grpc-xds".into()),
                        port: 15010,
                        protocol: Some("TCP".into()),
                        ..Default::default()
                    },
                ]),
            }]),
        };
        let summary = summarise(ep);
        assert_eq!(summary.name, "istiod");
        assert_eq!(summary.namespace, "istio-system");
        assert_eq!(summary.ready_count, "2/3");
        assert_eq!(
            summary.endpoints,
            "10.254.98.20:15012, 10.254.98.20:15010, 10.254.98.23:15012, 10.254.98.23:15010"
        );
        assert_eq!(summary.ports, "https-dns:15012/TCP, grpc-xds:15010/TCP");
        assert_eq!(summary.addresses.len(), 6); // 2 ready addrs * 2 ports + 1 not_ready * 2 ports

        let first = &summary.addresses[0];
        assert_eq!(first.ip, "10.254.98.20");
        assert_eq!(first.port, Some(15012));
        assert_eq!(first.port_name.as_deref(), Some("https-dns"));
        assert!(first.ready);
        assert_eq!(first.pod_name.as_deref(), Some("istiod-pod-1"));
        assert_eq!(first.node_name.as_deref(), Some("node-1"));

        let not_ready = &summary.addresses[4];
        assert_eq!(not_ready.ip, "10.254.98.99");
        assert!(!not_ready.ready);
        assert_eq!(not_ready.pod_name.as_deref(), Some("istiod-pod-starting"));
    }
}
