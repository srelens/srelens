//! Real cluster connection via kube-rs: the `k8s.clusterInfo` capability
//! connects to a named kubeconfig context and reports the server version and
//! reachability. Authentication (client certs, tokens, exec plugins) is handled
//! by kube-rs from the kubeconfig.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kube::config::{Config, KubeConfigOptions, Kubeconfig};
use kube::Client;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability};

use crate::client_cache::ClientCache;
use crate::context_resolve::resolve_context;

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
    paths
        .iter()
        .try_fold(Kubeconfig::default(), |merged, path| {
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
    // Map the (possibly disambiguated) display name back to its owning file and
    // in-file name, so a duplicate-named context scopes to its own cluster/user.
    let resolved = resolve_context(paths, context);
    let in_config = resolved
        .as_ref()
        .map(|target| target.original_name.clone())
        .unwrap_or_else(|| context.to_string());

    // Prefer the owning file (correct for duplicate names); fall back to the
    // merged view when the context is a single config split across files.
    if let Some(target) = &resolved {
        if let Ok(config) = Kubeconfig::read_from(&target.source) {
            if let Ok(yaml) = standalone_context_yaml(config, &in_config) {
                return Ok(yaml);
            }
        }
    }
    standalone_context_yaml(load_kubeconfigs(paths)?, &in_config)
}

/// Build a standalone kubeconfig YAML keeping only `context` plus the one
/// cluster and user it references, with `current-context` pinned to it.
fn standalone_context_yaml(mut config: Kubeconfig, context: &str) -> Result<String, String> {
    let named = config
        .contexts
        .iter()
        .find(|c| c.name == context)
        .ok_or_else(|| format!("context '{context}' not found in kubeconfig"))?;
    let inner = named
        .context
        .clone()
        .ok_or_else(|| format!("context '{context}' has no cluster/user"))?;
    // A split config might not carry this context's cluster/user in this file.
    if !config.clusters.iter().any(|c| c.name == inner.cluster)
        || !config.auth_infos.iter().any(|a| a.name == inner.user)
    {
        return Err(format!("context '{context}' is missing its cluster or user here"));
    }

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

static SCOPED_KUBECONFIG_SEQ: AtomicU64 = AtomicU64::new(1);

/// Write a standalone kubeconfig containing only `context` to a private temp
/// file (atomic create, mode 0600) and return its path. Callers point
/// `KUBECONFIG` at it to scope a child process (e.g. `helm`) to one cluster,
/// then delete it when done.
pub fn write_single_context_kubeconfig(
    paths: &[PathBuf],
    context: &str,
) -> Result<PathBuf, String> {
    let yaml = single_context_kubeconfig_yaml(paths, context)?;
    let id = SCOPED_KUBECONFIG_SEQ.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "srelens-helm-{}-{nanos}-{}.kubeconfig",
        std::process::id(),
        id
    ));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&path).map_err(|e| e.to_string())?;
    use std::io::Write as _;
    file.write_all(yaml.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Resolve a kube `Config` for `context`, preferring the file that declares it
/// (so merge "first-file-wins" can't shadow its cluster/user), else the merge.
///
/// `context` is the disambiguated display name from [`resolve_contexts`]; we map
/// it back to the exact file + in-file name that owns it, so duplicate-named
/// contexts across merged files each connect to their own cluster and user
/// instead of always resolving to the first.
pub(crate) async fn config_for_context(paths: &[PathBuf], context: &str) -> Result<Config, String> {
    let resolved = resolve_context(paths, context);
    // The name kube-rs must find inside the kubeconfig (display names may be
    // prefixed for disambiguation; the file itself still uses the raw name).
    let in_config = resolved
        .as_ref()
        .map(|target| target.original_name.clone())
        .unwrap_or_else(|| context.to_string());

    // Prefer the specific file that owns this context: kube-rs merge is "first
    // file wins" by name, so a same-named cluster/user in another merged file
    // can shadow (or drop) this context's own entries.
    if let Some(target) = &resolved {
        if let Ok(kc) = Kubeconfig::read_from(&target.source) {
            let options = KubeConfigOptions {
                context: Some(target.original_name.clone()),
                cluster: None,
                user: None,
            };
            if let Ok(config) = Config::from_custom_kubeconfig(kc, &options).await {
                return Ok(config);
            }
        }
    }
    // Fallback: merged resolution (handles a single config split across files).
    let kubeconfig = load_kubeconfigs(paths)?;
    let options = KubeConfigOptions {
        context: Some(in_config),
        cluster: None,
        user: None,
    };
    Config::from_custom_kubeconfig(kubeconfig, &options)
        .await
        .map_err(|e| e.to_string())
}

pub(crate) async fn build_client(paths: &[PathBuf], context: &str) -> Result<Client, String> {
    let config = config_for_context(paths, context).await?;
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
    use serde_json::json;
    use srelens_capability::Registry;
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
    fn writes_scoped_kubeconfig_file_0600_single_context() {
        use std::io::Write as _;
        // Two-context kubeconfig fixture.
        let dir = std::env::temp_dir().join(format!("srelens-cfgtest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config");
        let mut f = std::fs::File::create(&cfg).unwrap();
        f.write_all(
            b"apiVersion: v1\nkind: Config\ncurrent-context: a\nclusters:\n- name: ca\n  cluster: {server: https://a}\n- name: cb\n  cluster: {server: https://b}\ncontexts:\n- name: a\n  context: {cluster: ca, user: ua}\n- name: b\n  context: {cluster: cb, user: ub}\nusers:\n- name: ua\n  user: {}\n- name: ub\n  user: {}\n",
        )
        .unwrap();

        let out = write_single_context_kubeconfig(&[cfg], "b").unwrap();
        let written = std::fs::read_to_string(&out).unwrap();
        assert!(written.contains("name: b"));
        assert!(!written.contains("name: a\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&out).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_file(&out);
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

    /// Write `contents` to a unique temp file (pid + nanos) and return its path.
    fn write_temp_kubeconfig(tag: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "srelens-config-for-context-{tag}-{}-{}.yaml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn config_for_context_prefers_the_owning_files_cluster() {
        // fileA and fileB both declare a cluster named `shared`, but with
        // different servers. kube-rs merge is "first file wins" by name, so a
        // merge-only resolution of ctx-b would wrongly pick fileA's server.
        let file_a = write_temp_kubeconfig(
            "a",
            "apiVersion: v1\nkind: Config\ncurrent-context: ctx-a\nclusters:\n  - name: shared\n    cluster: { server: https://a.example:6443 }\nusers:\n  - name: ua\n    user: {}\ncontexts:\n  - name: ctx-a\n    context: { cluster: shared, user: ua }\n",
        );
        let file_b = write_temp_kubeconfig(
            "b",
            "apiVersion: v1\nkind: Config\nclusters:\n  - name: shared\n    cluster: { server: https://b.example:6443 }\nusers:\n  - name: ub\n    user: {}\ncontexts:\n  - name: ctx-b\n    context: { cluster: shared, user: ub }\n",
        );

        let config = config_for_context(&[file_a.clone(), file_b.clone()], "ctx-b")
            .await
            .unwrap();

        // ctx-b must resolve to fileB's own `shared` cluster, not fileA's.
        let url = config.cluster_url.to_string();
        assert!(
            url.starts_with("https://b.example"),
            "expected fileB's server, got {url}"
        );

        let _ = std::fs::remove_file(&file_a);
        let _ = std::fs::remove_file(&file_b);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn config_for_context_connects_duplicate_named_contexts_to_their_own_file() {
        // Both files name their context `default` but point at different
        // servers. kube-rs merge keeps only the first; disambiguation must let
        // each connect to its own cluster via its (distinct) display name.
        let file_a = write_temp_kubeconfig(
            "dupA",
            "apiVersion: v1\nkind: Config\nclusters:\n  - name: c\n    cluster: { server: https://a.example:6443 }\nusers:\n  - name: u\n    user: {}\ncontexts:\n  - name: default\n    context: { cluster: c, user: u }\n",
        );
        let file_b = write_temp_kubeconfig(
            "dupB",
            "apiVersion: v1\nkind: Config\nclusters:\n  - name: c\n    cluster: { server: https://b.example:6443 }\nusers:\n  - name: u\n    user: {}\ncontexts:\n  - name: default\n    context: { cluster: c, user: u }\n",
        );

        let paths = vec![file_a.clone(), file_b.clone()];
        let resolved = crate::context_resolve::resolve_contexts(&paths);
        assert_eq!(resolved.len(), 2, "both duplicate-named contexts must be visible");
        let a_display = resolved.iter().find(|c| c.source == file_a).unwrap().display_name.clone();
        let b_display = resolved.iter().find(|c| c.source == file_b).unwrap().display_name.clone();
        assert_ne!(a_display, b_display, "disambiguated names must differ");

        // Connecting by fileB's display name must reach fileB's server, not the
        // first-merged fileA.
        let config = config_for_context(&paths, &b_display).await.unwrap();
        assert!(
            config.cluster_url.to_string().starts_with("https://b.example"),
            "expected fileB's server, got {}",
            config.cluster_url
        );

        let _ = std::fs::remove_file(&file_a);
        let _ = std::fs::remove_file(&file_b);
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
