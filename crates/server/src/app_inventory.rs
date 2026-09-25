//! Each web user's app inventory (#515), kept as that user's row of the server
//! database rather than as a file.
//!
//! A file under the user's runtime directory would not do: `runtime/` is a
//! materialisation of what the database holds (tmpfs in the shipped image), and
//! it is removed whenever the user's environment is rebuilt, evicted or torn
//! down. The inventory has to outlive all of those, and go when the account does
//! (`ON DELETE CASCADE`), which is what the database already does for everything
//! else a user owns.
//!
//! The registry reads and writes an inventory through
//! [`srelens_registry::InventoryStore`] from blocking tasks, so this store blocks
//! on its query there. Everything the registry checks on the way in and out —
//! the byte limit, the strict parse, re-verifying each signed manifest, and the
//! install checks — is the registry's, and the same as the desktop's.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use srelens_registry::{InventoryKey, InventoryLock, InventoryStore};

use crate::db::Db;

/// Distinguishes one environment build from the next, see [`DbInventory::new`].
static BUILDS: AtomicU64 = AtomicU64::new(0);

/// One web user's inventory, as the row `extension_inventories` keeps for them.
pub struct DbInventory {
    db: Db,
    user_id: i64,
    runtime: tokio::runtime::Handle,
    /// This user's one writer lock, shared by every store built for them in this
    /// process (see `UserEnvs`), so a request still running on an environment
    /// that has since been rebuilt cannot interleave with one on the new one.
    lock: Arc<tokio::sync::Mutex<()>>,
    key: String,
}

impl DbInventory {
    /// The inventory of `user_id`, queried through `runtime`.
    ///
    /// Its key names this build as well as the user. App streams are shared by
    /// key, and they hold the client cache of the registry that started them;
    /// a rebuilt environment has new kubeconfig paths, so it must start its own
    /// rather than join streams bound to the old ones' clusters.
    pub fn new(
        db: Db,
        user_id: i64,
        runtime: tokio::runtime::Handle,
        lock: Arc<tokio::sync::Mutex<()>>,
    ) -> Self {
        let build = BUILDS.fetch_add(1, Ordering::Relaxed);
        Self {
            db,
            user_id,
            runtime,
            lock,
            key: format!("srelens-server/users/{user_id}/{build}"),
        }
    }
}

impl InventoryStore for DbInventory {
    fn key(&self) -> InventoryKey {
        InventoryKey::Named(self.key.clone())
    }
    fn load(&self, limit: usize) -> Result<Option<Vec<u8>>, String> {
        self.runtime.block_on(
            self.db
                .get_extension_inventory(self.user_id, limit.saturating_add(1)),
        )
    }
    fn save(&self, raw: &[u8]) -> Result<(), String> {
        self.runtime.block_on(
            self.db
                .put_extension_inventory(self.user_id, raw, crate::unix_now()),
        )
    }
    fn lock(&self) -> Result<InventoryLock, String> {
        Ok(InventoryLock::new(self.lock.clone().blocking_lock_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn a_users_inventory_is_their_row_and_loads_one_byte_past_the_limit() {
        let db = Db::open_in_memory().await.unwrap();
        let alice = db.upsert_user("i", "alice", "", "", 1).await.unwrap();
        let bob = db.upsert_user("i", "bob", "", "", 1).await.unwrap();
        let runtime = tokio::runtime::Handle::current();
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let store = DbInventory::new(db.clone(), alice.id, runtime.clone(), lock.clone());
        let other = DbInventory::new(db.clone(), bob.id, runtime.clone(), Arc::default());
        let rebuilt = DbInventory::new(db.clone(), alice.id, runtime, lock);
        assert_ne!(store.key(), rebuilt.key());

        tokio::task::spawn_blocking(move || {
            assert_eq!(store.load(8).unwrap(), None);
            let held = store.lock().unwrap();
            // The same user's other store waits for it, however it was built.
            assert!(rebuilt.lock.try_lock().is_err());
            store.save(b"0123456789").unwrap();
            drop(held);
            assert_eq!(rebuilt.load(4).unwrap().unwrap(), b"01234");
            assert_eq!(rebuilt.load(64).unwrap().unwrap(), b"0123456789");
            assert_eq!(other.load(64).unwrap(), None);
        })
        .await
        .unwrap();
    }
}
