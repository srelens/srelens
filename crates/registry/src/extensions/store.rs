//! Where an extension inventory is kept.
//!
//! The desktop keeps one inventory, in a file beside its settings. The web server keeps
//! one per user, in its database (#515). Both go through [`InventoryStore`], so what is
//! checked on the way in and out is the same for each: the byte limit, the strict parse,
//! the re-verification of every signed manifest on load, and the one lock a
//! read-modify-write holds from its read to its save.
use std::any::Any;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Names one inventory within this process. The registries that serve the same inventory
/// share its app streams under this key, and a write is announced to them by it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InventoryKey {
    /// A file, by the path it was opened with.
    File(PathBuf),
    /// Any other store, by the name it gives itself.
    Named(String),
}

/// Held while one read-modify-write of an inventory runs. Dropping it lets the next in.
pub struct InventoryLock {
    _guard: Box<dyn Any + Send>,
}

impl InventoryLock {
    /// Wrap whatever keeps the store's other writers out: released when this is dropped.
    pub fn new(guard: impl Any + Send) -> Self {
        Self {
            _guard: Box::new(guard),
        }
    }
}

/// Storage for one extension inventory: the saved document, as bytes.
///
/// Every method may block. The host calls them from blocking tasks, never from an async
/// executor thread, so an implementation may wait on a file lock or a database.
pub trait InventoryStore: Send + Sync {
    /// Which inventory this is.
    fn key(&self) -> InventoryKey;
    /// The saved inventory, or `None` when nothing has been saved yet. At most
    /// `limit + 1` bytes of it, so an oversized one is refused without being held whole.
    fn load(&self, limit: usize) -> Result<Option<Vec<u8>>, String>;
    /// Replace the saved inventory with `raw` in one step: a reader sees the old one or
    /// the new one, never part of each.
    fn save(&self, raw: &[u8]) -> Result<(), String>;
    /// Keep every other writer of this inventory out until the lock is dropped.
    fn lock(&self) -> Result<InventoryLock, String>;
}

/// The desktop's store: the inventory file, saved durably and locked across processes, so
/// the app and a headless `srelens mcp` writing the same file never interleave.
impl InventoryStore for Path {
    fn key(&self) -> InventoryKey {
        InventoryKey::File(self.to_path_buf())
    }
    fn load(&self, limit: usize) -> Result<Option<Vec<u8>>, String> {
        let mut raw = Vec::new();
        match fs::File::open(self)
            .and_then(|file| file.take(limit as u64 + 1).read_to_end(&mut raw))
        {
            Ok(_) => Ok(Some(raw)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("read extension inventory: {e}")),
        }
    }
    fn save(&self, raw: &[u8]) -> Result<(), String> {
        crate::durable::replace(self, raw).map_err(|e| format!("save extension inventory: {e}"))
    }
    fn lock(&self) -> Result<InventoryLock, String> {
        crate::settings::write_lock(self).map(InventoryLock::new)
    }
}

impl InventoryStore for PathBuf {
    fn key(&self) -> InventoryKey {
        self.as_path().key()
    }
    fn load(&self, limit: usize) -> Result<Option<Vec<u8>>, String> {
        self.as_path().load(limit)
    }
    fn save(&self, raw: &[u8]) -> Result<(), String> {
        self.as_path().save(raw)
    }
    fn lock(&self) -> Result<InventoryLock, String> {
        self.as_path().lock()
    }
}

impl<T: InventoryStore + ?Sized> InventoryStore for Arc<T> {
    fn key(&self) -> InventoryKey {
        (**self).key()
    }
    fn load(&self, limit: usize) -> Result<Option<Vec<u8>>, String> {
        (**self).load(limit)
    }
    fn save(&self, raw: &[u8]) -> Result<(), String> {
        (**self).save(raw)
    }
    fn lock(&self) -> Result<InventoryLock, String> {
        (**self).lock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_store_reads_nothing_before_the_first_save_and_bounds_what_it_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("apps").join("extensions.json");
        assert_eq!(path.load(8).unwrap(), None);
        let _lock = path.lock().unwrap();
        path.save(b"0123456789").unwrap();
        // One byte past the limit is enough for the caller to refuse it.
        assert_eq!(path.load(4).unwrap().unwrap(), b"01234");
        assert_eq!(path.load(64).unwrap().unwrap(), b"0123456789");
        assert_eq!(path.key(), InventoryKey::File(path.clone()));
    }
}
