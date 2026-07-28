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
    /// This user's private helm home (`<runtime>/<id>/helm`): helm's repository
    /// config, cache, and plugins live here so helm state never crosses users.
    pub helm_home: PathBuf,
    pub streams: Arc<crate::streams::UserStreams>,
    /// This user's index of OIDC clusters (issuer/client per oidc_key), built
    /// from their kubeconfigs — the cluster-login routes resolve `?key=`
    /// against this same registry the auth resolver uses.
    pub oidc_registry: Arc<crate::cluster_registry::ClusterOidcRegistry>,
    last_used: Mutex<Instant>,
}

pub struct UserEnvs {
    factory: RegistryFactory,
    data_dir: PathBuf,
    /// Public base URL of this server, used to build the cluster-OIDC redirect
    /// URI (`<public_url>/auth/cluster/callback`) for the token refresh client.
    public_url: String,
    map: Mutex<HashMap<i64, Arc<UserEnv>>>,
    // Serializes concurrent first-time builds per user: two racing misses on
    // the same user must not both `remove_dir_all` + materialize the runtime
    // dir at once (the loser can delete the winner's freshly-written files).
    build_locks: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
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
    pub fn new(factory: RegistryFactory, data_dir: PathBuf, public_url: String) -> Self {
        Self {
            factory,
            data_dir,
            public_url,
            map: Mutex::new(HashMap::new()),
            build_locks: Mutex::new(HashMap::new()),
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
        key: &Arc<MasterKey>,
        user_id: i64,
    ) -> Result<Arc<UserEnv>, String> {
        if let Some(env) = self.map.lock().unwrap().get(&user_id).cloned() {
            *env.last_used.lock().unwrap() = Instant::now();
            return Ok(env);
        }

        // Cache miss: serialize the build per-user so concurrent first-time
        // callers don't race (the loser's remove_dir_all could delete the
        // winner's just-materialized files). The std Mutex guarding the lock
        // map is never held across an .await — only the tokio lock is.
        let lock = {
            let mut locks = self.build_locks.lock().unwrap();
            locks.entry(user_id).or_default().clone()
        };
        let _guard = lock.lock().await;

        // Double-check: another builder may have finished while we waited.
        if let Some(env) = self.map.lock().unwrap().get(&user_id).cloned() {
            *env.last_used.lock().unwrap() = Instant::now();
            return Ok(env);
        }

        // Build outside the map lock: decrypt + materialize + construct.
        let metas = db.list_kubeconfigs(user_id).await?;
        let dir = user_runtime_dir(&self.data_dir, user_id);
        let _ = std::fs::remove_dir_all(&dir);
        create_private_dir(&dir)?;
        // Private per-user helm home; helm creates its subdirs (config/cache/
        // data) on demand under this 0700 root.
        let helm_home = dir.join("helm");
        create_private_dir(&helm_home)?;
        let mut paths = Vec::with_capacity(metas.len());
        let mut yamls = Vec::with_capacity(metas.len());
        for meta in &metas {
            let yaml = db
                .get_kubeconfig_yaml(user_id, meta.id, key)
                .await?
                .ok_or_else(|| format!("kubeconfig {} disappeared", meta.id))?;
            let path = dir.join(format!("kc-{}.yaml", meta.id));
            write_private_file(&path, yaml.as_bytes())?;
            paths.push(path);
            yamls.push(yaml);
        }

        let cache = ClientCache::new_many(paths.clone());

        // Install the cluster-OIDC bearer resolver: for a context whose user is
        // OIDC, the cache authenticates with a srelens-managed id_token (server-
        // side refresh) instead of running a headless-broken exec plugin.
        let oidc_registry = Arc::new(
            crate::cluster_registry::ClusterOidcRegistry::from_kubeconfig_yamls(&yamls),
        );
        let refresh = crate::cluster_oidc::make_refresh_fn(
            oidc_registry.clone(),
            format!("{}/auth/cluster/callback", self.public_url),
            crate::unix_now,
        );
        let provider = Arc::new(crate::oidc_provider::OidcTokenProvider::new(
            db.clone(),
            key.clone(),
            user_id,
            refresh,
        ));
        cache
            .set_auth_resolver(Arc::new(crate::cluster_auth_resolver::ClusterAuthResolver {
                registry: oidc_registry.clone(),
                provider,
            }))
            .await;

        let registry = Arc::new((self.factory)(cache.clone(), paths.clone()));
        let streams = Arc::new(crate::streams::UserStreams::new(cache.clone()));
        let env = Arc::new(UserEnv {
            registry,
            cache,
            paths,
            helm_home,
            streams,
            oidc_registry,
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

    /// Whether the user currently has any active port-forwards. Port-forward
    /// traffic runs over plain HTTP at `/pf/{id}/*`, not the WebSocket, so a
    /// user can drop their WS connection while still actively using a
    /// forward from an external tool. Used by the WS-disconnect teardown to
    /// avoid killing those forwards out from under them.
    pub fn has_active_forwards(&self, user_id: i64) -> bool {
        self.map
            .lock()
            .unwrap()
            .get(&user_id)
            .map(|env| env.streams.forward.active_count() > 0)
            .unwrap_or(false)
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

/// After `grace`, if the user has no live WS connections AND no active
/// port-forwards, drop their cached environment (aborting all stream tasks +
/// freeing snapshots). A reconnect during the grace cancels the teardown, and
/// so does an in-progress port-forward: forwards run over plain HTTP (not the
/// WS), so a user can close their tab and keep using a forward from an
/// external tool. Such forwards are kept alive by their own traffic via the
/// idle eviction that `env_for` bumps on each `/pf` request; keeping the rest
/// of the (small) env cached alongside them is an acceptable trade.
pub async fn teardown_streams_if_disconnected(
    hub: Arc<crate::ws::hub::WsHub>,
    user_envs: Arc<UserEnvs>,
    user_id: i64,
    grace: Duration,
) {
    tokio::time::sleep(grace).await;
    if hub.user_connection_count(user_id) == 0 && !user_envs.has_active_forwards(user_id) {
        user_envs.invalidate(user_id);
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

    fn key() -> Arc<MasterKey> {
        Arc::new(MasterKey::from_hex(&"ab".repeat(32)).unwrap())
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

        let envs = UserEnvs::new(factory(), data_dir.clone(), "http://127.0.0.1:8080".into());
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

        let envs = UserEnvs::new(factory(), data_dir.clone(), "http://127.0.0.1:8080".into());
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

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_env_for_builds_once() {
        let db = Db::open_in_memory().await.unwrap();
        let k = key();
        let data_dir = test_data_dir();
        let user = db.upsert_user("i", "u", "", "", 1).await.unwrap();
        db.put_kubeconfig(user.id, "kc", &k, "contexts: []\n", 1)
            .await
            .unwrap();
        let envs = Arc::new(UserEnvs::new(factory(), data_dir.clone(), "http://127.0.0.1:8080".into()));
        // Fire many concurrent first-time env_for for the same user.
        let mut handles = vec![];
        for _ in 0..8 {
            let envs = envs.clone();
            let db = db.clone();
            let k2 = key();
            handles.push(tokio::spawn(async move {
                envs.env_for(&db, &k2, user.id).await
            }));
        }
        let mut envs_built = vec![];
        for h in handles {
            envs_built.push(h.await.unwrap().expect("env_for ok"));
        }
        // All concurrent calls resolve to the SAME env (single build, no race error).
        for e in &envs_built {
            assert!(Arc::ptr_eq(&envs_built[0], e));
        }
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn teardown_invalidates_env_when_no_ws_connections_remain() {
        let db = Db::open_in_memory().await.unwrap();
        let k = key();
        let data_dir = test_data_dir();
        let user = db.upsert_user("i", "u", "", "", 1).await.unwrap();
        db.put_kubeconfig(user.id, "kc", &k, "apiVersion: v1", 1)
            .await
            .unwrap();

        let envs = Arc::new(UserEnvs::new(factory(), data_dir.clone(), "http://127.0.0.1:8080".into()));
        let hub = Arc::new(crate::ws::hub::WsHub::new());

        let env = envs.env_for(&db, &k, user.id).await.unwrap();
        // Cached: a second lookup returns the same Arc.
        let env_again = envs.env_for(&db, &k, user.id).await.unwrap();
        assert!(Arc::ptr_eq(&env, &env_again));

        // No live connections for this user in the hub.
        assert_eq!(hub.user_connection_count(user.id), 0);

        teardown_streams_if_disconnected(hub.clone(), envs.clone(), user.id, Duration::ZERO).await;

        // Invalidated: the runtime files are gone and a fresh lookup rebuilds
        // (yielding a different Arc) instead of returning the old one.
        assert!(!env.paths[0].exists());
        let env_rebuilt = envs.env_for(&db, &k, user.id).await.unwrap();
        assert!(!Arc::ptr_eq(&env, &env_rebuilt));

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn teardown_skips_when_a_ws_connection_is_still_live() {
        let db = Db::open_in_memory().await.unwrap();
        let k = key();
        let data_dir = test_data_dir();
        let user = db.upsert_user("i", "u", "", "", 1).await.unwrap();
        db.put_kubeconfig(user.id, "kc", &k, "apiVersion: v1", 1)
            .await
            .unwrap();

        let envs = Arc::new(UserEnvs::new(factory(), data_dir.clone(), "http://127.0.0.1:8080".into()));
        let hub = Arc::new(crate::ws::hub::WsHub::new());

        // Register a live WS connection for the user before building the env.
        let (_conn_id, _rx) = hub.register(user.id);
        let env = envs.env_for(&db, &k, user.id).await.unwrap();

        assert_eq!(hub.user_connection_count(user.id), 1);
        teardown_streams_if_disconnected(hub.clone(), envs.clone(), user.id, Duration::ZERO).await;

        // Teardown was skipped: the env is still cached as the same Arc.
        let env_again = envs.env_for(&db, &k, user.id).await.unwrap();
        assert!(Arc::ptr_eq(&env, &env_again));

        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn teardown_skips_when_user_has_an_active_port_forward() {
        let db = Db::open_in_memory().await.unwrap();
        let k = key();
        let data_dir = test_data_dir();
        let user = db.upsert_user("i", "u", "", "", 1).await.unwrap();
        db.put_kubeconfig(user.id, "kc", &k, "apiVersion: v1", 1)
            .await
            .unwrap();

        let envs = Arc::new(UserEnvs::new(factory(), data_dir.clone(), "http://127.0.0.1:8080".into()));
        let hub = Arc::new(crate::ws::hub::WsHub::new());

        let env = envs.env_for(&db, &k, user.id).await.unwrap();
        env.streams.forward.insert_test_forward(1, 12345);

        // No live WS connections, but an active forward is enough to skip.
        assert_eq!(hub.user_connection_count(user.id), 0);
        assert!(envs.has_active_forwards(user.id));

        teardown_streams_if_disconnected(hub.clone(), envs.clone(), user.id, Duration::ZERO).await;

        // Teardown was skipped: the env is still cached as the same Arc.
        let env_again = envs.env_for(&db, &k, user.id).await.unwrap();
        assert!(Arc::ptr_eq(&env, &env_again));

        let _ = std::fs::remove_dir_all(&data_dir);
    }
}
