//! Typed store methods over the server database. All timestamps are unix
//! seconds passed in by the caller; nothing here reads the clock.

use sha2::{Digest, Sha256};

use crate::crypto::{MasterKey, Sealed};
use crate::db::Db;

/// Idle session expiry: 12 hours.
pub const SESSION_IDLE_TTL_SECS: i64 = 43_200;
/// Absolute session expiry: 7 days.
pub const SESSION_ABSOLUTE_TTL_SECS: i64 = 604_800;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub iss: String,
    pub sub: String,
    pub email: String,
    pub display_name: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct KubeconfigMeta {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

fn hash_token(raw: &str) -> String {
    hex::encode(Sha256::digest(raw.as_bytes()))
}

impl Db {
    /// Insert the user on first login (keyed by `iss`+`sub`), or refresh
    /// email/display name/last login on subsequent logins.
    pub async fn upsert_user(
        &self,
        iss: &str,
        sub: &str,
        email: &str,
        display_name: &str,
        now: i64,
    ) -> Result<User, String> {
        sqlx::query(
            "INSERT INTO users (iss, sub, email, display_name, created_at, last_login_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (iss, sub) DO UPDATE SET
               email = excluded.email,
               display_name = excluded.display_name,
               last_login_at = excluded.last_login_at",
        )
        .bind(iss)
        .bind(sub)
        .bind(email)
        .bind(display_name)
        .bind(now)
        .bind(now)
        .execute(self.pool())
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query_as::<_, User>(
            "SELECT id, iss, sub, email, display_name FROM users WHERE iss = ? AND sub = ?",
        )
        .bind(iss)
        .bind(sub)
        .fetch_one(self.pool())
        .await
        .map_err(|e| e.to_string())
    }

    /// Create a session and return the RAW opaque token (256-bit, hex). Only
    /// its SHA-256 is stored — a leaked database cannot mint valid cookies.
    pub async fn create_session(&self, user_id: i64, now: i64) -> Result<String, String> {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
        let raw = hex::encode(bytes);
        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, created_at, expires_at, last_seen_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(hash_token(&raw))
        .bind(user_id)
        .bind(now)
        .bind(now + SESSION_ABSOLUTE_TTL_SECS)
        .bind(now)
        .execute(self.pool())
        .await
        .map_err(|e| e.to_string())?;
        Ok(raw)
    }

    /// Resolve a raw token to its user. Enforces absolute expiry
    /// (`expires_at`) and idle expiry (`last_seen_at` + idle TTL); an expired
    /// row is deleted. A valid session gets `last_seen_at` refreshed.
    pub async fn validate_session(
        &self,
        raw_token: &str,
        now: i64,
    ) -> Result<Option<User>, String> {
        let hash = hash_token(raw_token);
        let row: Option<(i64, i64, i64)> = sqlx::query_as(
            "SELECT user_id, expires_at, last_seen_at FROM sessions WHERE token_hash = ?",
        )
        .bind(&hash)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| e.to_string())?;

        let Some((user_id, expires_at, last_seen_at)) = row else {
            return Ok(None);
        };
        if now > expires_at || now > last_seen_at + SESSION_IDLE_TTL_SECS {
            sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
                .bind(&hash)
                .execute(self.pool())
                .await
                .map_err(|e| e.to_string())?;
            return Ok(None);
        }
        sqlx::query("UPDATE sessions SET last_seen_at = ? WHERE token_hash = ?")
            .bind(now)
            .bind(&hash)
            .execute(self.pool())
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query_as::<_, User>(
            "SELECT id, iss, sub, email, display_name FROM users WHERE id = ?",
        )
        .bind(user_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| e.to_string())
    }

    pub async fn revoke_session(&self, raw_token: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(hash_token(raw_token))
            .execute(self.pool())
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Delete every session past either expiry. Returns how many were removed.
    pub async fn purge_expired_sessions(&self, now: i64) -> Result<u64, String> {
        let result =
            sqlx::query("DELETE FROM sessions WHERE expires_at < ? OR last_seen_at + ? < ?")
                .bind(now)
                .bind(SESSION_IDLE_TTL_SECS)
                .bind(now)
                .execute(self.pool())
                .await
                .map_err(|e| e.to_string())?;
        Ok(result.rows_affected())
    }

    /// Insert or replace a user's kubeconfig by name, sealed under the master
    /// key. Returns the row id.
    pub async fn put_kubeconfig(
        &self,
        user_id: i64,
        name: &str,
        key: &MasterKey,
        yaml: &str,
        now: i64,
    ) -> Result<i64, String> {
        let sealed = key.seal(yaml.as_bytes())?;
        sqlx::query(
            "INSERT INTO kubeconfigs (user_id, name, ciphertext, nonce, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (user_id, name) DO UPDATE SET
               ciphertext = excluded.ciphertext,
               nonce = excluded.nonce,
               updated_at = excluded.updated_at",
        )
        .bind(user_id)
        .bind(name)
        .bind(&sealed.ciphertext)
        .bind(&sealed.nonce)
        .bind(now)
        .bind(now)
        .execute(self.pool())
        .await
        .map_err(|e| e.to_string())?;
        let (id,): (i64,) =
            sqlx::query_as("SELECT id FROM kubeconfigs WHERE user_id = ? AND name = ?")
                .bind(user_id)
                .bind(name)
                .fetch_one(self.pool())
                .await
                .map_err(|e| e.to_string())?;
        Ok(id)
    }

    pub async fn list_kubeconfigs(&self, user_id: i64) -> Result<Vec<KubeconfigMeta>, String> {
        sqlx::query_as::<_, KubeconfigMeta>(
            "SELECT id, name, created_at, updated_at FROM kubeconfigs
             WHERE user_id = ? ORDER BY name",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await
        .map_err(|e| e.to_string())
    }

    /// Decrypt one kubeconfig. The `user_id` in the WHERE clause is the
    /// ownership check — another user's id never resolves the row.
    pub async fn get_kubeconfig_yaml(
        &self,
        user_id: i64,
        id: i64,
        key: &MasterKey,
    ) -> Result<Option<String>, String> {
        let row: Option<(Vec<u8>, Vec<u8>)> = sqlx::query_as(
            "SELECT ciphertext, nonce FROM kubeconfigs WHERE id = ? AND user_id = ?",
        )
        .bind(id)
        .bind(user_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| e.to_string())?;
        let Some((ciphertext, nonce)) = row else {
            return Ok(None);
        };
        let plain = key.open(&Sealed { ciphertext, nonce })?;
        String::from_utf8(plain)
            .map(Some)
            .map_err(|e| e.to_string())
    }

    pub async fn delete_kubeconfig(&self, user_id: i64, id: i64) -> Result<bool, String> {
        let result = sqlx::query("DELETE FROM kubeconfigs WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .execute(self.pool())
            .await
            .map_err(|e| e.to_string())?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn set_setting(
        &self,
        user_id: i64,
        key: &str,
        value_json: &str,
    ) -> Result<(), String> {
        sqlx::query(
            "INSERT INTO settings (user_id, key, value_json) VALUES (?, ?, ?)
             ON CONFLICT (user_id, key) DO UPDATE SET value_json = excluded.value_json",
        )
        .bind(user_id)
        .bind(key)
        .bind(value_json)
        .execute(self.pool())
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn get_setting(&self, user_id: i64, key: &str) -> Result<Option<String>, String> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value_json FROM settings WHERE user_id = ? AND key = ?")
                .bind(user_id)
                .bind(key)
                .fetch_optional(self.pool())
                .await
                .map_err(|e| e.to_string())?;
        Ok(row.map(|(v,)| v))
    }

    pub async fn delete_setting(&self, user_id: i64, key: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM settings WHERE user_id = ? AND key = ?")
            .bind(user_id)
            .bind(key)
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

    async fn db() -> Db {
        Db::open_in_memory().await.unwrap()
    }

    fn key() -> MasterKey {
        MasterKey::from_hex(&"ab".repeat(32)).unwrap()
    }

    #[tokio::test]
    async fn upsert_user_is_stable_by_iss_sub() {
        let db = db().await;
        let a = db
            .upsert_user("https://idp", "u1", "a@x", "A", 100)
            .await
            .unwrap();
        let b = db
            .upsert_user("https://idp", "u1", "b@x", "B", 200)
            .await
            .unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(b.email, "b@x");
        let other = db
            .upsert_user("https://idp", "u2", "", "", 100)
            .await
            .unwrap();
        assert_ne!(a.id, other.id);
    }

    #[tokio::test]
    async fn session_lifecycle() {
        let db = db().await;
        let user = db.upsert_user("i", "s", "", "", 100).await.unwrap();
        let token = db.create_session(user.id, 1_000).await.unwrap();
        assert_eq!(token.len(), 64); // 32 bytes hex

        // Raw token is not stored anywhere.
        let (stored,): (String,) = sqlx::query_as("SELECT token_hash FROM sessions")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_ne!(stored, token);

        // Valid within idle window; last_seen refreshes.
        let got = db.validate_session(&token, 2_000).await.unwrap().unwrap();
        assert_eq!(got.id, user.id);

        // Unknown token → None.
        assert!(db
            .validate_session("deadbeef", 2_000)
            .await
            .unwrap()
            .is_none());

        // Revoked → None.
        db.revoke_session(&token).await.unwrap();
        assert!(db.validate_session(&token, 2_500).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn session_idle_and_absolute_expiry() {
        let db = db().await;
        let user = db.upsert_user("i", "s", "", "", 100).await.unwrap();

        // Idle expiry: valid at +idle, gone one second later.
        let t1 = db.create_session(user.id, 1_000).await.unwrap();
        assert!(db
            .validate_session(&t1, 1_000 + SESSION_IDLE_TTL_SECS)
            .await
            .unwrap()
            .is_some());
        let t2 = db.create_session(user.id, 1_000).await.unwrap();
        assert!(db
            .validate_session(&t2, 1_000 + SESSION_IDLE_TTL_SECS + 1)
            .await
            .unwrap()
            .is_none());

        // Absolute expiry: even continuously-touched sessions die at 7 days.
        let t3 = db.create_session(user.id, 1_000).await.unwrap();
        let mut now = 1_000;
        while now < 1_000 + SESSION_ABSOLUTE_TTL_SECS {
            now += SESSION_IDLE_TTL_SECS; // touch before idle expiry
            if db.validate_session(&t3, now).await.unwrap().is_none() {
                break;
            }
        }
        assert!(db
            .validate_session(&t3, 1_000 + SESSION_ABSOLUTE_TTL_SECS + 1)
            .await
            .unwrap()
            .is_none());

        // Purge removes expired rows.
        let removed = db
            .purge_expired_sessions(1_000 + SESSION_ABSOLUTE_TTL_SECS + 1)
            .await
            .unwrap();
        assert!(removed >= 1);
    }

    #[tokio::test]
    async fn kubeconfig_sealed_roundtrip_and_ownership() {
        let db = db().await;
        let k = key();
        let alice = db.upsert_user("i", "alice", "", "", 1).await.unwrap();
        let bob = db.upsert_user("i", "bob", "", "", 1).await.unwrap();

        let id = db
            .put_kubeconfig(alice.id, "prod", &k, "apiVersion: v1", 10)
            .await
            .unwrap();

        // Stored bytes are not the plaintext.
        let (ct,): (Vec<u8>,) = sqlx::query_as("SELECT ciphertext FROM kubeconfigs WHERE id = ?")
            .bind(id)
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_ne!(ct, b"apiVersion: v1");

        // Owner decrypts; the other user resolves nothing.
        assert_eq!(
            db.get_kubeconfig_yaml(alice.id, id, &k)
                .await
                .unwrap()
                .unwrap(),
            "apiVersion: v1"
        );
        assert!(db
            .get_kubeconfig_yaml(bob.id, id, &k)
            .await
            .unwrap()
            .is_none());

        // Upsert by (user, name) replaces content, keeps one row.
        let id2 = db
            .put_kubeconfig(alice.id, "prod", &k, "apiVersion: v2", 20)
            .await
            .unwrap();
        assert_eq!(id, id2);
        let metas = db.list_kubeconfigs(alice.id).await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].updated_at, 20);

        // Delete enforces ownership too.
        assert!(!db.delete_kubeconfig(bob.id, id).await.unwrap());
        assert!(db.delete_kubeconfig(alice.id, id).await.unwrap());
        assert!(db.list_kubeconfigs(alice.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn settings_roundtrip() {
        let db = db().await;
        let user = db.upsert_user("i", "s", "", "", 1).await.unwrap();
        assert!(db.get_setting(user.id, "theme").await.unwrap().is_none());
        db.set_setting(user.id, "theme", "\"dark\"").await.unwrap();
        db.set_setting(user.id, "theme", "\"light\"").await.unwrap();
        assert_eq!(
            db.get_setting(user.id, "theme").await.unwrap().unwrap(),
            "\"light\""
        );
    }

    #[tokio::test]
    async fn delete_setting_clears_the_value() {
        let db = db().await;
        let user = db.upsert_user("i", "s", "", "", 1).await.unwrap();
        db.set_setting(user.id, "theme", "\"dark\"").await.unwrap();
        db.delete_setting(user.id, "theme").await.unwrap();
        assert!(db.get_setting(user.id, "theme").await.unwrap().is_none());

        // Deleting an already-absent key is a no-op, not an error.
        db.delete_setting(user.id, "theme").await.unwrap();
        db.delete_setting(user.id, "never-set").await.unwrap();
    }

    #[tokio::test]
    async fn settings_are_isolated_per_user() {
        let db = db().await;
        let alice = db.upsert_user("i", "alice", "", "", 1).await.unwrap();
        let bob = db.upsert_user("i", "bob", "", "", 1).await.unwrap();

        db.set_setting(alice.id, "theme", "\"dark\"").await.unwrap();
        assert!(db.get_setting(bob.id, "theme").await.unwrap().is_none());

        db.set_setting(bob.id, "theme", "\"light\"").await.unwrap();
        assert_eq!(
            db.get_setting(alice.id, "theme").await.unwrap().unwrap(),
            "\"dark\""
        );
        assert_eq!(
            db.get_setting(bob.id, "theme").await.unwrap().unwrap(),
            "\"light\""
        );

        // Deleting one user's setting doesn't touch the other's.
        db.delete_setting(alice.id, "theme").await.unwrap();
        assert!(db.get_setting(alice.id, "theme").await.unwrap().is_none());
        assert_eq!(
            db.get_setting(bob.id, "theme").await.unwrap().unwrap(),
            "\"light\""
        );
    }
}
