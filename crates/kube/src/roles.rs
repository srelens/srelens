//! The `k8s.listRoles` (namespaced) and `k8s.listClusterRoles` (cluster-scoped)
//! capabilities.

use std::sync::Arc;

use k8s_openapi::api::rbac::v1::{ClusterRole, Role};
use kube::api::ListParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRolesIn {
    pub context: String,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct RoleSummary {
    pub name: String,
    pub namespace: String,
    /// Number of policy rules.
    pub rules: i32,
    pub age: String,
    /// Raw ISO 8601 timestamp `age` derives from, so UIs can recompute the
    /// age live at render time. Empty when the resource carries none.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListRolesOut {
    pub roles: Vec<RoleSummary>,
}

pub(crate) fn summarise(role: Role) -> RoleSummary {
    RoleSummary {
        name: role.metadata.name.clone().unwrap_or_default(),
        namespace: role.metadata.namespace.clone().unwrap_or_default(),
        rules: role.rules.as_ref().map_or(0, |r| r.len()) as i32,
        age: crate::humanize_age(role.metadata.creation_timestamp.as_ref()),
        created_at: crate::creation_timestamp_iso(role.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listRoles` — list Roles in a namespace.
pub fn list_roles_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListRolesIn, ListRolesOut, _, _>(
        "k8s.listRoles",
        "list Roles in a namespace of a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListRolesIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<Role> = crate::scoped_api(client, &input.namespace);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list roles timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListRolesOut {
                    roles: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListClusterRolesIn {
    pub context: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ClusterRoleSummary {
    pub name: String,
    /// Number of policy rules.
    pub rules: i32,
    pub age: String,
    /// Raw ISO 8601 timestamp `age` derives from, so UIs can recompute the
    /// age live at render time. Empty when the resource carries none.
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListClusterRolesOut {
    pub clusterroles: Vec<ClusterRoleSummary>,
}

pub(crate) fn summarise_cluster(role: ClusterRole) -> ClusterRoleSummary {
    ClusterRoleSummary {
        name: role.metadata.name.clone().unwrap_or_default(),
        rules: role.rules.as_ref().map_or(0, |r| r.len()) as i32,
        age: crate::humanize_age(role.metadata.creation_timestamp.as_ref()),
        created_at: crate::creation_timestamp_iso(role.metadata.creation_timestamp.as_ref()),
    }
}

/// `k8s.listClusterRoles` — list cluster ClusterRoles.
pub fn list_clusterroles_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListClusterRolesIn, ListClusterRolesOut, _, _>(
        "k8s.listClusterRoles",
        "list ClusterRoles of a connected kube context (cluster-scoped)",
        Annotations::READ_ONLY,
        move |input: ListClusterRolesIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: Api<ClusterRole> = Api::all(client);
                let list = tokio::time::timeout(request_timeout(), api.list(&ListParams::default()))
                    .await
                    .map_err(|_| CapabilityError::Handler("list clusterroles timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListClusterRolesOut {
                    clusterroles: list.items.into_iter().map(summarise_cluster).collect(),
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::rbac::v1::PolicyRule;
    use std::path::PathBuf;

    #[test]
    fn capabilities_have_expected_ids() {
        let cache = ClientCache::new(PathBuf::from("/x"));
        assert_eq!(list_roles_capability(cache.clone()).id, "k8s.listRoles");
        assert_eq!(
            list_clusterroles_capability(cache).id,
            "k8s.listClusterRoles"
        );
    }

    #[test]
    fn counts_role_rules() {
        let role = Role {
            metadata: kube::core::ObjectMeta {
                name: Some("reader".into()),
                namespace: Some("default".into()),
                ..Default::default()
            },
            rules: Some(vec![PolicyRule::default(), PolicyRule::default()]),
        };
        let s = summarise(role);
        assert_eq!(s.rules, 2);
        assert_eq!(s.namespace, "default");
    }

    #[test]
    fn counts_cluster_role_rules() {
        let role = ClusterRole {
            metadata: kube::core::ObjectMeta {
                name: Some("admin".into()),
                ..Default::default()
            },
            rules: Some(vec![PolicyRule::default()]),
            aggregation_rule: None,
        };
        assert_eq!(summarise_cluster(role).rules, 1);
    }
}
