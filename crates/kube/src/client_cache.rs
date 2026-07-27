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
    clients: Mutex<HashMap<String, (Client, Option<String>)>>,
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

    /// Return a client for `context`. With an auth resolver installed (web
    /// mode) the desired auth is resolved first; a cached client is reused only
    /// while its bearer is unchanged, so a refreshed OIDC token is picked up.
    /// Without a resolver (desktop) this is the original miss-builds-and-caches
    /// behavior, with `None` as the bearer.
    pub async fn get(&self, context: &str) -> Result<Client, String> {
        // Resolve desired auth first. Cheap for non-OIDC (in-memory lookup);
        // for OIDC this consults the token provider (and may refresh, single-
        // flighted). A needs-login propagates out as the marker String error.
        let resolver = self.auth_resolver.read().await.clone();
        let want_bearer: Option<String> = match resolver {
            Some(r) => match r.resolve(context).await? {
                crate::auth_resolver::AuthMode::Bearer(tok) => Some(tok),
                crate::auth_resolver::AuthMode::Default => None,
            },
            None => None,
        };

        // Cache hit only if the cached client was built with the same bearer.
        if let Some((client, cached)) = self.clients.lock().await.get(context) {
            if *cached == want_bearer {
                return Ok(client.clone());
            }
        }

        let paths = self.paths().await;
        let client = match &want_bearer {
            Some(tok) => crate::connect::build_client_with_bearer(&paths, context, tok).await?,
            None => build_client(&paths, context).await?,
        };
        self.clients
            .lock()
            .await
            .insert(context.to_string(), (client.clone(), want_bearer));
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

    /// Returns each token in turn (then repeats the last), counting calls — to
    /// simulate a bearer that changes after a token refresh.
    struct SequenceResolver {
        tokens: Vec<String>,
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl AuthResolver for SequenceResolver {
        async fn resolve(&self, _context: &str) -> Result<AuthMode, String> {
            let i = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let tok = self
                .tokens
                .get(i)
                .or_else(|| self.tokens.last())
                .expect("at least one token")
                .clone();
            Ok(AuthMode::Bearer(tok))
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
    async fn get_rebuilds_when_bearer_changes() {
        // A cached OIDC client must not outlive its bearer: once the resolver
        // reports a new token (a refresh happened), get() rebuilds rather than
        // serving the stale client.
        let path = write_temp_kubeconfig(
            "rotate",
            "apiVersion: v1\nkind: Config\ncurrent-context: ctx-a\nclusters:\n  - name: c\n    cluster: { server: https://a.example:6443 }\nusers:\n  - name: u\n    user:\n      auth-provider:\n        name: oidc\n        config: {}\ncontexts:\n  - name: ctx-a\n    context: { cluster: c, user: u }\n",
        );
        let cache = ClientCache::new(path.clone());
        let resolver = Arc::new(SequenceResolver {
            tokens: vec!["t1".to_string(), "t2".to_string()],
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        cache.set_auth_resolver(resolver.clone()).await;

        cache.get("ctx-a").await.expect("first get builds with t1");
        assert_eq!(
            cache.clients.lock().await.get("ctx-a").unwrap().1,
            Some("t1".to_string()),
        );

        cache.get("ctx-a").await.expect("second get rebuilds with t2");
        assert_eq!(
            cache.clients.lock().await.get("ctx-a").unwrap().1,
            Some("t2".to_string()),
            "cached bearer must be replaced, proving a rebuild not a stale hit",
        );
        assert_eq!(
            resolver.calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "resolver is consulted on every get, not only on a miss",
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_resolver_caches_and_does_not_rebuild() {
        // Desktop parity: with no resolver installed, the first get builds and
        // caches (bearer None) and a subsequent get is served from cache without
        // touching the kubeconfig again — proven by deleting the file first.
        let path = write_temp_kubeconfig(
            "parity",
            "apiVersion: v1\nkind: Config\ncurrent-context: ctx-a\nclusters:\n  - name: c\n    cluster: { server: https://a.example:6443 }\nusers:\n  - name: u\n    user: { token: static-abc }\ncontexts:\n  - name: ctx-a\n    context: { cluster: c, user: u }\n",
        );
        let cache = ClientCache::new(path.clone());
        cache.get("ctx-a").await.expect("first get builds");
        assert_eq!(cache.clients.lock().await.get("ctx-a").unwrap().1, None);

        // Remove the kubeconfig; a cache hit must not need to re-read it.
        std::fs::remove_file(&path).unwrap();
        cache
            .get("ctx-a")
            .await
            .expect("second get is served from cache, no rebuild");
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
