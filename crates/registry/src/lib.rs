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
mod settings;
pub use settings::default_settings_path;

// Test-only: every consumer of this module — `render_catalog` (regenerated via
// `UPDATE_CATALOG=1 cargo test`), the doc-scan tests below, and mcp_docs.rs's
// own unit tests — runs under `#[cfg(test)]`. Nothing in a normal build calls
// it, so `pub` would be the only thing keeping it from looking dead; gating
// the whole module here is more honest than papering over that with `pub`.
#[cfg(test)]
pub(crate) mod mcp_docs;

/// Sorted id + annotation-flag projection of the live registry, emitted to a
/// committed JSON so the frontend palette audit can cross-check it without
/// linking Rust. See `capability_catalog_json_is_in_sync`.
pub fn capability_catalog() -> Vec<CatalogEntry> {
    catalog_of(&build_registry())
}

pub use srelens_kube::connect::{default_kubeconfig_dir, managed_kubeconfig_files};

/// The STATIC kubeconfig sources: every `$KUBECONFIG` entry, else
/// `$HOME/.kube/config`.
///
/// Deliberately excludes the app's managed folder. That folder's contents
/// change while the app runs, and this result gets captured by long-lived
/// consumers — the capability registry snapshots it once at build time — so
/// folding volatile content in here means a file deleted at runtime is
/// reintroduced on every later call and never goes away. Callers that want the
/// managed folder as well should use [`all_kubeconfig_paths`], which resolves
/// it at call time.
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

/// Every kubeconfig source as of RIGHT NOW: the static ones plus whatever is
/// currently in the app's managed folder.
///
/// For callers that resolve paths per operation (a terminal, a helm command,
/// the initial client cache). Anything that captures its result for later
/// reuse wants [`default_kubeconfig_paths`] plus a live folder read instead.
pub fn all_kubeconfig_paths() -> Vec<PathBuf> {
    let mut paths = default_kubeconfig_paths();
    for managed in srelens_kube::connect::managed_kubeconfig_files() {
        if !paths.contains(&managed) {
            paths.push(managed);
        }
    }
    paths
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
    use srelens_kube::toolbox_install::{ambient_github_token, github_api_wants_auth, InstallError};
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("srelens/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| InstallError::Download(e.to_string()))?;
    let mut req = client.get(url);
    // Same rule as the desktop's progress fetch, from the same predicate:
    // api.github.com lookups authenticate with an ambient GITHUB_TOKEN so CI
    // (whose anonymous pool 403s intermittently) stays reliable; the token
    // never rides to other hosts.
    if github_api_wants_auth(url) {
        if let Some(token) = ambient_github_token() {
            req = req.bearer_auth(token);
        }
    }
    let resp = req
        .send()
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
    build_registry_with_paths_and_settings(cache, kubeconfig_paths, None)
}

/// Build a registry and optionally add the durable desktop settings surface.
/// Web-server registries omit it because web settings are per-user SQLite
/// rows; desktop GUI and MCP callers pass the stable desktop settings path.
pub fn build_registry_with_paths_and_settings(
    cache: Arc<ClientCache>,
    kubeconfig_paths: Vec<PathBuf>,
    settings_path: Option<PathBuf>,
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
        srelens_kube::connect::default_kubeconfig_dir(),
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

    if let Some(path) = settings_path {
        settings::register(&mut reg, path);
    }

    reg
}

/// Build the registry using a caller-provided client cache with the host's
/// default kubeconfig discovery (desktop + MCP behavior, unchanged).
pub fn build_registry_with(cache: Arc<ClientCache>) -> Registry {
    build_registry_with_paths_and_settings(
        cache,
        default_kubeconfig_paths(),
        default_settings_path(),
    )
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
    fn with_paths_and_settings_builds_same_capability_set() {
        let cache = ClientCache::new_many(vec![]);
        let reg = build_registry_with_paths_and_settings(
            cache,
            vec![std::path::PathBuf::from("/nonexistent")],
            default_settings_path(),
        );
        let mut ids = reg.ids();
        ids.sort();
        let default_reg = build_registry();
        let mut default_ids = default_reg.ids();
        default_ids.sort();
        assert_eq!(ids, default_ids, "same capabilities regardless of paths");
    }

    #[test]
    fn web_registry_omits_host_desktop_settings() {
        let cache = ClientCache::new_many(vec![]);
        let reg = build_registry_with_paths(cache, vec![]);
        assert!(!reg.ids().contains(&"settings.get"));
        assert!(!reg.ids().contains(&"settings.set"));
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
        // Lives with the service layer it describes (@srelens/core), which is
        // what the frontend palette audit reads.
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../packages/core/src/lib/capability-catalog.json");
        let want = serde_json::to_string_pretty(&capability_catalog()).unwrap() + "\n";
        if std::env::var("UPDATE_CATALOG").is_ok() {
            std::fs::write(path, &want).unwrap();
            return;
        }
        let got = std::fs::read_to_string(path).unwrap_or_default();
        assert_eq!(got, want, "capability-catalog.json is stale — run UPDATE_CATALOG=1 cargo test -p srelens-registry");
    }

    /// The published catalog MUST equal what the live registry, prompt library
    /// and resource templates render. Same convention as
    /// `capability_catalog_json_is_in_sync` above — one regeneration knob.
    #[test]
    fn mcp_catalog_md_is_in_sync() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/mcp-catalog.md");
        let want = crate::mcp_docs::render_catalog();
        if std::env::var("UPDATE_CATALOG").is_ok() {
            std::fs::write(path, &want).unwrap();
            return;
        }
        let got = std::fs::read_to_string(path).unwrap_or_default();
        assert_eq!(
            got, want,
            "docs/mcp-catalog.md is stale — run `UPDATE_CATALOG=1 cargo test -p srelens-registry`"
        );
    }

    fn doc(name: &str) -> String {
        let path = format!("{}/../../docs/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    /// A renamed tool must not leave a plausible-looking dead example behind.
    /// Scans MCP.md for capability-shaped identifiers and asserts each is real.
    #[test]
    fn every_tool_identifier_in_mcp_md_is_registered() {
        let md = doc("MCP.md");
        let reg = build_registry();
        let ids: std::collections::BTreeSet<&str> = reg.ids().into_iter().collect();

        let mut checked = 0usize;
        // Identifiers appear in backticks, e.g. `k8s.listPods`.
        for token in md.split('`').skip(1).step_by(2) {
            if !(token.starts_with("k8s.") || token.starts_with("toolbox.")) {
                continue;
            }
            // Skip URI-ish and prose-ish tokens that merely start with the prefix.
            if token.contains(' ') || token.contains('/') || token.contains('{') {
                continue;
            }
            checked += 1;
            assert!(
                ids.contains(token),
                "docs/MCP.md names `{token}`, which is not a registered capability"
            );
        }
        assert!(checked >= 5, "expected the examples to name several real tools, saw {checked}");
    }

    /// Flags the docs mention *in order to say they do not exist*. Keeping this
    /// list explicit is what lets the guard below stay strict without forcing a
    /// true statement out of the documentation.
    ///
    /// `--mcp-token` is documented as deliberately absent: a token in argv is
    /// visible to every account on the machine via `ps`, so it comes from
    /// `SRELENS_MCP_TOKEN` instead. Deleting that sentence to satisfy a test
    /// would make the docs less accurate, not more.
    const DOCUMENTED_AS_ABSENT: [&str; 2] = ["--mcp-token", "--version"];

    /// A renamed or removed CLI flag must fail here rather than silently
    /// breaking the documented setup path. Also scans for backticked
    /// `SRELENS_*` environment variable names, so a doc naming e.g.
    /// `SRELENS_MCP_BEARER` for a variable main.rs never reads would fail
    /// here too, not just the `TOKEN_ENV` constant the generated catalog
    /// cross-checks.
    /// Every `.rs` file under `crates/` and the desktop app, concatenated.
    ///
    /// Environment variables are read wherever they are needed — the desktop
    /// binary, `crates/server`, capability crates — so a documented variable
    /// has to be looked for across the workspace rather than in one file.
    fn workspace_rust_sources() -> String {
        fn walk(dir: &std::path::Path, out: &mut String) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n == "target") {
                        continue;
                    }
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    // Skip the catalog renderer. It defines the documented
                    // names (`TOKEN_ENV = "SRELENS_MCP_TOKEN"`) in order to
                    // *print* them, so finding a literal there is not evidence
                    // that anything reads the variable — it would let this
                    // guard pass even after the variable was renamed out of
                    // production. The guard has to look at code that uses the
                    // name, not code that documents it.
                    if path.file_name().is_some_and(|n| n == "mcp_docs.rs") {
                        continue;
                    }
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        // Comment lines are dropped. A name mentioned in prose
                        // — including the comment just above, and this test's
                        // own doc comment — is not evidence that any code reads
                        // it. Leaving them in let this guard be satisfied by its
                        // own explanation of itself.
                        for line in text.lines() {
                            let trimmed = line.trim_start();
                            if trimmed.starts_with("//") {
                                continue;
                            }
                            out.push_str(line);
                            out.push('\n');
                        }
                    }
                }
            }
        }
        let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let mut out = String::new();
        walk(&root.join("crates"), &mut out);
        walk(&root.join("apps/desktop/src-tauri/src"), &mut out);
        assert!(!out.is_empty(), "found no Rust sources to scan");
        out
    }

    #[test]
    fn every_flag_named_in_the_docs_is_real_or_explicitly_absent() {
        let main_rs = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../apps/desktop/src-tauri/src/main.rs"
        ))
        .expect("main.rs is readable");

        // Every file that names a flag, not just the new ones — a stale flag is
        // just as broken in INSTALL.md as in MCP.md.
        for name in ["MCP.md", "mcp-catalog.md", "USAGE.md", "INSTALL.md", "DEVELOPMENT.md"] {
            let md = doc(name);

            // Any flag the docs show being passed to the binary — `srelens
            // --foo` — not just `--mcp*` ones. A `srelens --version` suggestion
            // slipped in here once: there is no version flag, and an
            // unrecognised argument falls through and launches the GUI, so the
            // suggested verification command would hang a terminal. Scoped to
            // the `srelens ` prefix so cargo/kubectl flags in the same docs are
            // not swept in.
            for occurrence in md.split("srelens --").skip(1) {
                let flag: String = std::iter::once('-')
                    .chain(std::iter::once('-'))
                    .chain(occurrence.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-'))
                    .collect();
                if DOCUMENTED_AS_ABSENT.contains(&flag.as_str()) {
                    assert!(
                        !main_rs.contains(&format!("\"{flag}\"")),
                        "{flag} is on DOCUMENTED_AS_ABSENT but the CLI now accepts it — \
                         the docs saying it does not exist are now wrong"
                    );
                    continue;
                }
                assert!(
                    main_rs.contains(&format!("\"{flag}\"")),
                    "docs/{name} shows `srelens {flag}`, which the CLI does not accept"
                );
            }
            for token in md.split('`').skip(1).step_by(2) {
                if token.starts_with("--mcp") {
                    // `--mcp-allow-*` is prose shorthand for "whichever of the
                    // family applies", not a claim that a flag literally named
                    // with a `*` exists — a glob is self-evidently not a literal
                    // flag name, so it doesn't constrain how the docs are written.
                    if token.contains('*') {
                        continue;
                    }
                    // `--mcp-http 127.0.0.1:8765` — compare the flag, not its argument.
                    let flag = token.split_whitespace().next().unwrap_or(token);
                    if DOCUMENTED_AS_ABSENT.contains(&flag) {
                        assert!(
                            !main_rs.contains(&format!("\"{flag}\"")),
                            "{flag} is on DOCUMENTED_AS_ABSENT but the CLI now accepts it — \
                             the docs saying it does not exist are now wrong"
                        );
                        continue;
                    }
                    assert!(
                        main_rs.contains(&format!("\"{flag}\"")),
                        "docs/{name} names {flag}, which the CLI does not accept"
                    );
                } else if token.starts_with("SRELENS_") {
                    // A bare env var name (`SRELENS_MCP_TOKEN`) or one shown with
                    // a shell assignment (`SRELENS_MCP_TOKEN=...`) — compare just
                    // the name.
                    let var = token
                        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .next()
                        .unwrap_or(token);
                    // Searched across the whole workspace, NOT just the desktop
                    // `main.rs` the flag check uses. CLI flags are all parsed in
                    // one place, but environment variables are not: web-mode
                    // documents `SRELENS_MASTER_KEY` and `SRELENS_DEV_LOGIN`,
                    // which `crates/server` reads and `main.rs` never mentions.
                    // Scoping this to `main.rs` made the test fail on accurate
                    // prose — which is the test being wrong, not the docs.
                    assert!(
                        workspace_rust_sources().contains(&format!("\"{var}\"")),
                        "docs/{name} names environment variable {var}, which no Rust source reads"
                    );
                }
            }
        }
    }

    #[test]
    fn every_json_block_in_the_docs_parses() {
        for name in ["MCP.md", "mcp-catalog.md"] {
            let md = doc(name);
            for block in crate::mcp_docs::tests_support::json_blocks(&md) {
                serde_json::from_str::<serde_json::Value>(&block).unwrap_or_else(|e| {
                    panic!("docs/{name} has a json block that does not parse: {e}\n{block}")
                });
            }
        }
    }
}
