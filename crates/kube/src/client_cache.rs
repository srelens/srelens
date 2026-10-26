//! Caches authenticated kube-rs clients per context so capabilities don't
//! re-parse the kubeconfig and rebuild TLS config on every invocation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use kube::api::{Api, DynamicObject, ListParams};
use kube::core::{ApiResource, GroupVersionKind};
use kube::discovery::{Discovery, Scope};
use kube::Client;
use tokio::sync::{Mutex, RwLock};

use crate::auth_resolver::AuthResolver;
use crate::connect::build_client;

/// A discovered Kubernetes API resource, including its scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredResource {
    pub ar: ApiResource,
    pub namespaced: bool,
}

/// Fast lookup index for discovered API resources in a cluster.
#[derive(Debug, Default, Clone)]
pub struct DiscoveryIndex {
    by_name: HashMap<String, DiscoveredResource>,
    by_group_and_name: HashMap<(String, String), DiscoveredResource>,
}

impl DiscoveryIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, res: DiscoveredResource) {
        let kind_lower = res.ar.kind.to_lowercase();
        let plural_lower = res.ar.plural.to_lowercase();
        let group_lower = res.ar.group.to_lowercase();

        self.by_group_and_name
            .insert((group_lower.clone(), kind_lower.clone()), res.clone());
        self.by_group_and_name
            .insert((group_lower.clone(), plural_lower.clone()), res.clone());

        self.by_name
            .entry(kind_lower)
            .or_insert_with(|| res.clone());
        self.by_name.entry(plural_lower).or_insert_with(|| res);
    }

    pub fn insert_alias(&mut self, alias: &str, res: &DiscoveredResource) {
        let alias_lower = alias.trim().to_lowercase();
        if alias_lower.is_empty() {
            return;
        }
        let group_lower = res.ar.group.to_lowercase();
        self.by_group_and_name
            .insert((group_lower, alias_lower.clone()), res.clone());
        self.by_name
            .entry(alias_lower)
            .or_insert_with(|| res.clone());
    }

    pub fn find(&self, kind_or_plural: &str, group: Option<&str>) -> Option<DiscoveredResource> {
        let normalized = kind_or_plural.trim().to_lowercase();
        if let Some(grp) = group.filter(|g| !g.trim().is_empty()) {
            let grp_lower = grp.trim().to_lowercase();
            return self
                .by_group_and_name
                .get(&(grp_lower, normalized))
                .cloned();
        }
        self.by_name.get(&normalized).cloned()
    }

    /// Return all distinct discovered resources in this index.
    pub fn all_resources(&self) -> Vec<DiscoveredResource> {
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for res in self.by_group_and_name.values() {
            let key = (
                res.ar.group.clone(),
                res.ar.version.clone(),
                res.ar.kind.clone(),
            );
            if seen.insert(key) {
                result.push(res.clone());
            }
        }
        result
    }
}

pub struct ClientCache {
    paths: RwLock<Vec<PathBuf>>,
    clients: Mutex<HashMap<String, (Client, Option<String>)>>,
    discovery: Mutex<HashMap<String, Arc<DiscoveryIndex>>>,
    auth_resolver: RwLock<Option<Arc<dyn AuthResolver>>>,
    generation: AtomicU64,
    context_generations: Mutex<HashMap<String, u64>>,
}

impl ClientCache {
    pub fn new(path: PathBuf) -> Arc<Self> {
        Self::new_many(vec![path])
    }

    pub fn new_many(paths: Vec<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            paths: RwLock::new(paths),
            clients: Mutex::new(HashMap::new()),
            discovery: Mutex::new(HashMap::new()),
            auth_resolver: RwLock::new(None),
            generation: AtomicU64::new(0),
            context_generations: Mutex::new(HashMap::new()),
        })
    }

    /// Read the current generation for a specific context.
    async fn context_generation(&self, context: &str) -> u64 {
        self.context_generations
            .lock()
            .await
            .get(context)
            .copied()
            .unwrap_or(0)
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
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.context_generations.lock().await.clear();
        self.clients.lock().await.clear();
        self.discovery.lock().await.clear();
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
        let start_global_gen = self.generation.load(Ordering::SeqCst);
        let start_ctx_gen = self.context_generation(context).await;
        // Resolve desired auth first. Cheap for non-OIDC (in-memory lookup);
        // for OIDC this consults the token provider (and may refresh, single-
        // flighted). A needs-login propagates out as the marker String error.
        let resolver = self.auth_resolver.read().await.clone();
        let auth_context =
            if resolver.is_some() && crate::context_resolve::is_pinned_context(context) {
                crate::context_resolve::resolve_context(&self.paths().await, context)
                    .ok_or_else(|| "Pinned context is unavailable or ambiguous".to_string())?
                    .original_name
            } else {
                context.to_owned()
            };
        let want_bearer: Option<String> = match resolver {
            Some(r) => match r.resolve(&auth_context).await? {
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
        let mut lock = self.clients.lock().await;
        let curr_ctx_gen = self.context_generation(context).await;
        if self.generation.load(Ordering::SeqCst) == start_global_gen
            && curr_ctx_gen == start_ctx_gen
        {
            lock.insert(context.to_string(), (client.clone(), want_bearer));
        }
        Ok(client)
    }

    /// Test-only: hand the cache a ready-made client for `context`, so a
    /// capability handler can be driven end to end against a fake API server
    /// without a kubeconfig. Stored with no bearer, the desktop shape. Other
    /// crates' tests reach it through the `test-support` feature (#728).
    #[cfg(any(test, feature = "test-support"))]
    pub async fn preload(&self, context: &str, client: Client) {
        self.clients
            .lock()
            .await
            .insert(context.to_string(), (client, None));
    }

    /// Test-only: hand the cache a ready-made discovery index for `context`.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn preload_discovery(&self, context: &str, index: DiscoveryIndex) {
        self.discovery
            .lock()
            .await
            .insert(context.to_string(), Arc::new(index));
    }

    /// Drop any cached client or discovery for a context (e.g. after a connection failure).
    pub async fn invalidate(&self, context: &str) {
        {
            let mut gens = self.context_generations.lock().await;
            let current = gens.get(context).copied().unwrap_or(0);
            gens.insert(context.to_string(), current + 1);
        }
        self.clients.lock().await.remove(context);
        self.discovery.lock().await.remove(context);
    }

    /// Clear all cached clients and discovery (e.g. after a kubeconfig change).
    pub async fn clear(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.context_generations.lock().await.clear();
        self.clients.lock().await.clear();
        self.discovery.lock().await.clear();
    }

    /// Discover all available API resources in the cluster (aggregated discovery
    /// with fallback to standard discovery, plus CRD inspection for aliases and short names).
    async fn discover_cluster(&self, context: &str) -> Result<Arc<DiscoveryIndex>, String> {
        let start_global_gen = self.generation.load(Ordering::SeqCst);
        let start_ctx_gen = self.context_generation(context).await;
        let client = self.get(context).await?;
        let mut index = DiscoveryIndex::new();

        // 1. Try aggregated discovery, fallback to regular discovery, bounded by request_timeout
        let timeout = crate::connect::request_timeout();
        let disc_result =
            tokio::time::timeout(timeout, Discovery::new(client.clone()).run_aggregated()).await;
        let disc = match disc_result {
            Ok(Ok(d)) => Some(d),
            _ => tokio::time::timeout(timeout, Discovery::new(client.clone()).run())
                .await
                .ok()
                .and_then(|r| r.ok()),
        };

        let mut discovered_any = false;
        if let Some(disc) = disc {
            for group in disc.groups() {
                for (ar, caps) in group.recommended_resources() {
                    let namespaced = caps.scope == Scope::Namespaced;
                    index.insert(DiscoveredResource {
                        ar: ar.clone(),
                        namespaced,
                    });
                    discovered_any = true;
                }
            }
        }

        // 2. Discover CRDs to enrich with shortNames, singular names, and custom resources
        let crd_gvk =
            GroupVersionKind::gvk("apiextensions.k8s.io", "v1", "CustomResourceDefinition");
        let crd_ar = ApiResource::from_gvk(&crd_gvk);
        let crd_api: Api<DynamicObject> = Api::all_with(client.clone(), &crd_ar);
        if let Ok(Ok(crd_list)) = tokio::time::timeout(
            crate::connect::request_timeout(),
            crd_api.list(&ListParams::default()),
        )
        .await
        {
            for o in crd_list.items {
                let spec = &o.data["spec"];
                let group = spec["group"].as_str().unwrap_or_default().to_string();
                let kind = spec["names"]["kind"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let plural = spec["names"]["plural"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let singular = spec["names"]["singular"].as_str().unwrap_or_default();
                let namespaced = spec["scope"].as_str().unwrap_or("Namespaced") == "Namespaced";
                let version = crate::crds::pick_version(spec);
                if group.is_empty() || kind.is_empty() || plural.is_empty() || version.is_empty() {
                    continue;
                }
                let res = DiscoveredResource {
                    ar: crate::crds::custom_api_resource(&group, &version, &kind, &plural),
                    namespaced,
                };
                index.insert(res.clone());
                if !singular.is_empty() {
                    index.insert_alias(singular, &res);
                }
                if let Some(short_names) = spec["names"]["shortNames"].as_array() {
                    for sn in short_names.iter().filter_map(|s| s.as_str()) {
                        index.insert_alias(sn, &res);
                    }
                }
                discovered_any = true;
            }
        }

        if !discovered_any {
            return Err(format!(
                "failed to discover API resources from cluster for context '{context}'"
            ));
        }

        let arc_index = Arc::new(index);
        let mut lock = self.discovery.lock().await;
        let curr_ctx_gen = self.context_generation(context).await;
        if self.generation.load(Ordering::SeqCst) != start_global_gen
            || curr_ctx_gen != start_ctx_gen
        {
            return Err(format!(
                "discovery for context '{context}' invalidated by cache reset"
            ));
        }
        lock.insert(context.to_string(), arc_index.clone());
        Ok(arc_index)
    }

    /// Retrieve the discovery index for `context`, returning cached discovery if available
    /// or discovering cluster resources on demand.
    pub async fn get_discovery(&self, context: &str) -> Result<Arc<DiscoveryIndex>, String> {
        {
            let lock = self.discovery.lock().await;
            if let Some(idx) = lock.get(context) {
                return Ok(idx.clone());
            }
        }
        self.discover_cluster(context).await
    }

    /// Resolve an arbitrary Kubernetes kind/plural/alias to its `(ApiResource, namespaced)`.
    /// 1. If explicit group, version, and plural are provided, build custom ApiResource directly.
    /// 2. If group/version/plural are omitted and kind matches built-in static table, resolve via `gvk_for`.
    /// 3. Otherwise, look up in cached discovery index for `context`.
    /// 4. On cache miss, run dynamic discovery against the cluster and populate the cache.
    pub async fn resolve_resource(
        &self,
        context: &str,
        kind: &str,
        group: Option<&str>,
        version: Option<&str>,
        plural: Option<&str>,
    ) -> Result<(ApiResource, bool), String> {
        // Tier 1: Caller provided explicit GVK + plural hints
        if let (Some(g), Some(v), Some(p)) = (group, version, plural) {
            let namespaced = {
                let cached = {
                    let lock = self.discovery.lock().await;
                    lock.get(context)
                        .and_then(|idx| idx.find(kind, Some(g)))
                        .map(|r| r.namespaced)
                };
                if let Some(ns) = cached {
                    ns
                } else if let Ok(idx) = self.discover_cluster(context).await {
                    idx.find(kind, Some(g))
                        .map(|r| r.namespaced)
                        .unwrap_or(true)
                } else {
                    true
                }
            };
            return Ok((crate::crds::custom_api_resource(g, v, kind, p), namespaced));
        }

        // Tier 2: Static built-ins when group/version are omitted or match
        if version.is_none() && plural.is_none() {
            if let Some((gvk, namespaced)) = crate::manifest::gvk_for(kind) {
                if group.map_or(true, |g| g.is_empty() || g == gvk.group) {
                    return Ok((ApiResource::from_gvk(&gvk), namespaced));
                }
            }
        }

        // Tier 3: Check cached discovery index
        {
            let lock = self.discovery.lock().await;
            if let Some(idx) = lock.get(context) {
                if let Some(res) = idx.find(kind, group) {
                    if let Some(v) = version {
                        let mut ar = res.ar.clone();
                        ar.version = v.to_string();
                        return Ok((ar, res.namespaced));
                    }
                    return Ok((res.ar, res.namespaced));
                }
            }
        }

        // Tier 4: Cache miss -> query discovery from the cluster
        let idx = self.discover_cluster(context).await?;
        if let Some(res) = idx.find(kind, group) {
            if let Some(v) = version {
                let mut ar = res.ar.clone();
                ar.version = v.to_string();
                return Ok((ar, res.namespaced));
            }
            Ok((res.ar, res.namespaced))
        } else {
            Err(format!(
                "resource kind '{kind}' is not served by cluster '{context}'"
            ))
        }
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

    #[tokio::test]
    async fn pinned_requests_use_the_original_name_for_managed_auth() {
        let path = write_temp_kubeconfig(
            "pinned-auth",
            "contexts:\n- name: prod\n  context: {cluster: c, user: u}\n",
        );
        let pinned = crate::context_resolve::resolve_context(&[path.clone()], "prod")
            .unwrap()
            .pinned_id()
            .unwrap();
        let cache = ClientCache::new(path.clone());
        cache
            .set_auth_resolver(Arc::new(NeedsLoginResolver { key: "oidc".into() }))
            .await;
        let error = cache.get(&pinned).await.err().expect("login required");
        assert_eq!(error, needs_login_marker("oidc", "prod"));
        std::fs::remove_file(path).unwrap();
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

        cache
            .get("ctx-a")
            .await
            .expect("second get rebuilds with t2");
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

    #[tokio::test]
    async fn resolve_resource_fast_path_built_in() {
        let cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let (ar, namespaced) = cache
            .resolve_resource("ctx", "Pod", None, None, None)
            .await
            .expect("resolves built-in Pod");
        assert_eq!(ar.kind, "Pod");
        assert_eq!(ar.group, "");
        assert_eq!(ar.version, "v1");
        assert!(namespaced);

        let (node_ar, node_ns) = cache
            .resolve_resource("ctx", "Node", None, None, None)
            .await
            .expect("resolves built-in Node");
        assert_eq!(node_ar.kind, "Node");
        assert!(!node_ns);
    }

    #[tokio::test]
    async fn resolve_resource_with_explicit_hints() {
        let cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let (ar, namespaced) = cache
            .resolve_resource(
                "ctx",
                "CustomWidget",
                Some("widgets.io"),
                Some("v1"),
                Some("customwidgets"),
            )
            .await
            .expect("resolves with explicit hints");
        assert_eq!(ar.kind, "CustomWidget");
        assert_eq!(ar.group, "widgets.io");
        assert_eq!(ar.version, "v1");
        assert_eq!(ar.plural, "customwidgets");
        assert!(namespaced);
    }

    #[tokio::test]
    async fn resolve_resource_from_preloaded_discovery_index() {
        let cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let mut index = DiscoveryIndex::new();

        let vm_res = DiscoveredResource {
            ar: crate::crds::custom_api_resource(
                "kubevirt.io",
                "v1",
                "VirtualMachine",
                "virtualmachines",
            ),
            namespaced: true,
        };
        index.insert(vm_res.clone());
        index.insert_alias("vm", &vm_res);
        index.insert_alias("vms", &vm_res);

        cache.preload_discovery("cluster-a", index).await;

        // 1. By exact Kind
        let (ar1, ns1) = cache
            .resolve_resource("cluster-a", "VirtualMachine", None, None, None)
            .await
            .expect("resolves exact Kind");
        assert_eq!(ar1.kind, "VirtualMachine");
        assert_eq!(ar1.group, "kubevirt.io");
        assert!(ns1);

        // 2. By lowercase plural
        let (ar2, _) = cache
            .resolve_resource("cluster-a", "virtualmachines", None, None, None)
            .await
            .expect("resolves plural");
        assert_eq!(ar2.kind, "VirtualMachine");

        // 3. By short name alias
        let (ar3, _) = cache
            .resolve_resource("cluster-a", "vm", None, None, None)
            .await
            .expect("resolves short name");
        assert_eq!(ar3.kind, "VirtualMachine");

        // 4. Case-insensitivity check
        let (ar4, _) = cache
            .resolve_resource("cluster-a", "VM", None, None, None)
            .await
            .expect("resolves case-insensitively");
        assert_eq!(ar4.kind, "VirtualMachine");
    }

    #[tokio::test]
    async fn resolve_resource_group_disambiguation() {
        let cache = ClientCache::new(PathBuf::from("/nonexistent"));
        let mut index = DiscoveryIndex::new();

        let calico_bgp = DiscoveredResource {
            ar: crate::crds::custom_api_resource(
                "crd.projectcalico.org",
                "v1",
                "BGPPeer",
                "bgppeers",
            ),
            namespaced: false,
        };
        let metallb_bgp = DiscoveredResource {
            ar: crate::crds::custom_api_resource("metallb.io", "v1beta2", "BGPPeer", "bgppeers"),
            namespaced: true,
        };
        index.insert(calico_bgp);
        index.insert(metallb_bgp);

        cache.preload_discovery("cluster-a", index).await;

        let (ar_calico, ns_calico) = cache
            .resolve_resource(
                "cluster-a",
                "BGPPeer",
                Some("crd.projectcalico.org"),
                None,
                None,
            )
            .await
            .expect("resolves Calico BGPPeer");
        assert_eq!(ar_calico.group, "crd.projectcalico.org");
        assert!(!ns_calico);

        let (ar_metallb, ns_metallb) = cache
            .resolve_resource("cluster-a", "BGPPeer", Some("metallb.io"), None, None)
            .await
            .expect("resolves MetalLB BGPPeer");
        assert_eq!(ar_metallb.group, "metallb.io");
        assert!(ns_metallb);
    }

    #[tokio::test]
    async fn cache_generation_increments_on_global_resets() {
        let cache = ClientCache::new(PathBuf::from("/nonexistent"));
        assert_eq!(cache.generation.load(Ordering::SeqCst), 0);

        cache.set_paths(vec![PathBuf::from("/other")]).await;
        assert_eq!(cache.generation.load(Ordering::SeqCst), 1);

        cache.clear().await;
        assert_eq!(cache.generation.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn context_invalidation_does_not_affect_unrelated_contexts() {
        let cache = ClientCache::new(PathBuf::from("/nonexistent"));
        assert_eq!(cache.generation.load(Ordering::SeqCst), 0);
        assert_eq!(cache.context_generation("cluster-a").await, 0);
        assert_eq!(cache.context_generation("cluster-b").await, 0);

        // Invalidate cluster-a
        cache.invalidate("cluster-a").await;

        // Global generation must NOT change (unrelated cluster-b discovery is not disrupted)
        assert_eq!(cache.generation.load(Ordering::SeqCst), 0);
        // cluster-a generation increments
        assert_eq!(cache.context_generation("cluster-a").await, 1);
        // cluster-b generation remains unchanged
        assert_eq!(cache.context_generation("cluster-b").await, 0);

        // Whole-cache reset bumps global generation and clears context generations
        cache.clear().await;
        assert_eq!(cache.generation.load(Ordering::SeqCst), 1);
        assert_eq!(cache.context_generation("cluster-a").await, 0);
        assert_eq!(cache.context_generation("cluster-b").await, 0);
    }
}
