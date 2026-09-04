//! The `k8s.listRoleBindings` (namespaced) and `k8s.listClusterRoleBindings`
//! (cluster-scoped) capabilities.

use std::sync::Arc;

use k8s_openapi::api::rbac::v1::{ClusterRoleBinding, RoleBinding, RoleRef};
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

/// Render a binding's roleRef as "Kind/name" (e.g. "ClusterRole/view").
fn format_role_ref(role_ref: &RoleRef) -> String {
    format!("{}/{}", role_ref.kind, role_ref.name)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRoleBindingsIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct RoleBindingSummary {
    pub name: String,
    pub namespace: String,
    /// The referenced role as "Kind/name".
    pub role: String,
    /// Number of subjects.
    pub subjects: i32,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListRoleBindingsOut {
    pub rolebindings: Vec<RoleBindingSummary>,
}

pub(crate) fn summarise(rb: RoleBinding) -> RoleBindingSummary {
    RoleBindingSummary {
        name: rb.metadata.name.clone().unwrap_or_default(),
        namespace: rb.metadata.namespace.clone().unwrap_or_default(),
        role: format_role_ref(&rb.role_ref),
        subjects: rb.subjects.as_ref().map_or(0, |s| s.len()) as i32,
        created: crate::creation_rfc3339(rb.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(rb.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listRoleBindings` — list RoleBindings in a namespace.
pub fn list_rolebindings_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListRoleBindingsIn, ListRoleBindingsOut, _, _>(
        "k8s.listRoleBindings",
        "list RoleBindings in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListRoleBindingsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<RoleBinding> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list rolebindings timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListRoleBindingsOut {
                    rolebindings: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListClusterRoleBindingsIn {
    pub context: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ClusterRoleBindingSummary {
    pub name: String,
    /// The referenced role as "Kind/name".
    pub role: String,
    /// Number of subjects.
    pub subjects: i32,
    /// `creationTimestamp` (RFC 3339), so the frontend can derive a LIVE age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListClusterRoleBindingsOut {
    pub clusterrolebindings: Vec<ClusterRoleBindingSummary>,
}

pub(crate) fn summarise_cluster(crb: ClusterRoleBinding) -> ClusterRoleBindingSummary {
    ClusterRoleBindingSummary {
        name: crb.metadata.name.clone().unwrap_or_default(),
        role: format_role_ref(&crb.role_ref),
        subjects: crb.subjects.as_ref().map_or(0, |s| s.len()) as i32,
        created: crate::creation_rfc3339(crb.metadata.creation_timestamp.as_ref()),
        age: crate::humanize_age(crb.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listClusterRoleBindings` — list cluster ClusterRoleBindings.
pub fn list_clusterrolebindings_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListClusterRoleBindingsIn, ListClusterRoleBindingsOut, _, _>(
        "k8s.listClusterRoleBindings",
        "list ClusterRoleBindings of a connected kube context (cluster-scoped)",
        Annotations::READ_ONLY,
        move |input: ListClusterRoleBindingsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<ClusterRoleBinding> = Api::all(client);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| {
                        CapabilityError::Handler("list clusterrolebindings timed out".into())
                    })?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListClusterRoleBindingsOut {
                    clusterrolebindings: list
                        .items
                        .into_iter()
                        .map(summarise_cluster)
                        .collect(),
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::rbac::v1::Subject;
    use std::path::PathBuf;

    #[test]
    fn capabilities_have_expected_ids() {
        let cache = ClientCache::new(PathBuf::from("/x"));
        assert_eq!(
            list_rolebindings_capability(cache.clone()).id,
            "k8s.listRoleBindings"
        );
        assert_eq!(
            list_clusterrolebindings_capability(cache).id,
            "k8s.listClusterRoleBindings"
        );
    }

    #[test]
    fn summarises_role_ref_and_subject_count() {
        let rb = RoleBinding {
            metadata: kube::core::ObjectMeta {
                name: Some("read-pods".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            role_ref: RoleRef {
                api_group: "rbac.authorization.k8s.io".into(),
                kind: "Role".into(),
                name: "pod-reader".into(),
            },
            subjects: Some(vec![Subject {
                kind: "ServiceAccount".into(),
                name: "builder".into(),
                namespace: Some("default".into()),
                ..Default::default()
            }]),
        };
        let s = summarise(rb);
        assert_eq!(s.role, "Role/pod-reader");
        assert_eq!(s.subjects, 1);
        assert_eq!(s.namespace, "default");
    }

    #[test]
    fn summarises_cluster_role_binding() {
        let crb = ClusterRoleBinding {
            metadata: kube::core::ObjectMeta {
                name: Some("cluster-admin-binding".into()),
                ..Default::default()
            },
            role_ref: RoleRef {
                api_group: "rbac.authorization.k8s.io".into(),
                kind: "ClusterRole".into(),
                name: "cluster-admin".into(),
            },
            subjects: None,
        };
        let s = summarise_cluster(crb);
        assert_eq!(s.role, "ClusterRole/cluster-admin");
        assert_eq!(s.subjects, 0);
    }
}
