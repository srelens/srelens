//! The `k8s.listServiceAccounts` and `k8s.podsForServiceAccount` capabilities.

use std::sync::Arc;

use k8s_openapi::api::core::v1::{Pod, ServiceAccount};
use k8s_openapi::api::rbac::v1::{ClusterRoleBinding, RoleBinding, Subject};
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;
use crate::workloads::{summarise_pod, ListPodsOut};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListServiceAccountsIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ServiceAccountSummary {
    pub name: String,
    pub namespace: String,
    /// Number of referenced secrets.
    pub secrets: i32,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListServiceAccountsOut {
    pub serviceaccounts: Vec<ServiceAccountSummary>,
}

pub(crate) fn summarise(sa: ServiceAccount) -> ServiceAccountSummary {
    ServiceAccountSummary {
        name: sa.metadata.name.clone().unwrap_or_default(),
        namespace: sa.metadata.namespace.clone().unwrap_or_default(),
        secrets: sa.secrets.as_ref().map_or(0, |s| s.len()) as i32,
        created: crate::creation_rfc3339(sa.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(sa.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listServiceAccounts` — list ServiceAccounts in a namespace.
pub fn list_serviceaccounts_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListServiceAccountsIn, ListServiceAccountsOut, _, _>(
        "k8s.listServiceAccounts",
        "list ServiceAccounts in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListServiceAccountsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<ServiceAccount> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list serviceaccounts timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListServiceAccountsOut {
                    serviceaccounts: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PodsForServiceAccountIn {
    pub context: String,
    pub namespace: String,
    /// The ServiceAccount name whose consuming pods we want.
    pub serviceaccount: String,
}

/// The effective service account for a pod ("default" when unset).
fn pod_service_account(pod: &Pod) -> &str {
    pod.spec
        .as_ref()
        .and_then(|s| s.service_account_name.as_deref())
        .unwrap_or("default")
}

/// `k8s.podsForServiceAccount` — pods in a namespace that run as a given SA,
/// powering the "used by" view on a ServiceAccount's detail.
pub fn pods_for_service_account_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<PodsForServiceAccountIn, ListPodsOut, _, _>(
        "k8s.podsForServiceAccount",
        "list pods in a namespace running as a given ServiceAccount",
        Annotations::READ_ONLY,
        move |input: PodsForServiceAccountIn| {
            let cache = cache.clone();
            async move {
                if input.serviceaccount.is_empty() {
                    return Ok(ListPodsOut { pods: vec![] });
                }
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Pod> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list pods timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let pods = list
                    .items
                    .into_iter()
                    .filter(|p| pod_service_account(p) == input.serviceaccount)
                    .map(summarise_pod)
                    .collect();
                Ok(ListPodsOut { pods })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BindingsForServiceAccountIn {
    pub context: String,
    pub namespace: String,
    pub serviceaccount: String,
}

/// A (Cluster)RoleBinding that grants a ServiceAccount its permissions.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SaBinding {
    pub name: String,
    /// Binding namespace (absent for a ClusterRoleBinding).
    pub namespace: Option<String>,
    /// "RoleBinding" or "ClusterRoleBinding".
    pub kind: String,
    /// The granted role as "Kind/name".
    pub role: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BindingsForServiceAccountOut {
    pub bindings: Vec<SaBinding>,
}

/// Whether a subject list references the ServiceAccount `name` in `namespace`.
fn subjects_reference_sa(subjects: Option<&Vec<Subject>>, name: &str, namespace: &str) -> bool {
    subjects.is_some_and(|subs| {
        subs.iter().any(|s| {
            s.kind == "ServiceAccount"
                && s.name == name
                && s.namespace.as_deref() == Some(namespace)
        })
    })
}

/// `k8s.bindingsForServiceAccount` — the RoleBindings and ClusterRoleBindings
/// that grant a ServiceAccount its permissions (the "what can this SA do?" view).
pub fn bindings_for_service_account_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<BindingsForServiceAccountIn, BindingsForServiceAccountOut, _, _>(
        "k8s.bindingsForServiceAccount",
        "list the RoleBindings and ClusterRoleBindings that reference a ServiceAccount",
        Annotations::READ_ONLY,
        move |input: BindingsForServiceAccountIn| {
            let cache = cache.clone();
            async move {
                if input.serviceaccount.is_empty() {
                    return Ok(BindingsForServiceAccountOut { bindings: vec![] });
                }
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let rb_api: Api<RoleBinding> = crate::scoped_api(client.clone(), &input.namespace);
                let crb_api: Api<ClusterRoleBinding> = Api::all(client);
                let params = ListParams::default();
                let (rbs, crbs) = tokio::try_join!(
                    tokio::time::timeout(request_timeout(), rb_api.list(&params)),
                    tokio::time::timeout(request_timeout(), crb_api.list(&params)),
                )
                .map_err(|_| CapabilityError::Handler("list bindings timed out".into()))?;
                let rbs = rbs.map_err(|e| CapabilityError::Handler(e.to_string()))?;
                let crbs = crbs.map_err(|e| CapabilityError::Handler(e.to_string()))?;

                let mut bindings = Vec::new();
                for rb in rbs.items {
                    if subjects_reference_sa(
                        rb.subjects.as_ref(),
                        &input.serviceaccount,
                        &input.namespace,
                    ) {
                        bindings.push(SaBinding {
                            name: rb.metadata.name.clone().unwrap_or_default(),
                            namespace: rb.metadata.namespace.clone(),
                            kind: "RoleBinding".into(),
                            role: format!("{}/{}", rb.role_ref.kind, rb.role_ref.name),
                        });
                    }
                }
                for crb in crbs.items {
                    if subjects_reference_sa(
                        crb.subjects.as_ref(),
                        &input.serviceaccount,
                        &input.namespace,
                    ) {
                        bindings.push(SaBinding {
                            name: crb.metadata.name.clone().unwrap_or_default(),
                            namespace: None,
                            kind: "ClusterRoleBinding".into(),
                            role: format!("{}/{}", crb.role_ref.kind, crb.role_ref.name),
                        });
                    }
                }
                Ok(BindingsForServiceAccountOut { bindings })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{ObjectReference, PodSpec};
    use std::path::PathBuf;

    #[test]
    fn capabilities_have_expected_ids() {
        let cache = ClientCache::new(PathBuf::from("/x"));
        assert_eq!(
            list_serviceaccounts_capability(cache.clone()).id,
            "k8s.listServiceAccounts"
        );
        assert_eq!(
            pods_for_service_account_capability(cache).id,
            "k8s.podsForServiceAccount"
        );
    }

    #[test]
    fn counts_referenced_secrets() {
        let sa = ServiceAccount {
            metadata: kube::core::ObjectMeta {
                name: Some("builder".into()),
                namespace: Some("ci".into()),
                ..Default::default()
            },
            secrets: Some(vec![ObjectReference::default(), ObjectReference::default()]),
            ..Default::default()
        };
        let s = summarise(sa);
        assert_eq!(s.secrets, 2);
        assert_eq!(s.namespace, "ci");
    }

    #[test]
    fn resolves_pod_service_account_with_default() {
        let explicit = Pod {
            spec: Some(PodSpec {
                service_account_name: Some("builder".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let implicit = Pod {
            spec: Some(PodSpec::default()),
            ..Default::default()
        };
        assert_eq!(pod_service_account(&explicit), "builder");
        assert_eq!(pod_service_account(&implicit), "default");
    }

    #[test]
    fn bindings_capability_has_expected_id() {
        let cap = bindings_for_service_account_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.bindingsForServiceAccount");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn matches_subjects_by_name_and_namespace() {
        let sa_subject = Subject {
            kind: "ServiceAccount".into(),
            name: "builder".into(),
            namespace: Some("ci".into()),
            ..Default::default()
        };
        let user_subject = Subject {
            kind: "User".into(),
            name: "builder".into(),
            namespace: None,
            ..Default::default()
        };
        assert!(subjects_reference_sa(Some(&vec![sa_subject.clone()]), "builder", "ci"));
        // Wrong namespace, wrong kind, and no subjects are all misses.
        assert!(!subjects_reference_sa(Some(&vec![sa_subject]), "builder", "prod"));
        assert!(!subjects_reference_sa(Some(&vec![user_subject]), "builder", "ci"));
        assert!(!subjects_reference_sa(None, "builder", "ci"));
    }
}
