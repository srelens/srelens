//! The `k8s.listIngresses` capability.

use std::sync::Arc;

use k8s_openapi::api::networking::v1::Ingress;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListIngressesIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct IngressSummary {
    pub name: String,
    pub namespace: String,
    /// `spec.ingressClassName` ("-" when unset).
    pub class: String,
    /// Comma-joined rule hosts ("*" when a rule has no host).
    pub hosts: String,
    /// Load-balancer address(es) from `status` (empty until provisioned).
    pub address: String,
    /// Served ports: "80" normally, "80, 443" when TLS is configured.
    pub ports: String,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListIngressesOut {
    pub ingresses: Vec<IngressSummary>,
}

pub(crate) fn summarise(ing: Ingress) -> IngressSummary {
    let name = ing.metadata.name.clone().unwrap_or_default();
    let namespace = ing.metadata.namespace.clone().unwrap_or_default();
    let spec = ing.spec.as_ref();
    let class = spec
        .and_then(|s| s.ingress_class_name.clone())
        .unwrap_or_else(|| "-".into());
    let hosts = spec
        .and_then(|s| s.rules.as_ref())
        .map(|rules| {
            rules
                .iter()
                .map(|r| r.host.clone().unwrap_or_else(|| "*".into()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let has_tls = spec
        .and_then(|s| s.tls.as_ref())
        .is_some_and(|t| !t.is_empty());
    let ports = if has_tls {
        "80, 443".to_string()
    } else {
        "80".to_string()
    };
    let address = ing
        .status
        .as_ref()
        .and_then(|st| st.load_balancer.as_ref())
        .and_then(|lb| lb.ingress.as_ref())
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i.ip.clone().or_else(|| i.hostname.clone()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    IngressSummary {
        name,
        namespace,
        class,
        hosts,
        address,
        ports,
        created: crate::creation_rfc3339(ing.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(ing.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listIngresses` — list Ingresses in a namespace.
pub fn list_ingresses_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListIngressesIn, ListIngressesOut, _, _>(
        "k8s.listIngresses",
        "list Ingresses in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListIngressesIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Ingress> = crate::scoped_api(client, &input.namespace);
                let list =
                    tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                        .await
                        .map_err(|_| CapabilityError::Handler("list ingresses timed out".into()))?
                        .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListIngressesOut {
                    ingresses: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::networking::v1::{
        IngressLoadBalancerIngress, IngressLoadBalancerStatus, IngressRule, IngressSpec,
        IngressStatus, IngressTLS,
    };
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_ingresses_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listIngresses");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn summarises_class_hosts_address_and_tls_ports() {
        let ing = Ingress {
            metadata: kube::core::ObjectMeta {
                name: Some("web".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            spec: Some(IngressSpec {
                ingress_class_name: Some("nginx".into()),
                rules: Some(vec![IngressRule {
                    host: Some("app.example.com".into()),
                    ..Default::default()
                }]),
                tls: Some(vec![IngressTLS::default()]),
                ..Default::default()
            }),
            status: Some(IngressStatus {
                load_balancer: Some(IngressLoadBalancerStatus {
                    ingress: Some(vec![IngressLoadBalancerIngress {
                        ip: Some("203.0.113.4".into()),
                        ..Default::default()
                    }]),
                }),
                ..Default::default()
            }),
        };
        let s = summarise(ing);
        assert_eq!(s.class, "nginx");
        assert_eq!(s.hosts, "app.example.com");
        assert_eq!(s.address, "203.0.113.4");
        assert_eq!(s.ports, "80, 443");
    }

    #[test]
    fn defaults_when_unset() {
        let ing = Ingress {
            metadata: kube::core::ObjectMeta {
                name: Some("bare".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let s = summarise(ing);
        assert_eq!(s.class, "-");
        assert_eq!(s.ports, "80");
        assert_eq!(s.address, "");
    }
}
