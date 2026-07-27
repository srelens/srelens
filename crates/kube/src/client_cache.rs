//! Caches authenticated kube-rs clients per context so capabilities don't
//! re-parse the kubeconfig and rebuild TLS config on every invocation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use kube::Client;
use tokio::sync::{Mutex, RwLock};

use crate::auth_resolver::AuthResolver;
use crate::connect::build_client;

pub struct ClientCache {
    paths: RwLock<Vec<PathBuf>>,
    clients: Mutex<HashMap<String, Client>>,
    auth_resolver: RwLock<Option<Arc<dyn AuthResolver>>>,
}

impl ClientCache {
    pub fn new(path: PathBuf) -> Arc<Self> {
        Self::new_many(vec![path])
    }

    pub fn new_many(paths: Vec<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            paths: RwLock::new(paths),
            clients: Mutex::new(HashMap::new()),
            auth_resolver: RwLock::new(None),
        })
    }

    /// Install a per-context auth resolver (web mode). None = kube-rs default.
    pub async fn set_auth_resolver(&self, resolver: Arc<dyn AuthResolver>) {
        *self.auth_resolver.write().await = Some(resolver);
    }

    pub async fn set_paths(&self, paths: Vec<PathBuf>) {
        let mut current = self.paths.write().await;
        if *current == paths {
            return;
        }
        *current = paths;
        drop(current);
        self.clients.lock().await.clear();
    }

    /// Add any kubeconfig paths not already present, WITHOUT clearing cached
    /// clients (adding a path can't invalidate an existing client). Used by
    /// operations that know about additional files (e.g. a resource watch for a
    /// context that lives in a pasted/added kubeconfig) so they can't race the
    /// app's initial `set_paths`.
    pub async fn ensure_paths(&self, additional: Vec<PathBuf>) {
        let mut current = self.paths.write().await;
        for p in additional {
            if !current.contains(&p) {
                current.push(p);
            }
        }
    }

    pub async fn paths(&self) -> Vec<PathBuf> {
        self.paths.read().await.clone()
    }

    /// Return a cached client for `context`, building and caching one on a miss.
    pub async fn get(&self, context: &str) -> Result<Client, String> {
        if let Some(client) = self.clients.lock().await.get(context).cloned() {
            return Ok(client);
        }
        let paths = self.paths().await;
        let client = match self.auth_resolver.read().await.clone() {
            Some(resolver) => match resolver.resolve(context).await? {
                crate::auth_resolver::AuthMode::Bearer(bearer) => {
                    crate::connect::build_client_with_bearer(&paths, context, &bearer).await?
                }
                crate::auth_resolver::AuthMode::Default => build_client(&paths, context).await?,
            },
            None => build_client(&paths, context).await?,
        };
        self.clients
            .lock()
            .await
            .insert(context.to_string(), client.clone());
        Ok(client)
    }

    /// Drop any cached client for a context (e.g. after a connection failure).
    pub async fn invalidate(&self, context: &str) {
        self.clients.lock().await.remove(context);
    }

    /// Clear all cached clients (e.g. after a kubeconfig change).
    pub async fn clear(&self) {
        self.clients.lock().await.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_resolver::{needs_login_marker, AuthMode};

    struct FakeResolver(AuthMode);

    impl FakeResolver {
        fn bearer(token: &str) -> Self {
            Self(AuthMode::Bearer(token.to_string()))
        }
    }

    #[async_trait::async_trait]
    impl AuthResolver for FakeResolver {
        async fn resolve(&self, _context: &str) -> Result<AuthMode, String> {
            match &self.0 {
                AuthMode::Bearer(t) => Ok(AuthMode::Bearer(t.clone())),
                AuthMode::Default => Ok(AuthMode::Default),
            }
        }
    }

    struct NeedsLoginResolver {
        key: String,
    }

    #[async_trait::async_trait]
    impl AuthResolver for NeedsLoginResolver {
        async fn resolve(&self, context: &str) -> Result<AuthMode, String> {
            Err(needs_login_marker(&self.key, context))
        }
    }

    /// Write `contents` to a unique temp file (pid + nanos) and return its path.
    fn write_temp_kubeconfig(tag: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "srelens-client-cache-test-{tag}-{}-{}.yaml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_builds_via_bearer_path_when_resolver_returns_bearer() {
        let path = write_temp_kubeconfig(
            "bearer",
            "apiVersion: v1\nkind: Config\ncurrent-context: ctx-a\nclusters:\n  - name: c\n    cluster: { server: https://a.example:6443 }\nusers:\n  - name: u\n    user:\n      auth-provider:\n        name: oidc\n        config: {}\ncontexts:\n  - name: ctx-a\n    context: { cluster: c, user: u }\n",
        );

        let cache = ClientCache::new(path.clone());
        cache
            .set_auth_resolver(Arc::new(FakeResolver::bearer("test-token")))
            .await;

        let result = cache.get("ctx-a").await;
        if let Err(e) = &result {
            panic!("expected Ok via bearer path, got Err({e})");
        }

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_surfaces_needs_login_marker_from_resolver_error() {
        let cache = ClientCache::new(PathBuf::from("/nonexistent"));
        cache
            .set_auth_resolver(Arc::new(NeedsLoginResolver {
                key: "abc".to_string(),
            }))
            .await;

        let err = match cache.get("prod").await {
            Err(e) => e,
            Ok(_) => panic!("expected Err(needs_login_marker), got Ok"),
        };
        assert_eq!(err, needs_login_marker("abc", "prod"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_errors_for_unknown_context() {
        let dir = std::env::temp_dir();
        let path = dir.join("srelens-cache-test-kubeconfig.yaml");
        tokio::fs::write(
            &path,
            "clusters:\n  - name: a\n    cluster: { server: https://127.0.0.1:1 }\ncontexts:\n  - name: ctx-a\n    context: { cluster: a }\n",
        )
        .await
        .unwrap();

        let cache = ClientCache::new(path.clone());
        assert!(cache.get("does-not-exist").await.is_err());
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn invalidate_is_safe_on_empty_cache() {
        let cache = ClientCache::new(PathBuf::from("/x"));
        cache.invalidate("nope").await; // must not panic
    }

    #[tokio::test]
    async fn ensure_paths_adds_missing_without_duplicating_or_reordering() {
        let a = PathBuf::from("/a");
        let b = PathBuf::from("/b");
        let cache = ClientCache::new(a.clone());
        // `b` is new so it's appended; `a` already present so it's not
        // duplicated and the existing order (a first) is preserved.
        cache.ensure_paths(vec![b.clone(), a.clone()]).await;
        assert_eq!(cache.paths().await, vec![a.clone(), b.clone()]);

        // Ensuring an already-present set is a no-op on order/contents.
        cache.ensure_paths(vec![a.clone(), b.clone()]).await;
        assert_eq!(cache.paths().await, vec![a, b]);
    }
}
