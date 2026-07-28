//! Identity-provider abstraction. Routes and middleware depend only on this
//! trait; the real OIDC client is an adapter behind it, so the whole login
//! flow is testable with [`FakeIdp`].

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Everything needed to send the browser to the IdP and later verify the
/// callback: the redirect URL plus the per-login secrets we stash server-side.
#[derive(Debug)]
pub struct LoginBegin {
    pub auth_url: String,
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
}

/// Verified identity claims returned from a completed login.
#[derive(Debug, Clone)]
pub struct IdentityClaims {
    pub iss: String,
    pub sub: String,
    pub email: String,
    pub display_name: String,
}

#[async_trait::async_trait]
pub trait IdentityProvider: Send + Sync {
    fn begin_login(&self) -> Result<LoginBegin, String>;
    async fn complete_login(
        &self,
        code: &str,
        nonce: &str,
        pkce_verifier: &str,
    ) -> Result<IdentityClaims, String>;
}

/// Placeholder provider used when only dev login is configured: browser-based
/// OIDC login is unavailable, and says so.
pub struct NullIdp;

#[async_trait::async_trait]
impl IdentityProvider for NullIdp {
    fn begin_login(&self) -> Result<LoginBegin, String> {
        Err("OIDC is not configured (set SRELENS_OIDC_*)".into())
    }
    async fn complete_login(&self, _: &str, _: &str, _: &str) -> Result<IdentityClaims, String> {
        Err("OIDC is not configured (set SRELENS_OIDC_*)".into())
    }
}

const PENDING_TTL: Duration = Duration::from_secs(600);
const PENDING_CAP: usize = 1000;

struct Pending {
    nonce: String,
    pkce_verifier: String,
    binder_hash: String,
    created_at: Instant,
}

/// A redeemed pending login: the nonce/verifier needed to complete the OIDC
/// exchange, plus the binder hash to verify against the caller's cookie.
#[derive(Debug, PartialEq, Eq)]
pub struct PendingTaken {
    pub nonce: String,
    pub pkce_verifier: String,
    pub binder_hash: String,
}

/// In-flight logins keyed by OIDC `state`, held between /auth/login and
/// /auth/callback. Entries expire after 10 minutes; the map is capped so a
/// login-spam loop can't grow memory unboundedly.
#[derive(Default)]
pub struct PendingLogins {
    inner: Mutex<HashMap<String, Pending>>,
}

impl PendingLogins {
    /// Returns false (and stores nothing) when the cap is reached.
    pub fn insert(
        &self,
        state: String,
        nonce: String,
        pkce_verifier: String,
        binder_hash: String,
    ) -> bool {
        let mut map = self.inner.lock().unwrap();
        map.retain(|_, p| p.created_at.elapsed() < PENDING_TTL);
        if map.len() >= PENDING_CAP {
            return false;
        }
        map.insert(
            state,
            Pending {
                nonce,
                pkce_verifier,
                binder_hash,
                created_at: Instant::now(),
            },
        );
        true
    }

    /// One-shot: a state can only be redeemed once, and only within the TTL.
    pub fn take(&self, state: &str) -> Option<PendingTaken> {
        let mut map = self.inner.lock().unwrap();
        let pending = map.remove(state)?;
        if pending.created_at.elapsed() >= PENDING_TTL {
            return None;
        }
        Some(PendingTaken {
            nonce: pending.nonce,
            pkce_verifier: pending.pkce_verifier,
            binder_hash: pending.binder_hash,
        })
    }
}

/// Test identity provider. `begin_login` returns a fixed URL and a random
/// state; `complete_login` accepts codes of the form `ok:<sub>:<email>` and
/// rejects anything else.
pub struct FakeIdp;

/// A random 128-bit hex string, for the fake IdP's state/nonce/verifier.
fn random_hex() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
    Ok(hex::encode(bytes))
}

#[async_trait::async_trait]
impl IdentityProvider for FakeIdp {
    fn begin_login(&self) -> Result<LoginBegin, String> {
        let state = random_hex()?;
        Ok(LoginBegin {
            auth_url: format!("https://fake-idp.example/authorize?state={state}"),
            state,
            nonce: random_hex()?,
            pkce_verifier: random_hex()?,
        })
    }

    async fn complete_login(
        &self,
        code: &str,
        nonce: &str,
        pkce_verifier: &str,
    ) -> Result<IdentityClaims, String> {
        // A fake IdP doesn't verify the nonce/PKCE against a real token — it
        // only requires they were carried through the flow. The genuine state +
        // PKCE binding is enforced by PendingLogins and the real OidcProvider.
        if nonce.is_empty() || pkce_verifier.is_empty() {
            return Err("missing nonce or verifier".into());
        }
        let mut parts = code.splitn(3, ':');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("ok"), Some(sub), Some(email)) => Ok(IdentityClaims {
                iss: "https://fake-idp.example".into(),
                sub: sub.to_string(),
                email: email.to_string(),
                display_name: sub.to_string(),
            }),
            _ => Err("invalid code".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_state_is_one_shot() {
        let p = PendingLogins::default();
        assert!(p.insert("s1".into(), "n".into(), "v".into(), "h".into()));
        assert_eq!(
            p.take("s1"),
            Some(PendingTaken {
                nonce: "n".into(),
                pkce_verifier: "v".into(),
                binder_hash: "h".into(),
            })
        );
        assert_eq!(p.take("s1"), None);
        assert_eq!(p.take("unknown"), None);
    }

    #[tokio::test]
    async fn fake_idp_round_trips_claims() {
        let idp = FakeIdp;
        let begin = idp.begin_login().unwrap();
        assert!(begin.auth_url.contains(&begin.state));
        let claims = idp
            .complete_login(
                "ok:alice:alice@example.com",
                &begin.nonce,
                &begin.pkce_verifier,
            )
            .await
            .unwrap();
        assert_eq!(claims.sub, "alice");
        assert_eq!(claims.email, "alice@example.com");
        assert!(idp
            .complete_login("garbage", &begin.nonce, &begin.pkce_verifier)
            .await
            .is_err());
        // A missing nonce/verifier is rejected (they must be carried through
        // the flow). The fake doesn't validate their VALUE against a real token
        // — the genuine nonce/PKCE check is the real OidcProvider's job.
        assert!(idp
            .complete_login("ok:alice:alice@example.com", "", &begin.pkce_verifier)
            .await
            .is_err());
        assert!(idp
            .complete_login("ok:alice:alice@example.com", &begin.nonce, "")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn null_idp_reports_oidc_unconfigured() {
        let idp = NullIdp;
        let err = idp.begin_login().unwrap_err();
        assert!(err.contains("OIDC is not configured"));
        let err = idp.complete_login("c", "n", "v").await.unwrap_err();
        assert!(err.contains("OIDC is not configured"));
    }
}
