//! Desktop-side managed cluster OIDC. Reuses the server's OIDC machinery (token
//! provider, registry, resolver, flow) but with a LOCAL sqlite token store
//! under the app config dir and a loopback browser-login flow (see
//! commands in `cluster_oidc_cmd.rs` / lib.rs). Managed sign-in is the desktop
//! default for detected OIDC contexts; non-OIDC exec plugins still run natively.

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
    pub registry: Arc<ClusterOidcRegistry>,
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
            registry,
            // Refresh (no browser) uses only the token endpoint; the redirect
            // isn't sent on the refresh grant, so a stable placeholder is fine.
            redirect_base: "http://127.0.0.1".to_string(),
        })
    }

    /// Build the provider + resolver and install it on `cache` so a detected
    /// OIDC context authenticates with the managed token (or signals login).
    pub async fn install_on(&self, cache: &Arc<ClientCache>) {
        let refresh = make_refresh_fn(
            self.registry.clone(),
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
            .set_auth_resolver(Arc::new(ClusterAuthResolver {
                registry: self.registry.clone(),
                provider,
            }))
            .await;
    }
}

fn unix_now() -> i64 {
    srelens_server::unix_now()
}
