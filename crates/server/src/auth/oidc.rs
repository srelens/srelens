//! The real OIDC identity provider: authorization-code flow with PKCE via
//! the openidconnect crate. Constructed once at startup with fail-fast
//! issuer discovery.

use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::reqwest::async_http_client;
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, ErrorResponse, IssuerUrl, Nonce,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, RequestTokenError, Scope,
};

use super::idp::{IdentityClaims, IdentityProvider, LoginBegin};
use super::OidcSettings;

pub struct OidcProvider {
    client: CoreClient,
}

impl OidcProvider {
    /// Discover the issuer and build the client. Fails fast at startup so a
    /// misconfigured issuer is a boot error, not a first-login surprise.
    pub async fn discover(settings: &OidcSettings, public_url: &str) -> Result<Self, String> {
        let issuer = IssuerUrl::new(settings.issuer.clone())
            .map_err(|e| format!("invalid SRELENS_OIDC_ISSUER: {e}"))?;
        let metadata = CoreProviderMetadata::discover_async(issuer, async_http_client)
            .await
            .map_err(|e| format!("OIDC discovery failed: {e}"))?;
        let redirect = RedirectUrl::new(format!("{public_url}/auth/callback"))
            .map_err(|e| format!("invalid redirect url: {e}"))?;
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(settings.client_id.clone()),
            Some(ClientSecret::new(settings.client_secret.clone())),
        )
        .set_redirect_uri(redirect);
        Ok(Self { client })
    }
}

/// Summarize a token-exchange error without ever propagating the raw
/// token-endpoint response body into a user- or log-facing string.
///
/// `oauth2`'s `RequestTokenError` variants already avoid this in their own
/// `Display` impl: each formats to a fixed, kind-only message (e.g. "Server
/// returned error response") except `Other`, which only ever carries
/// internal diagnostic strings the library itself constructs — never HTTP
/// response bytes (those live in the `Parse` variant's second field, which
/// its `Display` impl does not print). We still match on the kind
/// explicitly, rather than trust `{e}` forever, so a future dependency bump
/// that starts embedding response content in `Display` can't silently leak
/// it through this path.
fn summarize_exchange_error<RE, TE>(err: &RequestTokenError<RE, TE>) -> String
where
    RE: std::error::Error + 'static,
    TE: ErrorResponse + 'static,
{
    match err {
        RequestTokenError::ServerResponse(_) => {
            "token endpoint returned an error response".to_string()
        }
        RequestTokenError::Request(_) => "request to token endpoint failed".to_string(),
        RequestTokenError::Parse(_, _) => "failed to parse token endpoint response".to_string(),
        RequestTokenError::Other(msg) => format!("token exchange error: {msg}"),
    }
}

#[async_trait::async_trait]
impl IdentityProvider for OidcProvider {
    fn begin_login(&self) -> Result<LoginBegin, String> {
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let (auth_url, state, nonce) = self
            .client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .add_scope(Scope::new("email".to_string()))
            .add_scope(Scope::new("profile".to_string()))
            .set_pkce_challenge(pkce_challenge)
            .url();
        Ok(LoginBegin {
            auth_url: auth_url.to_string(),
            state: state.secret().clone(),
            nonce: nonce.secret().clone(),
            pkce_verifier: pkce_verifier.secret().clone(),
        })
    }

    async fn complete_login(
        &self,
        code: &str,
        nonce: &str,
        pkce_verifier: &str,
    ) -> Result<IdentityClaims, String> {
        let tokens = self
            .client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_string()))
            .request_async(async_http_client)
            .await
            .map_err(|e| summarize_exchange_error(&e))?;
        let id_token = tokens
            .extra_fields()
            .id_token()
            .ok_or("IdP returned no ID token")?;
        let claims = id_token
            .claims(
                &self.client.id_token_verifier(),
                &Nonce::new(nonce.to_string()),
            )
            .map_err(|e| format!("ID token verification failed: {e}"))?;
        let email = claims
            .email()
            .map(|e| e.as_str().to_string())
            .unwrap_or_default();
        let display_name = claims
            .name()
            .and_then(|n| n.get(None))
            .map(|n| n.as_str().to_string())
            .unwrap_or_else(|| email.clone());
        if email.is_empty() {
            return Err("IdP returned no email claim (request the email scope)".into());
        }
        if claims.email_verified() == Some(false) {
            return Err("IdP reports the email address is unverified".into());
        }
        Ok(IdentityClaims {
            iss: claims.issuer().as_str().to_string(),
            sub: claims.subject().as_str().to_string(),
            email,
            display_name,
        })
    }
}
