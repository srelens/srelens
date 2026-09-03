//! The `k8s.listNetworkPolicies` capability.

use std::sync::Arc;

use k8s_openapi::api::networking::v1::NetworkPolicy;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListNetworkPoliciesIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct NetworkPolicySummary {
    pub name: String,
    pub namespace: String,
    /// Pod selector summary ("all pods" when the selector is empty).
    #[serde(rename = "podSelector")]
    pub pod_selector: String,
    /// Number of ingress rules.
    pub ingress: i32,
    /// Number of egress rules.
    pub egress: i32,
    /// Comma-joined `policyTypes` (e.g. "Ingress, Egress").
    #[serde(rename = "policyTypes")]
    pub policy_types: String,
    pub age: String,
    /// Raw ISO 8601 timestamp `age` derives from, so UIs can recompute the
    /// age live at render time. Empty when the resource carries none.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListNetworkPoliciesOut {
    pub networkpolicies: Vec<NetworkPolicySummary>,
}

/// Render a label selector's `matchLabels` as "k=v, k=v", or "all pods" when empty.
fn summarise_selector(sel: &LabelSelector) -> String {
    let has_expr = sel
        .match_expressions
        .as_ref()
        .is_some_and(|e| !e.is_empty());
    match sel.match_labels.as_ref() {
        Some(labels) if !labels.is_empty() => labels
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", "),
        _ if has_expr => "matchExpressions".to_string(),
        _ => "all pods".to_string(),
    }
}

pub(crate) fn summarise(np: NetworkPolicy) -> NetworkPolicySummary {
    let name = np.metadata.name.clone().unwrap_or_default();
    let namespace = np.metadata.namespace.clone().unwrap_or_default();
    let spec = np.spec.as_ref();
    let pod_selector = spec
        .and_then(|s| s.pod_selector.as_ref())
        .map(summarise_selector)
        .unwrap_or_else(|| "all pods".into());
    let ingress = spec.and_then(|s| s.ingress.as_ref()).map_or(0, |r| r.len()) as i32;
    let egress = spec.and_then(|s| s.egress.as_ref()).map_or(0, |r| r.len()) as i32;
    let policy_types = spec
        .and_then(|s| s.policy_types.as_ref())
        .map(|t| t.join(", "))
        .unwrap_or_default();
    NetworkPolicySummary {
        name,
        namespace,
        pod_selector,
        ingress,
        egress,
        policy_types,
        age: crate::humanize_age(np.metadata.creation_timestamp.as_ref()),
        created_at: crate::creation_timestamp_iso(np.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listNetworkPolicies` — list NetworkPolicies in a namespace.
pub fn list_networkpolicies_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListNetworkPoliciesIn, ListNetworkPoliciesOut, _, _>(
        "k8s.listNetworkPolicies",
        "list NetworkPolicies in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListNetworkPoliciesIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<NetworkPolicy> = crate::scoped_api(client, &input.namespace);
                let list =
                    tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                        .await
                        .map_err(|_| {
                            CapabilityError::Handler("list networkpolicies timed out".into())
                        })?
                        .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListNetworkPoliciesOut {
                    networkpolicies: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::networking::v1::{
        NetworkPolicyEgressRule, NetworkPolicyIngressRule, NetworkPolicySpec,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_networkpolicies_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listNetworkPolicies");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn summarises_selector_rule_counts_and_types() {
        let mut match_labels = BTreeMap::new();
        match_labels.insert("app".to_string(), "web".to_string());
        let np = NetworkPolicy {
            metadata: kube::core::ObjectMeta {
                name: Some("deny".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            spec: Some(NetworkPolicySpec {
                pod_selector: Some(LabelSelector {
                    match_labels: Some(match_labels),
                    ..Default::default()
                }),
                ingress: Some(vec![NetworkPolicyIngressRule::default()]),
                egress: Some(vec![
                    NetworkPolicyEgressRule::default(),
                    NetworkPolicyEgressRule::default(),
                ]),
                policy_types: Some(vec!["Ingress".into(), "Egress".into()]),
            }),
        };
        let s = summarise(np);
        assert_eq!(s.pod_selector, "app=web");
        assert_eq!(s.ingress, 1);
        assert_eq!(s.egress, 2);
        assert_eq!(s.policy_types, "Ingress, Egress");
    }

    #[test]
    fn empty_selector_is_all_pods() {
        let np = NetworkPolicy {
            metadata: kube::core::ObjectMeta {
                name: Some("default-deny".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            spec: Some(NetworkPolicySpec::default()),
        };
        let s = summarise(np);
        assert_eq!(s.pod_selector, "all pods");
        assert_eq!(s.ingress, 0);
        assert_eq!(s.egress, 0);
    }
}
