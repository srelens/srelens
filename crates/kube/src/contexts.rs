//! The `k8s.listContexts` capability — reads the kubeconfig and returns its
//! contexts. Surfaced to both the UI and MCP via the shared registry.

use std::path::PathBuf;
use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::client_cache::ClientCache;
use crate::connect::load_kubeconfigs;
use crate::local_cluster::classify;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ListContextsIn {
    /// Additional kubeconfig files merged after the default files.
    pub paths: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ContextDto {
    pub name: String,
    pub cluster: String,
    pub server: String,
    #[serde(rename = "isCurrent")]
    pub is_current: bool,
    /// Whether this context points at a local development cluster (kind, k3d,
    /// minikube, docker-desktop, kiac, vind, …). Classified precision-first:
    /// only a tool-generated name earns this, and cloud auth always wins as
    /// remote, so a production cluster is never marked local. See
    /// [`crate::local_cluster`].
    #[serde(rename = "isLocal")]
    pub is_local: bool,
    /// The detected local provider (e.g. `"kind"`, `"vind"`), when `isLocal`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListContextsOut {
    pub contexts: Vec<ContextDto>,
}

/// Build the capability over the shared cache. Supplying `paths` replaces the
/// additional kubeconfig files and invalidates authenticated clients.
pub fn list_contexts_capability(cache: Arc<ClientCache>, default_paths: Vec<PathBuf>) -> Capability {
    Capability::typed::<ListContextsIn, ListContextsOut, _, _>(
        "k8s.listContexts",
        "list the kube contexts available in the kubeconfig",
        Annotations::READ_ONLY,
        move |input: ListContextsIn| {
            let cache = cache.clone();
            let default_paths = default_paths.clone();
            async move {
                if let Some(additional) = input.paths {
                    let mut paths = default_paths;
                    for path in additional.into_iter().map(PathBuf::from) {
                        if !paths.contains(&path) {
                            paths.push(path);
                        }
                    }
                    cache.set_paths(paths).await;
                }
                let config = load_kubeconfigs(&cache.paths().await)
                    .map_err(CapabilityError::Handler)?;
                let current = config.current_context.unwrap_or_default();
                let clusters = config.clusters;
                let auth_infos = config.auth_infos;
                let contexts = config.contexts.into_iter().map(|named| {
                    let context = named.context.unwrap_or_default();
                    let cluster_name = context.cluster;
                    let user_name = context.user;
                    let server = clusters.iter()
                        .find(|cluster| cluster.name == cluster_name)
                        .and_then(|cluster| cluster.cluster.as_ref())
                        .and_then(|cluster| cluster.server.clone())
                        .unwrap_or_default();
                    // Resolve the user's auth to gate cloud/managed clusters out
                    // of the "local" bucket (see local_cluster::classify).
                    let auth = auth_infos.iter()
                        .find(|entry| entry.name == user_name)
                        .and_then(|entry| entry.auth_info.as_ref());
                    let exec_command = auth
                        .and_then(|info| info.exec.as_ref())
                        .and_then(|exec| exec.command.as_deref());
                    let auth_provider = auth
                        .and_then(|info| info.auth_provider.as_ref())
                        .map(|provider| provider.name.as_str());
                    let class = classify(&named.name, &cluster_name, &server, exec_command, auth_provider);
                    ContextDto {
                        is_current: named.name == current,
                        name: named.name,
                        cluster: cluster_name,
                        server,
                        is_local: class.is_local,
                        provider: class.provider.map(|provider| provider.as_str().to_string()),
                    }
                }).collect();
                Ok(ListContextsOut { contexts })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_capability::Registry;
    use serde_json::json;

    #[test]
    fn capability_has_expected_id_and_annotations() {
        let path = PathBuf::from("/nonexistent");
        let cap = list_contexts_capability(ClientCache::new(path.clone()), vec![path]);
        assert_eq!(cap.id, "k8s.listContexts");
        assert!(cap.annotations.read_only);
    }

    #[tokio::test]
    async fn reads_and_parses_a_kubeconfig_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("srelens-test-kubeconfig.yaml");
        tokio::fs::write(
            &path,
            "clusters:\n  - name: a\n    cluster: { server: https://a }\ncontexts:\n  - name: ctx-a\n    context: { cluster: a, user: user-a }\n",
        )
        .await
        .unwrap();

        let mut reg = Registry::new();
        reg.register(list_contexts_capability(ClientCache::new(path.clone()), vec![path.clone()]));
        let out = reg.invoke("k8s.listContexts", json!({})).await.unwrap();

        assert_eq!(out["contexts"][0]["name"], "ctx-a");
        assert_eq!(out["contexts"][0]["server"], "https://a");
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn missing_file_is_a_handler_error() {
        let mut reg = Registry::new();
        let path = PathBuf::from("/no/such/kubeconfig");
        reg.register(list_contexts_capability(ClientCache::new(path.clone()), vec![path]));
        let err = reg.invoke("k8s.listContexts", json!({})).await.unwrap_err();
        assert!(matches!(err, CapabilityError::Handler(_)));
    }

    #[tokio::test]
    async fn classifies_local_and_remote_contexts() {
        let dir = std::env::temp_dir();
        // Unique per process so concurrent test runs don't collide on the path.
        let path = dir.join(format!("srelens-classify-kubeconfig-{}.yaml", std::process::id()));
        // A local kind cluster (client-cert auth) and a managed EKS cluster
        // (aws exec plugin) side by side.
        tokio::fs::write(
            &path,
            r#"apiVersion: v1
kind: Config
clusters:
- name: kind-dev
  cluster: { server: "https://127.0.0.1:6443" }
- name: eks-prod
  cluster: { server: "https://abc123.gr7.us-east-1.eks.amazonaws.com" }
contexts:
- name: kind-dev
  context: { cluster: kind-dev, user: kind-dev }
- name: eks-prod
  context: { cluster: eks-prod, user: eks-prod }
users:
- name: kind-dev
  user: { client-certificate-data: abc }
- name: eks-prod
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: aws
      args: ["eks", "get-token"]
"#,
        )
        .await
        .unwrap();

        let mut reg = Registry::new();
        reg.register(list_contexts_capability(ClientCache::new(path.clone()), vec![path.clone()]));
        let out = reg.invoke("k8s.listContexts", json!({})).await.unwrap();
        let contexts = out["contexts"].as_array().unwrap();

        let kind = contexts.iter().find(|c| c["name"] == "kind-dev").unwrap();
        assert_eq!(kind["isLocal"], true);
        assert_eq!(kind["provider"], "kind");

        // Managed EKS: remote despite reachability, and `provider` is omitted.
        let eks = contexts.iter().find(|c| c["name"] == "eks-prod").unwrap();
        assert_eq!(eks["isLocal"], false);
        assert!(eks["provider"].is_null());

        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn merges_additional_kubeconfig_files() {
        let dir = std::env::temp_dir();
        let first = dir.join("srelens-contexts-first.yaml");
        let second = dir.join("srelens-contexts-second.yaml");
        tokio::fs::write(
            &first,
            "clusters:\n- name: a\n  cluster: { server: https://a }\ncontexts:\n- name: ctx-a\n  context: { cluster: a, user: user-a }\n",
        ).await.unwrap();
        tokio::fs::write(
            &second,
            "clusters:\n- name: b\n  cluster: { server: https://b }\ncontexts:\n- name: ctx-b\n  context: { cluster: b, user: user-b }\n",
        ).await.unwrap();

        let cache = ClientCache::new(first.clone());
        let mut reg = Registry::new();
        reg.register(list_contexts_capability(cache, vec![first.clone()]));
        let out = reg.invoke(
            "k8s.listContexts",
            json!({ "paths": [second.to_string_lossy()] }),
        ).await.unwrap();
        assert_eq!(out["contexts"].as_array().unwrap().len(), 2);
        assert_eq!(out["contexts"][1]["name"], "ctx-b");

        let _ = tokio::fs::remove_file(first).await;
        let _ = tokio::fs::remove_file(second).await;
    }
}
