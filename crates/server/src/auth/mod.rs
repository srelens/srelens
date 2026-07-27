//! Authentication: configuration, identity-provider abstraction, session
//! middleware, and the login/callback/logout routes.

pub mod cluster_routes;
pub mod idp;
pub mod oidc;
pub mod routes;
pub mod session;

/// OIDC client settings. `Debug` is manual so the client secret can never end
/// up in logs.
#[derive(Clone)]
pub struct OidcSettings {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
}

impl std::fmt::Debug for OidcSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcSettings")
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .finish()
    }
}

/// Auth configuration, resolved once at startup from environment variables.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub public_url: String,
    pub allowed_email_domains: Vec<String>,
    pub dev_login: Option<String>,
    pub oidc: Option<OidcSettings>,
}

impl AuthConfig {
    /// Build from an injected env getter (pure — tests pass a closure over a
    /// map). Requires OIDC settings or a dev login; web mode never runs
    /// unauthenticated.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Result<AuthConfig, String> {
        let non_empty = |k: &str| get(k).filter(|v| !v.trim().is_empty());
        let public_url = non_empty("SRELENS_PUBLIC_URL")
            .unwrap_or_else(|| "http://127.0.0.1:8080".to_string())
            .trim_end_matches('/')
            .to_string();
        let allowed_email_domains = non_empty("SRELENS_OIDC_ALLOWED_DOMAINS")
            .map(|v| {
                v.split(',')
                    .map(|d| d.trim().to_ascii_lowercase())
                    .filter(|d| !d.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let dev_login = non_empty("SRELENS_DEV_LOGIN");
        let oidc = match (
            non_empty("SRELENS_OIDC_ISSUER"),
            non_empty("SRELENS_OIDC_CLIENT_ID"),
            non_empty("SRELENS_OIDC_CLIENT_SECRET"),
        ) {
            (None, None, None) => None,
            (Some(issuer), Some(client_id), Some(client_secret)) => Some(OidcSettings {
                issuer,
                client_id,
                client_secret,
            }),
            _ => {
                return Err(
                    "SRELENS_OIDC_ISSUER, SRELENS_OIDC_CLIENT_ID, and SRELENS_OIDC_CLIENT_SECRET must be set together".into(),
                )
            }
        };
        if oidc.is_none() && dev_login.is_none() {
            return Err(
                "web mode requires auth: set SRELENS_OIDC_* (or SRELENS_DEV_LOGIN for local development)".into(),
            );
        }
        Ok(AuthConfig {
            public_url,
            allowed_email_domains,
            dev_login,
            oidc,
        })
    }

    /// Session cookies are `Secure` when the deployment is served over https.
    pub fn cookie_secure(&self) -> bool {
        self.public_url.starts_with("https://")
    }

    /// Empty allowlist admits everyone; otherwise the email's domain must be
    /// listed (case-insensitive).
    pub fn email_domain_allowed(&self, email: &str) -> bool {
        if self.allowed_email_domains.is_empty() {
            return true;
        }
        let Some(domain) = email.rsplit('@').next().filter(|d| *d != email) else {
            return false;
        };
        self.allowed_email_domains
            .contains(&domain.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn requires_oidc_or_dev_login() {
        assert!(AuthConfig::from_env(env(&[])).is_err());
        assert!(AuthConfig::from_env(env(&[("SRELENS_DEV_LOGIN", "dev@example.com")])).is_ok());
    }

    #[test]
    fn oidc_settings_must_be_complete() {
        let err = AuthConfig::from_env(env(&[("SRELENS_OIDC_ISSUER", "https://idp")])).unwrap_err();
        assert!(err.contains("must be set together"));
        let ok = AuthConfig::from_env(env(&[
            ("SRELENS_OIDC_ISSUER", "https://idp"),
            ("SRELENS_OIDC_CLIENT_ID", "cid"),
            ("SRELENS_OIDC_CLIENT_SECRET", "sec"),
        ]))
        .unwrap();
        assert!(ok.oidc.is_some());
    }

    #[test]
    fn public_url_defaults_and_trims_trailing_slash() {
        let cfg = AuthConfig::from_env(env(&[
            ("SRELENS_DEV_LOGIN", "d@x"),
            ("SRELENS_PUBLIC_URL", "https://srelens.example.com/"),
        ]))
        .unwrap();
        assert_eq!(cfg.public_url, "https://srelens.example.com");
        assert!(cfg.cookie_secure());
        let dflt = AuthConfig::from_env(env(&[("SRELENS_DEV_LOGIN", "d@x")])).unwrap();
        assert_eq!(dflt.public_url, "http://127.0.0.1:8080");
        assert!(!dflt.cookie_secure());
    }

    #[test]
    fn domain_allowlist() {
        let open = AuthConfig::from_env(env(&[("SRELENS_DEV_LOGIN", "d@x")])).unwrap();
        assert!(open.email_domain_allowed("anyone@anywhere.io"));
        let gated = AuthConfig::from_env(env(&[
            ("SRELENS_DEV_LOGIN", "d@x"),
            ("SRELENS_OIDC_ALLOWED_DOMAINS", "Example.com, corp.io"),
        ]))
        .unwrap();
        assert!(gated.email_domain_allowed("a@example.COM"));
        assert!(gated.email_domain_allowed("b@corp.io"));
        assert!(!gated.email_domain_allowed("c@evil.com"));
        assert!(!gated.email_domain_allowed("no-at-sign"));
    }

    #[test]
    fn oidc_settings_debug_redacts_secret() {
        let s = OidcSettings {
            issuer: "i".into(),
            client_id: "c".into(),
            client_secret: "SUPERSECRET".into(),
        };
        let dbg = format!("{s:?}");
        assert!(!dbg.contains("SUPERSECRET"));
        assert!(dbg.contains("<redacted>"));
    }
}
