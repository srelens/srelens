//! Server-side [`AuthResolver`]: maps a kubeconfig context to a srelens-managed
//! OIDC Bearer token (or a needs-login signal) using the per-user cluster
//! registry and token provider. Installed on each user's `ClientCache` so an
//! OIDC-protected cluster authenticates with the managed id_token instead of a
//! (headless-broken) kubelogin exec plugin.

use std::sync::Arc;

use srelens_kube::auth_resolver::{needs_login_marker, AuthMode, AuthResolver};

use crate::cluster_registry::ClusterOidcRegistry;
use crate::oidc_provider::OidcTokenProvider;

/// Skew, in seconds, before a token's `exp` at which we proactively refresh.
const TOKEN_SKEW_SECS: i64 = 60;

pub struct ClusterAuthResolver {
    pub registry: Arc<ClusterOidcRegistry>,
    pub provider: Arc<OidcTokenProvider>,
}

#[async_trait::async_trait]
impl AuthResolver for ClusterAuthResolver {
    async fn resolve(&self, context: &str) -> Result<AuthMode, String> {
        // Not an OIDC cluster → let kube-rs resolve auth from the kubeconfig.
        let Some(key) = self.registry.key_for_context(context) else {
            return Ok(AuthMode::Default);
        };
        match self
            .provider
            .current_bearer(&key, crate::unix_now(), TOKEN_SKEW_SECS)
            .await
        {
            Ok(bearer) => Ok(AuthMode::Bearer(bearer)),
            // No valid token / refresh failed → signal an interactive login.
            Err(_needs_login) => Err(needs_login_marker(&key, context)),
        }
    }
}
