//! The single place the app registers every backend capability.
//!
//! Each capability is exposed automatically through BOTH the Tauri command
//! bridge (for the WebView) and the MCP server (for external clients). The
//! `every_capability_is_mcp_exposed` test enforces that guarantee.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use srelens_capability::{Capability, Registry};
use srelens_kube::client_cache::ClientCache;

mod catalog;
pub use catalog::{catalog_of, CatalogEntry};

/// Sorted id + annotation-flag projection of the live registry, emitted to a
/// committed JSON so the frontend palette audit can cross-check it without
/// linking Rust. See `capability_catalog_json_is_in_sync`.
pub fn capability_catalog() -> Vec<CatalogEntry> {
    catalog_of(&build_registry())
}

/// Resolve kubeconfig paths: every `$KUBECONFIG` entry, else `$HOME/.kube/config`.
pub fn default_kubeconfig_paths() -> Vec<PathBuf> {
    if let Some(value) = std::env::var_os("KUBECONFIG") {
        let paths = std::env::split_paths(&value)
            .filter(|path| !path.as_os_str().is_empty())
            .collect::<Vec<_>>();
        if !paths.is_empty() {
            return paths;
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    vec![PathBuf::from(home).join(".kube").join("config")]
}

#[cfg(test)]
pub fn default_kubeconfig_path() -> PathBuf {
    default_kubeconfig_paths()
        .into_iter()
        .next()
        .unwrap_or_default()
}

/// Blocking HTTP GET for Toolbox tool downloads. Called only from inside
/// `spawn_blocking` (the install capabilities), so blocking here is fine. A
/// non-2xx or transport error maps to the retryable `Download` variant.
fn http_get(url: &str) -> Result<Vec<u8>, srelens_kube::toolbox_install::InstallError> {
    use srelens_kube::toolbox_install::InstallError;
    let resp = reqwest::blocking::Client::builder()
        .user_agent(concat!("srelens/", env!("CARGO_PKG_VERSION")))
        .build()
        .and_then(|client| client.get(url).send())
        .map_err(|e| InstallError::Download(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(InstallError::Download(format!("{} for {url}", resp.status())));
    }
    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| InstallError::Download(e.to_string()))
}

/// Run a managed tool with args, mapping a non-zero exit (with stderr) to a
/// retryable error. Used for krew's self-bootstrap; called inside spawn_blocking.
///
/// Public (not `pub(crate)`) because the desktop crate's streaming toolbox
/// install (`toolbox.rs`) reuses this same helper via the re-export in
/// `apps/desktop/src-tauri/src/capabilities.rs`.
pub fn run_tool(
    bin: &std::path::Path,
    args: &[&str],
) -> Result<(), srelens_kube::toolbox_install::InstallError> {
    use srelens_kube::toolbox_install::InstallError;
    let output = std::process::Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| InstallError::Download(e.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(InstallError::Download(String::from_utf8_lossy(&output.stderr).into_owned()))
    }
}

/// Run `kubectl-krew` with args, returning stdout (or stderr as an error).
/// Prefers the krew shim under `~/.krew/bin`, falling back to PATH. Called
/// inside spawn_blocking by the plugin capabilities.
fn run_krew(args: &[&str]) -> Result<String, srelens_kube::toolbox_install::InstallError> {
    use srelens_kube::toolbox_install::InstallError;
    let shim = srelens_kube::toolbox::krew_bin_dir().join("kubectl-krew");
    let bin = if shim.is_file() { shim } else { std::path::PathBuf::from("kubectl-krew") };
    let output = std::process::Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| InstallError::Download(e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(InstallError::Download(String::from_utf8_lossy(&output.stderr).into_owned()))
    }
}

/// Probe a managed tool's version by running it and scanning for a semver.
/// Each tool prints its version differently, so we pass tool-specific flags and
/// let `first_semver` pull the `vX.Y.Z` out of whatever text comes back.
fn tool_version(name: &str, path: &std::path::Path) -> Option<String> {
    let args: &[&str] = match name {
        "kubectl" => &["version", "--client", "-o", "json"],
        "helm" => &["version", "--short"],
        _ => &["version"], // krew and any future tool
    };
    let output = std::process::Command::new(path).args(args).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    srelens_kube::toolbox::first_semver(&text)
}

/// Build the registry with a freshly-created client cache. Used by the MCP
/// stdio binary, which doesn't need to share the cache with watch tasks.
pub fn build_registry() -> Registry {
    build_registry_with(ClientCache::new_many(default_kubeconfig_paths()))
}

/// Build the registry using a caller-provided client cache AND kubeconfig
/// paths. The web server uses this with per-user paths; the desktop/MCP
/// surfaces delegate with the host defaults.
pub fn build_registry_with_paths(
    cache: Arc<ClientCache>,
    kubeconfig_paths: Vec<PathBuf>,
) -> Registry {
    let mut reg = Registry::new();

    reg.register(Capability::read_only(
        "ping",
        "health check; echoes the input back as { pong: <input> }",
        |input| async move { Ok(json!({ "pong": input })) },
    ));

    reg.register(srelens_kube::contexts::list_contexts_capability(
        cache.clone(),
        kubeconfig_paths.clone(),
    ));
    reg.register(srelens_kube::contexts::delete_context_capability(
        cache.clone(),
    ));

    reg.register(srelens_kube::toolbox::diagnose_context_capability(
        kubeconfig_paths.clone(),
        srelens_kube::toolbox::SearchPaths::from_env(),
        |path| path.is_file(),
    ));
    reg.register(srelens_kube::toolbox::install_kubectl_capability(
        srelens_kube::toolbox::srelens_bin_dir(),
        http_get,
    ));
    reg.register(srelens_kube::toolbox::install_helm_capability(
        srelens_kube::toolbox::srelens_bin_dir(),
        http_get,
    ));
    reg.register(srelens_kube::toolbox::install_krew_capability(
        std::env::temp_dir(),
        http_get,
        run_tool,
    ));
    reg.register(srelens_kube::toolbox::status_capability(
        srelens_kube::toolbox::SearchPaths::from_env(),
        vec![
            srelens_kube::toolbox::srelens_bin_dir(),
            srelens_kube::toolbox::krew_bin_dir(),
        ],
        |path| path.is_file(),
        tool_version,
    ));
    reg.register(srelens_kube::toolbox::search_plugins_capability(run_krew));
    reg.register(srelens_kube::toolbox::install_plugin_capability(run_krew));
    reg.register(srelens_kube::toolbox::upgrade_plugin_capability(run_krew));
    reg.register(srelens_kube::toolbox::remove_plugin_capability(run_krew));

    reg.register(srelens_kube::connect::cluster_info_capability(
        cache.clone(),
    ));
    // Add-cluster form support (desktop + web): synthesize a kubeconfig from
    // form fields, and probe a kubeconfig's reachability before saving it.
    reg.register(srelens_kube::cluster_synth::synthesize_cluster_capability());
    reg.register(srelens_kube::connect::test_cluster_connection_capability());
    reg.register(srelens_kube::workloads::list_namespaces_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::workloads::list_pods_capability(cache.clone()));
    reg.register(srelens_kube::workloads::pods_for_selector_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::logs::pod_logs_capability(cache.clone()));
    reg.register(srelens_kube::deployments::list_deployments_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::deployments::list_replicasets_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::statefulsets::list_statefulsets_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::daemonsets::list_daemonsets_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::jobs::list_jobs_capability(cache.clone()));
    reg.register(srelens_kube::cronjobs::list_cronjobs_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::cronjobs::cronjob_set_suspend_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::cronjobs::cronjob_trigger_now_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::configmaps::list_configmaps_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::secrets::list_secrets_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::resourcequotas::list_resourcequotas_capability(cache.clone()));
    reg.register(srelens_kube::limitranges::list_limitranges_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::services::list_services_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::ingresses::list_ingresses_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::endpointslices::list_endpointslices_capability(cache.clone()));
    reg.register(srelens_kube::networkpolicies::list_networkpolicies_capability(cache.clone()));
    reg.register(srelens_kube::pvcs::list_pvcs_capability(cache.clone()));
    reg.register(srelens_kube::pvcs::pods_for_pvc_capability(cache.clone()));
    reg.register(srelens_kube::persistentvolumes::list_pvs_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::storageclasses::list_storageclasses_capability(cache.clone()));
    reg.register(srelens_kube::serviceaccounts::list_serviceaccounts_capability(cache.clone()));
    reg.register(srelens_kube::serviceaccounts::pods_for_service_account_capability(cache.clone()));
    reg.register(
        srelens_kube::serviceaccounts::bindings_for_service_account_capability(cache.clone()),
    );
    reg.register(srelens_kube::roles::list_roles_capability(cache.clone()));
    reg.register(srelens_kube::roles::list_clusterroles_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::rolebindings::list_rolebindings_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::rolebindings::list_clusterrolebindings_capability(cache.clone()));
    reg.register(srelens_kube::actions::delete_pod_capability(cache.clone()));
    reg.register(srelens_kube::actions::evict_pod_capability(cache.clone()));
    reg.register(srelens_kube::actions::delete_resource_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::actions::scale_capability(cache.clone()));
    reg.register(srelens_kube::actions::rollout_restart_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::actions::update_config_data_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::actions::cordon_node_capability(cache.clone()));
    reg.register(srelens_kube::actions::drain_node_capability(cache.clone()));
    reg.register(srelens_kube::debug::debug_pod_capability(cache.clone()));
    reg.register(srelens_kube::debug::node_debug_pod_capability(cache.clone()));
    reg.register(srelens_kube::events::list_events_capability(cache.clone()));
    reg.register(srelens_kube::metrics::node_metrics_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::metrics::pod_metrics_capability(cache.clone()));
    reg.register(srelens_kube::nodes::list_nodes_capability(cache.clone()));
    reg.register(srelens_kube::manifest::get_manifest_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::manifest::get_object_capability(cache.clone()));
    reg.register(srelens_kube::secrets::get_secret_capability(cache.clone()));
    reg.register(srelens_kube::manifest::apply_manifest_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::manifest::validate_manifest_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::manifest::diff_manifest_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::access::can_i_capability(cache.clone()));
    reg.register(srelens_kube::schema::open_api_schema_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::crds::list_crds_capability(cache.clone()));
    reg.register(srelens_kube::crds::list_custom_resource_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::helm::list_helm_releases_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::helm::get_helm_release_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::helm_cli::helm_version_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::helm_cli::helm_template_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::helm_cli::helm_install_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::helm_cli::helm_upgrade_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::helm_cli::helm_rollback_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::helm_cli::helm_uninstall_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::helm_cli::helm_repo_add_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::helm_cli::helm_repo_update_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::helm_cli::helm_search_repo_capability(
        cache.clone(),
    ));
    reg.register(srelens_kube::manifest::list_resource_capability(cache));

    reg
}

/// Build the registry using a caller-provided client cache with the host's
/// default kubeconfig discovery (desktop + MCP behavior, unchanged).
pub fn build_registry_with(cache: Arc<ClientCache>) -> Registry {
    build_registry_with_paths(cache, default_kubeconfig_paths())
}

/// The real resource kind resolver, over `srelens_kube::manifest::gvk_for`.
///
/// `Secret` is excluded deliberately: it is the one read-only kind that is
/// consent-gated (`k8s.getSecret` is `SENSITIVE_READ`), and resource reads must
/// never trip the consent gate — clients auto-fetch resources to populate
/// context, so a confirm dialog there would be a consent-fatigue vector.
pub fn kind_resolver() -> std::sync::Arc<dyn srelens_mcp::resources::KindResolver> {
    struct GvkKinds;
    impl srelens_mcp::resources::KindResolver for GvkKinds {
        fn scope(&self, kind: &str) -> Option<srelens_mcp::resources::KindScope> {
            use srelens_mcp::resources::KindScope;
            if kind == "Secret" {
                return None;
            }
            srelens_kube::manifest::gvk_for(kind).map(|(_, namespaced)| {
                if namespaced { KindScope::Namespaced } else { KindScope::ClusterScoped }
            })
        }
    }
    std::sync::Arc::new(GvkKinds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_mcp::completeness::assert_every_capability_has_a_tool;
    use srelens_mcp::McpServer;
    use std::sync::Arc;

    #[test]
    fn every_capability_is_mcp_exposed() {
        let reg = build_registry();
        let server = McpServer::new(Arc::new(reg.clone()));
        assert_eq!(assert_every_capability_has_a_tool(&reg, &server), Ok(()));
        srelens_mcp::completeness::assert_mutating_capabilities_are_gated(&reg);
    }

    #[test]
    fn registers_core_capabilities() {
        let reg = build_registry();
        let mut ids = reg.ids();
        ids.sort();
        assert!(ids.contains(&"ping"));
        assert!(ids.contains(&"k8s.listContexts"));
        assert!(ids.contains(&"k8s.clusterInfo"));
    }

    #[tokio::test]
    async fn ping_echoes_input() {
        let reg = build_registry();
        let out = reg.invoke("ping", json!("hello")).await.unwrap();
        assert_eq!(out, json!({ "pong": "hello" }));
    }

    #[test]
    fn kubeconfig_path_prefers_env() {
        // Default falls back to a path under HOME when KUBECONFIG is unset.
        let path = default_kubeconfig_path();
        assert!(
            path.to_string_lossy().contains(".kube/config") || std::env::var("KUBECONFIG").is_ok()
        );
    }

    #[test]
    fn with_paths_builds_same_capability_set() {
        let cache = ClientCache::new_many(vec![]);
        let reg = build_registry_with_paths(cache, vec![std::path::PathBuf::from("/nonexistent")]);
        let mut ids = reg.ids();
        ids.sort();
        let default_reg = build_registry();
        let mut default_ids = default_reg.ids();
        default_ids.sort();
        assert_eq!(ids, default_ids, "same capabilities regardless of paths");
    }

    #[test]
    fn the_real_kind_resolver_covers_namespaced_and_cluster_scoped_kinds() {
        use srelens_mcp::resources::KindScope;
        let r = crate::kind_resolver();
        assert_eq!(r.scope("Pod"), Some(KindScope::Namespaced));
        assert_eq!(r.scope("Deployment"), Some(KindScope::Namespaced));
        assert_eq!(r.scope("Node"), Some(KindScope::ClusterScoped));
        assert_eq!(r.scope("PersistentVolume"), Some(KindScope::ClusterScoped));
        assert_eq!(r.scope("Nonsense"), None);
    }

    /// The curation that makes "the consent gate never fires on a resource
    /// read" true: Secrets are reachable only through the gated
    /// `k8s.getSecret` tool, never as an addressable resource.
    #[test]
    fn the_real_kind_resolver_excludes_secrets() {
        assert_eq!(crate::kind_resolver().scope("Secret"), None);
    }

    #[test]
    fn capability_catalog_json_is_in_sync() {
        // The committed JSON is the bridge to the frontend palette audit. It MUST
        // equal the live registry; regenerate with `UPDATE_CATALOG=1 cargo test`.
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/desktop/src/lib/capability-catalog.json");
        let want = serde_json::to_string_pretty(&capability_catalog()).unwrap() + "\n";
        if std::env::var("UPDATE_CATALOG").is_ok() {
            std::fs::write(path, &want).unwrap();
            return;
        }
        let got = std::fs::read_to_string(path).unwrap_or_default();
        assert_eq!(got, want, "capability-catalog.json is stale — run UPDATE_CATALOG=1 cargo test -p srelens-registry");
    }
}
