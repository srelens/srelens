//! The `k8s.listServices` capability.

use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use k8s_openapi::api::core::v1::Service;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListServicesIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ServiceSummary {
    pub name: String,
    pub namespace: String,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(rename = "clusterIP")]
    pub cluster_ip: String,
    /// The address the service is reachable on from outside the cluster:
    /// load-balancer ingress, explicit `spec.externalIPs`, or the target of an
    /// ExternalName. Empty when the service has none; `<pending>` while a
    /// LoadBalancer waits on its provider.
    #[serde(rename = "externalIP")]
    pub external_ip: String,
    pub ports: String,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
}

/// The service's external address, following `kubectl get svc`'s EXTERNAL-IP.
///
/// A LoadBalancer publishes ingress entries in `status`, each carrying an ip or
/// a hostname (AWS gives hostnames, most others ips); `spec.externalIPs` is a
/// separate, manually assigned set that any service type may carry, and both
/// are shown. An ExternalName has no address of its own — it resolves to the
/// name it points at, which is what kubectl prints in this column.
///
/// A LoadBalancer with nothing yet is `<pending>` rather than empty: waiting on
/// a cloud provider and having no external address at all are different states,
/// and the first is the one that resolves itself.
pub(crate) fn external_ip(svc: &Service) -> String {
    let spec = svc.spec.as_ref();
    let type_ = spec.and_then(|s| s.type_.as_deref()).unwrap_or("ClusterIP");
    if type_ == "ExternalName" {
        return spec.and_then(|s| s.external_name.clone()).unwrap_or_default();
    }
    let mut addresses: Vec<String> = svc
        .status
        .as_ref()
        .and_then(|st| st.load_balancer.as_ref())
        .and_then(|lb| lb.ingress.as_ref())
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i.ip.clone().or_else(|| i.hostname.clone()))
                .collect()
        })
        .unwrap_or_default();
    addresses.extend(spec.and_then(|s| s.external_ips.clone()).unwrap_or_default());
    if !addresses.is_empty() {
        return addresses.join(", ");
    }
    if type_ == "LoadBalancer" {
        return "<pending>".into();
    }
    String::new()
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListServicesOut {
    pub services: Vec<ServiceSummary>,
}

pub(crate) fn summarise(svc: Service) -> ServiceSummary {
    let external_ip = external_ip(&svc);
    let name = svc.metadata.name.clone().unwrap_or_default();
    let namespace = svc.metadata.namespace.clone().unwrap_or_default();
    let spec = svc.spec.as_ref();
    let type_ = spec
        .and_then(|s| s.type_.clone())
        .unwrap_or_else(|| "ClusterIP".into());
    let cluster_ip = spec
        .and_then(|s| s.cluster_ip.clone())
        .unwrap_or_else(|| "None".into());
    let ports = spec
        .and_then(|s| s.ports.as_ref())
        .map(|ports| {
            ports
                .iter()
                .map(|p| {
                    let proto = p.protocol.clone().unwrap_or_else(|| "TCP".into());
                    format!("{}/{}", p.port, proto)
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();
    ServiceSummary {
        name,
        namespace,
        type_,
        cluster_ip,
        external_ip,
        ports,
        created: crate::creation_rfc3339(svc.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(svc.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listServices` — list services in a namespace.
pub fn list_services_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListServicesIn, ListServicesOut, _, _>(
        "k8s.listServices",
        "list services in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListServicesIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Service> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list services timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListServicesOut {
                    services: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        LoadBalancerIngress, LoadBalancerStatus, ServicePort, ServiceSpec, ServiceStatus,
    };
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_services_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listServices");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn summarises_type_and_ports() {
        let svc = Service {
            metadata: kube::core::ObjectMeta {
                name: Some("api".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            spec: Some(ServiceSpec {
                type_: Some("ClusterIP".into()),
                cluster_ip: Some("10.0.0.1".into()),
                ports: Some(vec![ServicePort {
                    port: 80,
                    protocol: Some("TCP".into()),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = summarise(svc);
        assert_eq!(s.type_, "ClusterIP");
        assert_eq!(s.cluster_ip, "10.0.0.1");
        assert_eq!(s.ports, "80/TCP");
        assert_eq!(s.external_ip, "");
    }

    fn svc(spec: ServiceSpec, status: Option<ServiceStatus>) -> Service {
        Service {
            metadata: kube::core::ObjectMeta {
                name: Some("api".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            spec: Some(spec),
            status,
        }
    }

    fn lb_status(ingress: Vec<LoadBalancerIngress>) -> ServiceStatus {
        ServiceStatus {
            load_balancer: Some(LoadBalancerStatus {
                ingress: Some(ingress),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn reports_a_load_balancer_ip() {
        let s = summarise(svc(
            ServiceSpec {
                type_: Some("LoadBalancer".into()),
                ..Default::default()
            },
            Some(lb_status(vec![LoadBalancerIngress {
                ip: Some("34.1.2.3".into()),
                ..Default::default()
            }])),
        ));
        assert_eq!(s.external_ip, "34.1.2.3");
    }

    #[test]
    fn reports_a_load_balancer_hostname() {
        // AWS publishes a hostname instead of an ip; the column is empty
        // without this even though the service is fully provisioned.
        let s = summarise(svc(
            ServiceSpec {
                type_: Some("LoadBalancer".into()),
                ..Default::default()
            },
            Some(lb_status(vec![LoadBalancerIngress {
                hostname: Some("a1b2.elb.amazonaws.com".into()),
                ..Default::default()
            }])),
        ));
        assert_eq!(s.external_ip, "a1b2.elb.amazonaws.com");
    }

    #[test]
    fn distinguishes_a_pending_load_balancer_from_having_no_address() {
        // Waiting on a provider is a state that resolves itself; a ClusterIP
        // with no external address never will. Both would otherwise be blank.
        let pending = summarise(svc(
            ServiceSpec {
                type_: Some("LoadBalancer".into()),
                ..Default::default()
            },
            None,
        ));
        assert_eq!(pending.external_ip, "<pending>");
        let never = summarise(svc(
            ServiceSpec {
                type_: Some("ClusterIP".into()),
                ..Default::default()
            },
            None,
        ));
        assert_eq!(never.external_ip, "");
    }

    #[test]
    fn joins_manually_assigned_external_ips() {
        // spec.externalIPs is set by hand and is independent of the type.
        let s = summarise(svc(
            ServiceSpec {
                type_: Some("NodePort".into()),
                external_ips: Some(vec!["192.0.2.1".into(), "192.0.2.2".into()]),
                ..Default::default()
            },
            None,
        ));
        assert_eq!(s.external_ip, "192.0.2.1, 192.0.2.2");
    }

    #[test]
    fn shows_both_the_load_balancer_and_the_assigned_addresses() {
        let s = summarise(svc(
            ServiceSpec {
                type_: Some("LoadBalancer".into()),
                external_ips: Some(vec!["192.0.2.1".into()]),
                ..Default::default()
            },
            Some(lb_status(vec![LoadBalancerIngress {
                ip: Some("34.1.2.3".into()),
                ..Default::default()
            }])),
        ));
        assert_eq!(s.external_ip, "34.1.2.3, 192.0.2.1");
    }

    #[test]
    fn an_external_name_resolves_to_what_it_points_at() {
        let s = summarise(svc(
            ServiceSpec {
                type_: Some("ExternalName".into()),
                external_name: Some("db.example.com".into()),
                ..Default::default()
            },
            None,
        ));
        assert_eq!(s.external_ip, "db.example.com");
    }
}
