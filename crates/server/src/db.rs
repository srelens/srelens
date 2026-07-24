//! SQLite persistence: pool construction and embedded migrations. Store
//! methods (users, sessions, kubeconfigs, settings) are implemented on [`Db`]
//! in this module as well.

use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;

/// Handle to the server database. Cheap to clone (wraps a pool).
#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    /// Open (creating if missing) the database at `path`, enable WAL and
    /// foreign keys, and run embedded migrations.
    pub async fn open(path: &Path) -> Result<Self, String> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await
            .map_err(|e| format!("open database {}: {e}", path.display()))?;
        Self::migrate(pool).await
    }

    /// In-memory database for tests. One connection only — every connection
    /// to `:memory:` is otherwise a separate empty database.
    pub async fn open_in_memory() -> Result<Self, String> {
        let opts = SqliteConnectOptions::new()
            .filename(":memory:")
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .map_err(|e| format!("open in-memory database: {e}"))?;
        Self::migrate(pool).await
    }

    async fn migrate(pool: SqlitePool) -> Result<Self, String> {
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| format!("run migrations: {e}"))?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_create_all_tables() {
        let db = Db::open_in_memory().await.unwrap();
        for table in ["users", "sessions", "kubeconfigs", "settings"] {
            let found: Option<(String,)> =
                sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?")
                    .bind(table)
                    .fetch_optional(db.pool())
                    .await
                    .unwrap();
            assert!(found.is_some(), "missing table {table}");
        }
    }

    #[tokio::test]
    async fn open_creates_file_and_is_reopenable() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("srelens.db");
        {
            let db = Db::open(&path).await.unwrap();
            sqlx::query(
                "INSERT INTO users (iss, sub, created_at, last_login_at) VALUES ('i', 's', 1, 1)",
            )
            .execute(db.pool())
            .await
            .unwrap();
        }
        let db = Db::open(&path).await.unwrap(); // migrations are idempotent
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
}
