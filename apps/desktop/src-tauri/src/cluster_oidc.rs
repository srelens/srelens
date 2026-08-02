//! Desktop-side managed cluster OIDC. Reuses the server's OIDC machinery (token
//! provider, registry, resolver, flow) but with a LOCAL sqlite token store
//! under the app config dir and a loopback browser-login flow (see
//! commands in `cluster_oidc_cmd.rs` / lib.rs). Managed sign-in applies only to
//! clusters added through the srelens Add-cluster form (their synthesized
//! kubeconfig carries srelens's managed-OIDC marker); every pre-existing
//! kubeconfig context — including one with its own kubelogin/aws/gke exec
//! plugin — keeps running that plugin natively.

use std::path::Path;
use std::sync::Arc;

use srelens_kube::client_cache::ClientCache;
use srelens_server::cluster_auth_resolver::ClusterAuthResolver;
use srelens_server::cluster_oidc::make_refresh_fn;
use srelens_server::cluster_registry::ClusterOidcRegistry;
use srelens_server::crypto::MasterKey;
use srelens_server::db::Db;
use srelens_server::oidc_provider::OidcTokenProvider;

/// Read every kubeconfig file's YAML (best-effort; unreadable files skipped).
pub fn read_kubeconfig_yamls(paths: &[std::path::PathBuf]) -> Vec<String> {
    paths
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect()
}

pub struct DesktopClusterOidc {
    pub db: Db,
    pub master_key: Arc<MasterKey>,
    pub user_id: i64,
    /// Interior-mutable so a runtime kubeconfig change (Add-cluster form, file
    /// watcher) can rebuild the registry without restarting the app. Readers
    /// go through `registry()`, which clones the current `Arc` under the lock
    /// and releases it immediately.
    registry: std::sync::Mutex<Arc<ClusterOidcRegistry>>,
    /// The public base URL used to build the loopback redirect (host+port set
    /// per sign-in; see the command). Kept here so provider refresh uses the
    /// same shape.
    pub redirect_base: String,
}

impl DesktopClusterOidc {
    /// Build the desktop OIDC environment: open/seal a local token db, upsert a
    /// single local user, and index the user's OIDC clusters.
    pub async fn build(config_dir: &Path, kubeconfig_yamls: &[String]) -> Result<Self, String> {
        std::fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
        let db = Db::open(&config_dir.join("cluster-tokens.db")).await?;
        // A single synthetic local user owns all desktop cluster tokens (the
        // cluster_oidc_tokens FK references users).
        let user = db
            .upsert_user("desktop", "local", "", "srelens desktop", unix_now())
            .await?;
        let master_key = MasterKey::load_or_generate(None, &config_dir.join("cluster.key"))?;
        let registry = Arc::new(ClusterOidcRegistry::from_kubeconfig_yamls(kubeconfig_yamls));
        Ok(Self {
            db,
            master_key: Arc::new(master_key),
            user_id: user.id,
            registry: std::sync::Mutex::new(registry),
            // Refresh (no browser) uses only the token endpoint; the redirect
            // isn't sent on the refresh grant, so a stable placeholder is fine.
            redirect_base: "http://127.0.0.1".to_string(),
        })
    }

    /// The current OIDC cluster registry (a cheap `Arc` clone).
    pub fn registry(&self) -> Arc<ClusterOidcRegistry> {
        self.registry.lock().unwrap().clone()
    }

    /// Build the provider + resolver from the CURRENT registry and install it
    /// on `cache` so a detected OIDC context authenticates with the managed
    /// token (or signals login).
    pub async fn install_on(&self, cache: &Arc<ClientCache>) {
        let registry = self.registry();
        let refresh = make_refresh_fn(
            registry.clone(),
            format!("{}/auth/cluster/callback", self.redirect_base),
            unix_now,
        );
        let provider = Arc::new(OidcTokenProvider::new(
            self.db.clone(),
            self.master_key.clone(),
            self.user_id,
            refresh,
        ));
        cache
            .set_auth_resolver(Arc::new(ClusterAuthResolver { registry, provider }))
            .await;
    }

    /// Re-read the kubeconfigs on disk, rebuild the OIDC registry, swap it in,
    /// and reinstall the resolver on `cache` so newly detected/removed OIDC
    /// clusters (added at runtime — pasted kubeconfig, file watcher) are
    /// resolvable without an app restart.
    pub async fn rebuild(&self, cache: &Arc<ClientCache>) {
        // Index from the cache's CURRENT path set — not just the default
        // kubeconfig — so clusters added via the app (pasted / Add-cluster form,
        // materialized under the app config dir and pushed into the cache) are
        // recognized. The watcher calls this when cache.paths() changes.
        let yamls = read_kubeconfig_yamls(&cache.paths().await);
        let new_registry = ClusterOidcRegistry::from_kubeconfig_yamls(&yamls);
        *self.registry.lock().unwrap() = Arc::new(new_registry);
        self.install_on(cache).await;
    }
}

fn unix_now() -> i64 {
    srelens_server::unix_now()
}
