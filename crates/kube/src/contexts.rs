//! The `k8s.listContexts` capability — reads the kubeconfig and returns its
//! contexts. Surfaced to both the UI and MCP via the shared registry.

use std::path::PathBuf;
use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::client_cache::ClientCache;
use crate::connect::load_kubeconfigs;

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
                let contexts = config.contexts.into_iter().map(|named| {
                    let context = named.context.unwrap_or_default();
                    let cluster_name = context.cluster;
                    let server = clusters.iter()
                        .find(|cluster| cluster.name == cluster_name)
                        .and_then(|cluster| cluster.cluster.as_ref())
                        .and_then(|cluster| cluster.server.clone())
                        .unwrap_or_default();
                    ContextDto {
                        is_current: named.name == current,
                        name: named.name,
                        cluster: cluster_name,
                        server,
                    }
                }).collect();
                Ok(ListContextsOut { contexts })
            }
        },
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteContextIn {
    pub context: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeleteContextOut {
    pub success: bool,
}

/// Build the capability that deletes a context (and its now-unused cluster and
/// user entries) from whichever kubeconfig file it was found in.
///
/// The kubeconfig is rewritten via `serde_yaml`, so any existing YAML comments
/// or formatting in the source file are not preserved — this is a known
/// limitation. The write itself is atomic: the new contents are written to a
/// temporary file in the same directory and then renamed over the original,
/// so a failure mid-write cannot leave the kubeconfig truncated or corrupted.
pub fn delete_context_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<DeleteContextIn, DeleteContextOut, _, _>(
        "k8s.deleteContext",
        "delete a context and its associated cluster and user from its kubeconfig source",
        Annotations::DESTRUCTIVE,
        move |input: DeleteContextIn| {
            let cache = cache.clone();
            async move {
                let context_name = input.context;
                let paths = cache.paths().await;
                let mut found = false;

                for path in paths {
                    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
                        continue;
                    }
                    let content = match tokio::fs::read_to_string(&path).await {
                        Ok(c) => c,
                        Err(_) => continue,
                    };

                    let mut yaml_value: serde_yaml::Value = match serde_yaml::from_str(&content) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    if let Some(mapping) = yaml_value.as_mapping_mut() {
                        if let Some(contexts) = mapping.get_mut("contexts").and_then(|c| c.as_sequence_mut()) {
                            let mut target_index = None;
                            let mut cluster_to_remove = None;
                            let mut user_to_remove = None;

                            for (i, ctx) in contexts.iter().enumerate() {
                                if let Some(name) = ctx.get("name").and_then(|n| n.as_str()) {
                                    if name == context_name {
                                        target_index = Some(i);
                                        if let Some(context_data) = ctx.get("context") {
                                            cluster_to_remove = context_data.get("cluster").and_then(|c| c.as_str()).map(String::from);
                                            user_to_remove = context_data.get("user").and_then(|u| u.as_str()).map(String::from);
                                        }
                                        break;
                                    }
                                }
                            }

                            if let Some(idx) = target_index {
                                contexts.remove(idx);

                                // Check if cluster or user are used by any other contexts in this file
                                let mut cluster_used = false;
                                let mut user_used = false;
                                for ctx in contexts.iter() {
                                    if let Some(context_data) = ctx.get("context") {
                                        if let Some(c) = context_data.get("cluster").and_then(|c| c.as_str()) {
                                            if Some(c) == cluster_to_remove.as_deref() {
                                                cluster_used = true;
                                            }
                                        }
                                        if let Some(u) = context_data.get("user").and_then(|u| u.as_str()) {
                                            if Some(u) == user_to_remove.as_deref() {
                                                user_used = true;
                                            }
                                        }
                                    }
                                }

                                // Remove the cluster if no longer used
                                if !cluster_used {
                                    if let Some(cluster_name) = cluster_to_remove {
                                        if let Some(clusters) = mapping.get_mut("clusters").and_then(|c| c.as_sequence_mut()) {
                                            if let Some(pos) = clusters.iter().position(|c| c.get("name").and_then(|n| n.as_str()) == Some(&cluster_name)) {
                                                clusters.remove(pos);
                                            }
                                        }
                                    }
                                }

                                // Remove the user if no longer used
                                if !user_used {
                                    if let Some(user_name) = user_to_remove {
                                        if let Some(users) = mapping.get_mut("users").and_then(|u| u.as_sequence_mut()) {
                                            if let Some(pos) = users.iter().position(|u| u.get("name").and_then(|n| n.as_str()) == Some(&user_name)) {
                                                users.remove(pos);
                                            }
                                        }
                                    }
                                }

                                // Update current-context if needed
                                if let Some(current_context) = mapping.get_mut("current-context") {
                                    if current_context.as_str() == Some(&context_name) {
                                        mapping.remove("current-context");
                                    }
                                }

                                // Write back atomically: write to a temp file in the same
                                // directory, then rename over the original so a mid-write
                                // failure can't truncate or corrupt the kubeconfig.
                                let updated_yaml = serde_yaml::to_string(&yaml_value)
                                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                                let dir =
                                    path.parent().unwrap_or_else(|| std::path::Path::new("."));
                                let tmp = dir.join(format!(
                                    ".{}.srelens-tmp",
                                    path.file_name()
                                        .and_then(|f| f.to_str())
                                        .unwrap_or("kubeconfig")
                                ));
                                tokio::fs::write(&tmp, &updated_yaml)
                                    .await
                                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                                #[cfg(unix)]
                                {
                                    use std::os::unix::fs::PermissionsExt;
                                    if let Ok(meta) = tokio::fs::metadata(&path).await {
                                        let _ = tokio::fs::set_permissions(
                                            &tmp,
                                            std::fs::Permissions::from_mode(
                                                meta.permissions().mode(),
                                            ),
                                        )
                                        .await;
                                    }
                                }
                                tokio::fs::rename(&tmp, &path)
                                    .await
                                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;

                                found = true;
                                break;
                            }
                        }
                    }
                }

                if found {
                    cache.invalidate(&context_name).await;
                    Ok(DeleteContextOut { success: true })
                } else {
                    Err(CapabilityError::Handler(format!(
                        "Context '{}' not found in any loaded kubeconfigs",
                        context_name
                    )))
                }
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

    #[tokio::test]
    async fn deletes_a_context_from_kubeconfig_file() {
        let dir = std::env::temp_dir();
        let unique = format!(
            "srelens-test-delete-{}-{}.yaml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let path = dir.join(unique);
        tokio::fs::write(
            &path,
            "clusters:\n  - name: a\n    cluster: { server: https://a }\ncontexts:\n  - name: ctx-a\n    context: { cluster: a, user: user-a }\nusers:\n  - name: user-a\n    user: {}\n",
        )
        .await
        .unwrap();

        let cache = ClientCache::new(path.clone());
        let mut reg = Registry::new();
        reg.register(delete_context_capability(cache.clone()));
        reg.register(list_contexts_capability(cache.clone(), vec![path.clone()]));

        // Delete the context
        let out = reg.invoke("k8s.deleteContext", json!({ "context": "ctx-a" })).await.unwrap();
        assert_eq!(out["success"], true);

        // List contexts to check that it is empty
        let out_list = reg.invoke("k8s.listContexts", json!({})).await.unwrap();
        assert_eq!(out_list["contexts"].as_array().unwrap().len(), 0);

        // Verify the file content on disk has no clusters, contexts, or users
        let file_content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(!file_content.contains("ctx-a"));
        assert!(!file_content.contains("https://a"));
        assert!(!file_content.contains("user-a"));

        let _ = tokio::fs::remove_file(&path).await;
    }
}
