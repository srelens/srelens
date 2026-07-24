//! Per-user execution environments: each user's capability calls run against
//! a registry + client cache built ONLY from their own uploaded kubeconfigs,
//! materialized as 0600 files under `<data>/runtime/users/<id>/`.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use srelens_capability::Registry;
use srelens_kube::client_cache::ClientCache;

use crate::crypto::MasterKey;
use crate::db::Db;
use crate::RegistryFactory;

/// Idle eviction threshold for a user's environment.
pub const USER_ENV_IDLE_SECS: u64 = 1800;

pub struct UserEnv {
    pub registry: Arc<Registry>,
    pub cache: Arc<ClientCache>,
    pub paths: Vec<PathBuf>,
    pub streams: Arc<crate::streams::UserStreams>,
    last_used: Mutex<Instant>,
}

pub struct UserEnvs {
    factory: RegistryFactory,
    data_dir: PathBuf,
    map: Mutex<HashMap<i64, Arc<UserEnv>>>,
}

fn user_runtime_dir(data_dir: &Path, user_id: i64) -> PathBuf {
    data_dir
        .join("runtime")
        .join("users")
        .join(user_id.to_string())
}

fn create_private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Tighten every component we own (runtime/, users/, <id>/).
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("chmod {}: {e}", dir.display()))?;
    }
    Ok(())
}

fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts
        .open(path)
        .map_err(|e| format!("create {}: {e}", path.display()))?;
    file.write_all(contents)
        .map_err(|e| format!("write {}: {e}", path.display()))
}

impl UserEnvs {
    pub fn new(factory: RegistryFactory, data_dir: PathBuf) -> Self {
        Self {
            factory,
            data_dir,
            map: Mutex::new(HashMap::new()),
        }
    }

    /// Remove ALL materialized runtime files (startup hygiene: a crash may
    /// have left decrypted kubeconfigs behind).
    pub fn wipe_runtime(data_dir: &Path) {
        let _ = std::fs::remove_dir_all(data_dir.join("runtime"));
    }

    /// Get (building if needed) the user's environment and touch its
    /// last-used stamp.
    pub async fn env_for(
        &self,
        db: &Db,
        key: &MasterKey,
        user_id: i64,
    ) -> Result<Arc<UserEnv>, String> {
        if let Some(env) = self.map.lock().unwrap().get(&user_id).cloned() {
            *env.last_used.lock().unwrap() = Instant::now();
            return Ok(env);
        }

        // Build outside the map lock: decrypt + materialize + construct.
        let metas = db.list_kubeconfigs(user_id).await?;
        let dir = user_runtime_dir(&self.data_dir, user_id);
        let _ = std::fs::remove_dir_all(&dir);
        create_private_dir(&dir)?;
        let mut paths = Vec::with_capacity(metas.len());
        for meta in &metas {
            let yaml = db
                .get_kubeconfig_yaml(user_id, meta.id, key)
                .await?
                .ok_or_else(|| format!("kubeconfig {} disappeared", meta.id))?;
            let path = dir.join(format!("kc-{}.yaml", meta.id));
            write_private_file(&path, yaml.as_bytes())?;
            paths.push(path);
        }

        let cache = ClientCache::new_many(paths.clone());
        let registry = Arc::new((self.factory)(cache.clone(), paths.clone()));
        let streams = Arc::new(crate::streams::UserStreams::new(cache.clone()));
        let env = Arc::new(UserEnv {
            registry,
            cache,
            paths,
            streams,
            last_used: Mutex::new(Instant::now()),
        });
        // Last-writer-wins is fine: both candidates were built from the same
        // stored kubeconfigs.
        self.map.lock().unwrap().insert(user_id, env.clone());
        Ok(env)
    }

    /// Drop a user's environment (their kubeconfigs changed) and remove the
    /// materialized files.
    pub fn invalidate(&self, user_id: i64) {
        self.map.lock().unwrap().remove(&user_id);
        let _ = std::fs::remove_dir_all(user_runtime_dir(&self.data_dir, user_id));
    }

    /// Evict environments idle longer than `max_idle` (and their files).
    pub fn evict_idle(&self, max_idle: Duration) {
        let stale: Vec<i64> = self
            .map
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, env)| env.last_used.lock().unwrap().elapsed() > max_idle)
            .map(|(id, _)| *id)
            .collect();
        for user_id in stale {
            self.invalidate(user_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::MasterKey;
    use crate::db::Db;
    use serde_json::json;
    use srelens_capability::Capability;

    fn factory() -> RegistryFactory {
        Arc::new(|_cache, paths: Vec<PathBuf>| {
            let mut reg = Registry::new();
            let n = paths.len();
            reg.register(Capability::read_only(
                "paths.count",
                "how many paths",
                move |_| {
                    let n = n;
                    async move { Ok(json!({ "count": n })) }
                },
            ));
            reg
        })
    }

    fn key() -> MasterKey {
        MasterKey::from_hex(&"ab".repeat(32)).unwrap()
    }

    fn test_data_dir() -> PathBuf {
        let mut bytes = [0u8; 8];
        getrandom::getrandom(&mut bytes).unwrap();
        let dir = std::env::temp_dir().join(format!("srelens-users-test-{}", hex::encode(bytes)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn materializes_kubeconfigs_and_builds_registry() {
        let db = Db::open_in_memory().await.unwrap();
        let k = key();
        let data_dir = test_data_dir();
        let user = db.upsert_user("i", "alice", "a@x", "A", 1).await.unwrap();
        db.put_kubeconfig(user.id, "prod", &k, "apiVersion: v1", 10)
            .await
            .unwrap();

        let envs = UserEnvs::new(factory(), data_dir.clone());
        let env = envs.env_for(&db, &k, user.id).await.unwrap();
        assert_eq!(env.paths.len(), 1);
        let on_disk = std::fs::read_to_string(&env.paths[0]).unwrap();
        assert_eq!(on_disk, "apiVersion: v1");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&env.paths[0])
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let out = env
            .registry
            .invoke("paths.count", json!(null))
            .await
            .unwrap();
        assert_eq!(out, json!({ "count": 1 }));

        // The per-user stream bundle is constructed alongside the registry.
        env.streams.watch.stop("no-such-channel"); // no panic == bundle present

        // Cached: same Arc on second call.
        let env2 = envs.env_for(&db, &k, user.id).await.unwrap();
        assert!(Arc::ptr_eq(&env, &env2));

        // Invalidate removes files; next env_for rebuilds with new content.
        envs.invalidate(user.id);
        assert!(!env.paths[0].exists());
        db.put_kubeconfig(user.id, "stage", &k, "apiVersion: v1", 20)
            .await
            .unwrap();
        let env3 = envs.env_for(&db, &k, user.id).await.unwrap();
        assert_eq!(env3.paths.len(), 2);

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn users_are_isolated_and_eviction_cleans_up() {
        let db = Db::open_in_memory().await.unwrap();
        let k = key();
        let data_dir = test_data_dir();
        let alice = db.upsert_user("i", "alice", "", "", 1).await.unwrap();
        let bob = db.upsert_user("i", "bob", "", "", 1).await.unwrap();
        db.put_kubeconfig(alice.id, "a", &k, "alice-config", 1)
            .await
            .unwrap();

        let envs = UserEnvs::new(factory(), data_dir.clone());
        let a = envs.env_for(&db, &k, alice.id).await.unwrap();
        let b = envs.env_for(&db, &k, bob.id).await.unwrap();
        assert_eq!(a.paths.len(), 1);
        assert_eq!(b.paths.len(), 0);
        assert!(a.paths[0].starts_with(
            data_dir
                .join("runtime")
                .join("users")
                .join(alice.id.to_string())
        ));

        // Zero idle tolerance evicts both and removes alice's files.
        envs.evict_idle(Duration::from_secs(0));
        assert!(!a.paths[0].exists());

        let _ = std::fs::remove_dir_all(&data_dir);
    }
}
