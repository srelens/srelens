//! Per-cluster OIDC: discover the cluster's IdP, run authorization-code + PKCE
//! in the browser, and refresh the id_token server-side. Distinct from the
//! app-login OIDC (crates/server/src/auth/oidc.rs) — each cluster has its own
//! issuer/client, read from the user's kubeconfig.

use std::sync::Arc;

use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::reqwest::async_http_client;
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, OAuth2TokenResponse,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, RefreshToken, Scope,
};

use srelens_kube::oidc_detect::OidcClusterConfig;

use crate::cluster_registry::ClusterOidcRegistry;
use crate::oidc_provider::{RefreshFn, RefreshedToken};

/// Discover the cluster's IdP and build a client bound to srelens's callback.
pub async fn build_core_client(
    cfg: &OidcClusterConfig,
    redirect_uri: &str,
) -> Result<CoreClient, String> {
    let issuer = IssuerUrl::new(cfg.issuer.clone()).map_err(|e| format!("invalid issuer: {e}"))?;
    let metadata = CoreProviderMetadata::discover_async(issuer, async_http_client)
        .await
        .map_err(|e| format!("cluster OIDC discovery failed: {e}"))?;
    let redirect = RedirectUrl::new(redirect_uri.to_string()).map_err(|e| e.to_string())?;
    let client = CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(cfg.client_id.clone()),
        cfg.client_secret.clone().map(ClientSecret::new),
    )
    .set_redirect_uri(redirect);
    Ok(client)
}

pub struct ClusterLoginBegin {
    pub auth_url: String,
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
}

/// Begin the authorization-code + PKCE flow for a cluster's IdP. Requests
/// `offline_access` (in addition to the standard identity scopes) so the
/// token endpoint returns a refresh token we can use server-side, plus any
/// `extra_scopes` the kubeconfig's OIDC user asked for.
pub async fn begin_login(
    cfg: &OidcClusterConfig,
    redirect_uri: &str,
) -> Result<ClusterLoginBegin, String> {
    let client = build_core_client(cfg, redirect_uri).await?;
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let mut req = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("openid".to_string()))
        .add_scope(Scope::new("email".to_string()))
        .add_scope(Scope::new("profile".to_string()))
        .add_scope(Scope::new("offline_access".to_string()))
        .set_pkce_challenge(pkce_challenge);
    for s in &cfg.extra_scopes {
        req = req.add_scope(Scope::new(s.clone()));
    }
    let (auth_url, state, nonce) = req.url();
    Ok(ClusterLoginBegin {
        auth_url: auth_url.to_string(),
        state: state.secret().clone(),
        nonce: nonce.secret().clone(),
        pkce_verifier: pkce_verifier.secret().clone(),
    })
}

/// Exchange the callback code for tokens. `expires_at` is absolute epoch
/// secs: `now + expires_in`, falling back to `now + 3600` when the token
/// endpoint doesn't report a lifetime.
pub async fn exchange_code(
    cfg: &OidcClusterConfig,
    redirect_uri: &str,
    code: &str,
    pkce_verifier: &str,
    now: i64,
) -> Result<RefreshedToken, String> {
    let client = build_core_client(cfg, redirect_uri).await?;
    let tokens = client
        .exchange_code(AuthorizationCode::new(code.to_string()))
        .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_string()))
        .request_async(async_http_client)
        .await
        .map_err(|_| "cluster code exchange failed".to_string())?;
    into_refreshed(&tokens, now)
}

/// Pull the id_token/refresh_token/lifetime out of a token response into our
/// storage shape. Never logs the token values.
fn into_refreshed(
    tokens: &openidconnect::core::CoreTokenResponse,
    now: i64,
) -> Result<RefreshedToken, String> {
    let id_token = tokens
        .extra_fields()
        .id_token()
        .ok_or("IdP returned no id_token")?
        .to_string();
    let refresh_token = tokens.refresh_token().map(|t| t.secret().clone());
    let expires_at = tokens
        .expires_in()
        .map(|d| now + d.as_secs() as i64)
        .unwrap_or(now + 3600);
    Ok(RefreshedToken {
        id_token,
        refresh_token,
        expires_at,
    })
}

/// The real `RefreshFn` for `OidcTokenProvider`: look up the cluster config by
/// key, then run the refresh grant. `now_fn` supplies absolute time so
/// `expires_at` stays a pure function of it (testable, no wall-clock reads
/// buried in the flow).
pub fn make_refresh_fn(
    registry: Arc<ClusterOidcRegistry>,
    redirect_uri: String,
    now_fn: fn() -> i64,
) -> RefreshFn {
    Arc::new(move |oidc_key: String, refresh_token: String| {
        let registry = registry.clone();
        let redirect_uri = redirect_uri.clone();
        Box::pin(async move {
            let cfg = registry
                .config_for_key(&oidc_key)
                .ok_or_else(|| "unknown cluster oidc key".to_string())?;
            let client = build_core_client(&cfg, &redirect_uri).await?;
            let tokens = client
                .exchange_refresh_token(&RefreshToken::new(refresh_token))
                .request_async(async_http_client)
                .await
                .map_err(|_| "cluster token refresh failed".to_string())?;
            into_refreshed(&tokens, now_fn())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn refresh_fn_errors_on_unknown_key_without_network() {
        let registry = Arc::new(ClusterOidcRegistry::default());
        let f = make_refresh_fn(
            registry,
            "http://localhost/auth/cluster/callback".into(),
            || 1000,
        );
        let err = f("no-such-key".into(), "rt".into()).await.unwrap_err();
        assert!(err.contains("unknown cluster oidc key"));
    }
}
