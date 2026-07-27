//! Per-user, per-cluster OIDC tokens (id + refresh), sealed at rest under the
//! master key. Keyed by (user_id, oidc_key) so contexts sharing an issuer+client
//! share one sign-in.

use crate::crypto::{MasterKey, Sealed};
use crate::db::Db;

#[derive(Clone)]
pub struct StoredToken {
    pub id_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
}

impl std::fmt::Debug for StoredToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredToken")
            .field("id_token", &"<redacted>")
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "<redacted>"))
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl Db {
    pub async fn put_cluster_token(
        &self,
        user_id: i64,
        oidc_key: &str,
        key: &MasterKey,
        token: &StoredToken,
    ) -> Result<(), String> {
        let id_sealed = key.seal(token.id_token.as_bytes())?;
        let (refresh_ct, refresh_nonce) = match &token.refresh_token {
            Some(rt) => {
                let s = key.seal(rt.as_bytes())?;
                (Some(s.ciphertext), Some(s.nonce))
            }
            None => (None, None),
        };
        sqlx::query(
            "INSERT INTO cluster_oidc_tokens
               (user_id, oidc_key, id_token_ct, id_token_nonce, refresh_ct, refresh_nonce, expires_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (user_id, oidc_key) DO UPDATE SET
               id_token_ct = excluded.id_token_ct,
               id_token_nonce = excluded.id_token_nonce,
               refresh_ct = excluded.refresh_ct,
               refresh_nonce = excluded.refresh_nonce,
               expires_at = excluded.expires_at",
        )
        .bind(user_id)
        .bind(oidc_key)
        .bind(id_sealed.ciphertext)
        .bind(id_sealed.nonce)
        .bind(refresh_ct)
        .bind(refresh_nonce)
        .bind(token.expires_at)
        .execute(self.pool())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn get_cluster_token(
        &self,
        user_id: i64,
        oidc_key: &str,
        key: &MasterKey,
    ) -> Result<Option<StoredToken>, String> {
        // The row is the sealed id/refresh ciphertext+nonce pairs plus expiry;
        // the tuple mirrors the SELECT below, so a type alias would obscure it.
        #[allow(clippy::type_complexity)]
        let row: Option<(Vec<u8>, Vec<u8>, Option<Vec<u8>>, Option<Vec<u8>>, i64)> = sqlx::query_as(
            "SELECT id_token_ct, id_token_nonce, refresh_ct, refresh_nonce, expires_at
             FROM cluster_oidc_tokens WHERE user_id = ? AND oidc_key = ?",
        )
        .bind(user_id)
        .bind(oidc_key)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| e.to_string())?;

        let Some((id_ct, id_nonce, r_ct, r_nonce, expires_at)) = row else {
            return Ok(None);
        };
        let id_token = String::from_utf8(key.open(&Sealed { ciphertext: id_ct, nonce: id_nonce })?)
            .map_err(|e| e.to_string())?;
        let refresh_token = match (r_ct, r_nonce) {
            (Some(ct), Some(nonce)) => Some(
                String::from_utf8(key.open(&Sealed { ciphertext: ct, nonce })?).map_err(|e| e.to_string())?,
            ),
            _ => None,
        };
        Ok(Some(StoredToken { id_token, refresh_token, expires_at }))
    }

    pub async fn delete_cluster_token(&self, user_id: i64, oidc_key: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM cluster_oidc_tokens WHERE user_id = ? AND oidc_key = ?")
            .bind(user_id)
            .bind(oidc_key)
            .execute(self.pool())
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::MasterKey;

    fn key() -> MasterKey {
        MasterKey::from_hex(&"ab".repeat(32)).unwrap()
    }

    #[tokio::test]
    async fn round_trip_and_ownership() {
        let db = Db::open_in_memory().await.unwrap();
        let k = key();
        let alice = db.upsert_user("i", "alice", "", "", 1).await.unwrap();
        let bob = db.upsert_user("i", "bob", "", "", 1).await.unwrap();

        db.put_cluster_token(alice.id, "K1", &k, &StoredToken {
            id_token: "idtok".into(),
            refresh_token: Some("reftok".into()),
            expires_at: 5000,
        }).await.unwrap();

        // Stored bytes are not the plaintext.
        let (ct,): (Vec<u8>,) = sqlx::query_as("SELECT id_token_ct FROM cluster_oidc_tokens WHERE user_id = ?")
            .bind(alice.id).fetch_one(db.pool()).await.unwrap();
        assert_ne!(ct, b"idtok");

        let got = db.get_cluster_token(alice.id, "K1", &k).await.unwrap().unwrap();
        assert_eq!(got.id_token, "idtok");
        assert_eq!(got.refresh_token, Some("reftok".into()));
        assert_eq!(got.expires_at, 5000);

        // Ownership: bob's same key resolves nothing.
        assert!(db.get_cluster_token(bob.id, "K1", &k).await.unwrap().is_none());

        // No-refresh-token variant.
        db.put_cluster_token(alice.id, "K2", &k, &StoredToken {
            id_token: "x".into(), refresh_token: None, expires_at: 1,
        }).await.unwrap();
        assert_eq!(db.get_cluster_token(alice.id, "K2", &k).await.unwrap().unwrap().refresh_token, None);

        // Delete.
        db.delete_cluster_token(alice.id, "K1").await.unwrap();
        assert!(db.get_cluster_token(alice.id, "K1", &k).await.unwrap().is_none());
    }

    #[test]
    fn stored_token_debug_redacts_secrets() {
        let token = StoredToken {
            id_token: "super-secret-id-token".into(),
            refresh_token: Some("super-secret-refresh-token".into()),
            expires_at: 5000,
        };
        let debug = format!("{:?}", token);
        assert!(!debug.contains("super-secret-id-token"));
        assert!(!debug.contains("super-secret-refresh-token"));
        assert!(debug.contains("<redacted>"));
        assert!(debug.contains("5000"));
    }
}
