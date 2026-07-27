//! Resolves the current Bearer id_token for a user's OIDC cluster, refreshing
//! server-side when it's expired, or reporting that an interactive sign-in is
//! required. The actual token-endpoint call is injected (`RefreshFn`) so this
//! is pure/unit-testable; Plan 2 wires the openidconnect-backed implementation.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::cluster_tokens::StoredToken;
use crate::crypto::MasterKey;
use crate::db::Db;

#[derive(Debug, Clone, PartialEq)]
pub struct NeedsLogin {
    pub oidc_key: String,
}

#[derive(Debug, Clone)]
pub struct RefreshedToken {
    pub id_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
}

pub type RefreshFn = Arc<
    dyn Fn(String, String) -> Pin<Box<dyn Future<Output = Result<RefreshedToken, String>> + Send>>
        + Send
        + Sync,
>;

pub struct OidcTokenProvider {
    pub db: Db,
    pub master_key: Arc<MasterKey>,
    pub user_id: i64,
    pub refresh: RefreshFn,
}

impl OidcTokenProvider {
    /// The current Bearer for `oidc_key`, refreshing if expired within `skew_secs`.
    pub async fn current_bearer(
        &self,
        oidc_key: &str,
        now: i64,
        skew_secs: i64,
    ) -> Result<String, NeedsLogin> {
        let stored = self
            .db
            .get_cluster_token(self.user_id, oidc_key, &self.master_key)
            .await
            .map_err(|_| NeedsLogin { oidc_key: oidc_key.to_string() })?;

        let Some(stored) = stored else {
            return Err(NeedsLogin { oidc_key: oidc_key.to_string() });
        };

        if now < stored.expires_at - skew_secs {
            return Ok(stored.id_token);
        }

        // Expired (or within skew): refresh if we can.
        let Some(refresh_token) = stored.refresh_token.clone() else {
            let _ = self.db.delete_cluster_token(self.user_id, oidc_key).await;
            return Err(NeedsLogin { oidc_key: oidc_key.to_string() });
        };

        match (self.refresh)(oidc_key.to_string(), refresh_token).await {
            Ok(refreshed) => {
                let new = StoredToken {
                    id_token: refreshed.id_token.clone(),
                    // Some IdPs rotate refresh tokens; keep the new one, else the old.
                    refresh_token: refreshed.refresh_token.or(stored.refresh_token),
                    expires_at: refreshed.expires_at,
                };
                let _ = self
                    .db
                    .put_cluster_token(self.user_id, oidc_key, &self.master_key, &new)
                    .await;
                Ok(refreshed.id_token)
            }
            Err(_) => {
                let _ = self.db.delete_cluster_token(self.user_id, oidc_key).await;
                Err(NeedsLogin { oidc_key: oidc_key.to_string() })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster_tokens::StoredToken;
    use crate::crypto::MasterKey;

    fn key() -> Arc<MasterKey> {
        Arc::new(MasterKey::from_hex(&"ab".repeat(32)).unwrap())
    }

    fn never_refresh() -> RefreshFn {
        Arc::new(|_, _| Box::pin(async { Err("should not be called".to_string()) }))
    }

    async fn provider(db: Db, refresh: RefreshFn, user_id: i64) -> OidcTokenProvider {
        OidcTokenProvider { db, master_key: key(), user_id, refresh }
    }

    #[tokio::test]
    async fn valid_token_is_returned_without_refresh() {
        let db = Db::open_in_memory().await.unwrap();
        let u = db.upsert_user("i", "s", "", "", 1).await.unwrap();
        db.put_cluster_token(u.id, "K", &key(), &StoredToken {
            id_token: "good".into(), refresh_token: Some("r".into()), expires_at: 1000,
        }).await.unwrap();
        let p = provider(db, never_refresh(), u.id).await;
        assert_eq!(p.current_bearer("K", 500, 60).await.unwrap(), "good");
    }

    #[tokio::test]
    async fn expired_token_is_refreshed_and_persisted() {
        let db = Db::open_in_memory().await.unwrap();
        let u = db.upsert_user("i", "s", "", "", 1).await.unwrap();
        db.put_cluster_token(u.id, "K", &key(), &StoredToken {
            id_token: "old".into(), refresh_token: Some("r".into()), expires_at: 1000,
        }).await.unwrap();
        let refresh: RefreshFn = Arc::new(|_key, _rt| Box::pin(async {
            Ok(RefreshedToken { id_token: "fresh".into(), refresh_token: Some("r2".into()), expires_at: 9000 })
        }));
        let p = provider(db.clone(), refresh, u.id).await;
        // now=2000 is past expires_at=1000 → refresh.
        assert_eq!(p.current_bearer("K", 2000, 60).await.unwrap(), "fresh");
        // Persisted: a fresh read returns the new token+expiry.
        let got = db.get_cluster_token(u.id, "K", &key()).await.unwrap().unwrap();
        assert_eq!(got.id_token, "fresh");
        assert_eq!(got.expires_at, 9000);
        assert_eq!(got.refresh_token, Some("r2".into()));
    }

    #[tokio::test]
    async fn no_token_or_failed_refresh_needs_login() {
        let db = Db::open_in_memory().await.unwrap();
        let u = db.upsert_user("i", "s", "", "", 1).await.unwrap();
        let p = provider(db.clone(), never_refresh(), u.id).await;
        // No token at all.
        assert_eq!(p.current_bearer("K", 1, 60).await.unwrap_err(), NeedsLogin { oidc_key: "K".into() });

        // Expired, no refresh token → needs login + row deleted.
        db.put_cluster_token(u.id, "K", &key(), &StoredToken {
            id_token: "old".into(), refresh_token: None, expires_at: 1000,
        }).await.unwrap();
        assert!(p.current_bearer("K", 2000, 60).await.is_err());
        assert!(db.get_cluster_token(u.id, "K", &key()).await.unwrap().is_none());

        // Expired, refresh fails → needs login + row deleted.
        db.put_cluster_token(u.id, "K", &key(), &StoredToken {
            id_token: "old".into(), refresh_token: Some("r".into()), expires_at: 1000,
        }).await.unwrap();
        let failing: RefreshFn = Arc::new(|_, _| Box::pin(async { Err("revoked".into()) }));
        let p2 = provider(db.clone(), failing, u.id).await;
        assert!(p2.current_bearer("K", 2000, 60).await.is_err());
        assert!(db.get_cluster_token(u.id, "K", &key()).await.unwrap().is_none());
    }
}
