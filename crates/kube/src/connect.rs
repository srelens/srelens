//! Real cluster connection via kube-rs: the `k8s.clusterInfo` capability
//! connects to a named kubeconfig context and reports the server version and
//! reachability. Authentication (client certs, tokens, exec plugins) is handled
//! by kube-rs from the kubeconfig.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use srelens_capability::{Annotations, Capability};
use kube::config::{Config, KubeConfigOptions, Kubeconfig};
use kube::Client;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::client_cache::ClientCache;

/// Default per-request timeout budget (connect + list/get/apply), in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 8;
/// Smallest timeout a user may configure, in seconds.
pub const MIN_TIMEOUT_SECS: u64 = 1;
/// Largest timeout a user may configure, in seconds.
pub const MAX_TIMEOUT_SECS: u64 = 30;

/// Environment variable that overrides the default timeout at startup — lets
/// headless/MCP runs (which have no Settings UI) raise it for large clusters.
pub const TIMEOUT_ENV: &str = "SRELENS_TIMEOUT_SECS";

/// Runtime-configurable per-request timeout, shared by every capability. Kept
/// as a process-wide atomic so the Settings UI can adjust it live without
/// threading a value through the whole capability registry.
static TIMEOUT_SECS: AtomicU64 = AtomicU64::new(DEFAULT_TIMEOUT_SECS);

/// The current per-request timeout budget.
pub fn request_timeout() -> Duration {
    Duration::from_secs(TIMEOUT_SECS.load(Ordering::Relaxed))
}

/// The current per-request timeout, in seconds.
pub fn request_timeout_secs() -> u64 {
    TIMEOUT_SECS.load(Ordering::Relaxed)
}

/// Set the per-request timeout, clamping to `[MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS]`.
/// Returns the value actually applied so callers can reflect clamping back to the user.
pub fn set_request_timeout_secs(secs: u64) -> u64 {
    let clamped = secs.clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS);
    TIMEOUT_SECS.store(clamped, Ordering::Relaxed);
    clamped
}

/// Apply the `SRELENS_TIMEOUT_SECS` override if present and parseable. Invalid
/// values are ignored, leaving the default in place. Returns the applied value.
pub fn init_timeout_from_env() -> u64 {
    if let Some(raw) = std::env::var_os(TIMEOUT_ENV) {
        if let Some(secs) = raw.to_str().and_then(|s| s.trim().parse::<u64>().ok()) {
            return set_request_timeout_secs(secs);
        }
    }
    request_timeout_secs()
}

/// Build an authenticated kube-rs client for a named kubeconfig context.
/// Authentication (certs, tokens, exec plugins) is resolved by kube-rs.
pub(crate) fn load_kubeconfigs(paths: &[PathBuf]) -> Result<Kubeconfig, String> {
    paths.iter().try_fold(Kubeconfig::default(), |merged, path| {
        let next = Kubeconfig::read_from(path).map_err(|e| e.to_string())?;
        merged.merge(next).map_err(|e| e.to_string())
    })
}

pub fn validate_kubeconfig_yaml(yaml: &str) -> Result<usize, String> {
    let config = Kubeconfig::from_yaml(yaml).map_err(|error| error.to_string())?;
    if config.contexts.is_empty() {
        return Err("kubeconfig contains no contexts".to_string());
    }
    Ok(config.contexts.len())
}

/// Build a standalone kubeconfig (YAML) containing ONLY `context` plus the one
/// cluster and user it references, with `current-context` pinned to it.
///
/// Used to lock the in-app terminal to a single cluster: pointing `KUBECONFIG`
/// at this file means `kubectl config get-contexts` lists just that context and
/// `use-context` can't switch to any other.
pub fn single_context_kubeconfig_yaml(paths: &[PathBuf], context: &str) -> Result<String, String> {
    let mut config = load_kubeconfigs(paths)?;
    let named = config
        .contexts
        .iter()
        .find(|c| c.name == context)
        .ok_or_else(|| format!("context '{context}' not found in kubeconfig"))?;
    let inner = named
        .context
        .clone()
        .ok_or_else(|| format!("context '{context}' has no cluster/user"))?;

    config.contexts.retain(|c| c.name == context);
    config.clusters.retain(|c| c.name == inner.cluster);
    config.auth_infos.retain(|a| a.name == inner.user);
    config.current_context = Some(context.to_string());
    if config.kind.is_none() {
        config.kind = Some("Config".to_string());
    }
    if config.api_version.is_none() {
        config.api_version = Some("v1".to_string());
    }
    serde_yaml::to_string(&config).map_err(|e| e.to_string())
}

pub(crate) async fn build_client(paths: &[PathBuf], context: &str) -> Result<Client, String> {
    let kubeconfig = load_kubeconfigs(paths)?;
    let options = KubeConfigOptions {
        context: Some(context.to_string()),
        cluster: None,
        user: None,
    };
    let config = Config::from_custom_kubeconfig(kubeconfig, &options)
        .await
        .map_err(|e| e.to_string())?;
    Client::try_from(config).map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClusterInfoIn {
    /// The kubeconfig context name to connect to.
    pub context: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ClusterInfoOut {
    pub context: String,
    pub reachable: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

/// Connect to `context` and return the apiserver git version, or an error
/// string if the connection/auth/handshake fails (or times out).
async fn connect_and_version(cache: &ClientCache, context: &str) -> Result<String, String> {
    let client = cache.get(context).await?;
    let info = tokio::time::timeout(request_timeout(), client.apiserver_version())
        .await
        .map_err(|_| "connection timed out".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(info.git_version)
}

/// Build the `k8s.clusterInfo` capability backed by a shared client cache.
pub fn cluster_info_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ClusterInfoIn, ClusterInfoOut, _, _>(
        "k8s.clusterInfo",
        "connect to a kube context and report server version and reachability",
        Annotations::READ_ONLY,
        move |input: ClusterInfoIn| {
            let cache = cache.clone();
            async move {
                Ok(match connect_and_version(&cache, &input.context).await {
                    Ok(version) => ClusterInfoOut {
                        context: input.context,
                        reachable: true,
                        version: Some(version),
                        error: None,
                    },
                    Err(error) => {
                        // A failed handshake may mean a stale cached client.
                        cache.invalidate(&input.context).await;
                        ClusterInfoOut {
                            context: input.context,
                            reachable: false,
                            version: None,
                            error: Some(error),
                        }
                    }
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_capability::Registry;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn timeout_setter_clamps_to_supported_range() {
        // Above the max is clamped down.
        assert_eq!(set_request_timeout_secs(120), MAX_TIMEOUT_SECS);
        assert_eq!(request_timeout(), Duration::from_secs(MAX_TIMEOUT_SECS));
        // Zero is clamped up to the minimum.
        assert_eq!(set_request_timeout_secs(0), MIN_TIMEOUT_SECS);
        // A value in range is applied verbatim.
        assert_eq!(set_request_timeout_secs(20), 20);
        assert_eq!(request_timeout_secs(), 20);
        // Restore the default so other tests see a known value.
        set_request_timeout_secs(DEFAULT_TIMEOUT_SECS);
    }

    #[test]
    fn single_context_kubeconfig_keeps_only_the_named_context() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "srelens-single-context-{}-{}.yaml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(
            &path,
            "apiVersion: v1\nkind: Config\ncurrent-context: ctx-a\nclusters:\n  - name: cluster-a\n    cluster: { server: https://a:6443 }\n  - name: cluster-b\n    cluster: { server: https://b:6443 }\nusers:\n  - name: user-a\n    user: {}\n  - name: user-b\n    user: {}\ncontexts:\n  - name: ctx-a\n    context: { cluster: cluster-a, user: user-a }\n  - name: ctx-b\n    context: { cluster: cluster-b, user: user-b }\n",
        )
        .unwrap();

        let yaml = single_context_kubeconfig_yaml(&[path.clone()], "ctx-b").unwrap();
        let locked = Kubeconfig::from_yaml(&yaml).unwrap();

        // Only ctx-b (and its cluster/user) survive, and it's the current context.
        assert_eq!(locked.current_context.as_deref(), Some("ctx-b"));
        assert_eq!(locked.contexts.len(), 1);
        assert_eq!(locked.contexts[0].name, "ctx-b");
        assert_eq!(locked.clusters.len(), 1);
        assert_eq!(locked.clusters[0].name, "cluster-b");
        assert_eq!(locked.auth_infos.len(), 1);
        assert_eq!(locked.auth_infos[0].name, "user-b");

        // A missing context is an error, not a full config.
        assert!(single_context_kubeconfig_yaml(&[path.clone()], "nope").is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn capability_has_expected_id_and_annotations() {
        let cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let cap = cluster_info_capability(cache);
        assert_eq!(cap.id, "k8s.clusterInfo");
        assert!(cap.annotations.read_only);
    }

    #[test]
    fn validates_pasted_kubeconfig_contexts() {
        let yaml = "apiVersion: v1\nkind: Config\nclusters:\n- name: a\n  cluster: { server: https://a }\ncontexts:\n- name: ctx-a\n  context: { cluster: a, user: user-a }\n";
        assert_eq!(validate_kubeconfig_yaml(yaml), Ok(1));
        assert!(validate_kubeconfig_yaml("apiVersion: v1\nkind: Config\ncontexts: []\n").is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_context_is_reported_as_unreachable() {
        let dir = std::env::temp_dir();
        let path = dir.join("srelens-connect-test-kubeconfig.yaml");
        tokio::fs::write(
            &path,
            "clusters:\n  - name: a\n    cluster: { server: https://127.0.0.1:1 }\ncontexts:\n  - name: ctx-a\n    context: { cluster: a }\n",
        )
        .await
        .unwrap();

        let mut reg = Registry::new();
        reg.register(cluster_info_capability(ClientCache::new(path.clone())));
        let out = reg
            .invoke("k8s.clusterInfo", json!({ "context": "does-not-exist" }))
            .await
            .unwrap();

        assert_eq!(out["reachable"], false);
        assert!(out["error"].is_string());
        let _ = tokio::fs::remove_file(&path).await;
    }
}
