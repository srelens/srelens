//! The real `ObjectWatcher`: bridges an MCP subscription to a single-object
//! kube watch, using the app's shared authenticated client cache.

use std::sync::Arc;

use srelens_kube::client_cache::ClientCache;
use srelens_mcp::resources::{KindResolver, KindScope, ObjectWatcher, ResourceUri, CLUSTER_SCOPED};

/// Watches single objects through the app's shared authenticated client cache,
/// so an MCP subscription sees exactly the clusters the GUI does.
pub struct CacheWatcher {
    cache: Arc<ClientCache>,
    // The same addressability decision `plan_read` uses, held internally so the
    // exclusion of unresolvable kinds (and `Secret`, which is curated out of
    // `k8s://` addressing entirely) cannot drift between the read path and the
    // watch path.
    kinds: Arc<dyn KindResolver>,
}

impl CacheWatcher {
    pub fn new(cache: Arc<ClientCache>) -> Self {
        Self { cache, kinds: srelens_registry::kind_resolver() }
    }
}

impl ObjectWatcher for CacheWatcher {
    fn watch(
        &self,
        uri: &ResourceUri,
        on_change: Box<dyn FnMut() + Send>,
        on_dead: Box<dyn FnOnce(String) + Send>,
    ) -> Result<tokio::task::AbortHandle, String> {
        let ResourceUri::Object { context, namespace, kind, name, .. } = uri else {
            return Err("only object URIs can be watched".to_string());
        };

        // `srelens_registry::kind_resolver()` returns `None` both for kinds it
        // cannot resolve at all and for `Secret`, which is deliberately not
        // addressable as a resource — it stays reachable only through the
        // consent-gated `k8s.getSecret` tool. Gating here (rather than on
        // `srelens_kube::manifest::gvk_for`, which *does* resolve `Secret`)
        // keeps that exclusion in one place instead of duplicating it.
        let scope = self.kinds.scope(kind).ok_or_else(|| {
            if kind == "Secret" {
                "`Secret` is not addressable as a resource; read secrets with the \
                 `k8s.getSecret` tool, which is consent-gated"
                    .to_string()
            } else {
                format!("kind `{kind}` is not addressable as a resource")
            }
        })?;

        // `watch_object`'s contract: `namespace: None` must mean the kind is
        // genuinely cluster-scoped, or a namespaced kind's bare
        // `metadata.name` selector would silently match same-named objects in
        // every namespace. Mirrors the equivalent check in `plan_read`
        // (`crates/mcp/src/resources.rs`) so the message is consistent
        // whichever path a caller hits.
        match (scope, namespace) {
            (KindScope::Namespaced, None) => {
                return Err(format!(
                    "`{kind}` is namespaced; supply a namespace instead of `{CLUSTER_SCOPED}`"
                ))
            }
            (KindScope::ClusterScoped, Some(ns)) => {
                return Err(format!(
                    "`{kind}` is cluster-scoped; use `{CLUSTER_SCOPED}` for the namespace, not `{ns}`"
                ))
            }
            _ => {}
        }

        // The scope check above already establishes this kind is addressable,
        // so `gvk_for` is used only to obtain the GVK `watch_object` needs.
        let (gvk, _namespaced) = srelens_kube::manifest::gvk_for(kind)
            .ok_or_else(|| format!("kind `{kind}` cannot be watched"))?;

        let cache = self.cache.clone();
        let (context, namespace, name) = (context.clone(), namespace.clone(), name.clone());
        // `watch` is synchronous and returns before the client is ever
        // resolved, so it cannot validate the subscription up front. What it
        // CAN do is report the death once the task runs: `watch_object`
        // returns only when the watch is genuinely over — client resolution
        // failed (unknown context), a permanent watch error (RBAC
        // `Forbidden`), or the stream ending; transient errors reconnect
        // internally and never return. Any return therefore feeds `on_dead`,
        // which the stdio loop uses to evict the registry entry (#195). An
        // abort cancels the task at an await point, so `on_dead` never fires
        // for a deliberate unsubscribe. Status transitions still go to
        // stderr (stdout is the JSON-RPC channel on stdio — see `main.rs`).
        let watch_uri = uri.to_string();
        let task = tokio::spawn(async move {
            let result = srelens_kube::watch::watch_object(
                cache,
                context,
                namespace,
                gvk,
                name,
                on_change,
                |status| {
                    eprintln!("srelens: mcp watch {watch_uri}: {}", status.as_str());
                },
            )
            .await;
            let reason = match result {
                Ok(()) => "the watch stream ended".to_string(),
                Err(e) => e,
            };
            on_dead(reason);
        });
        Ok(task.abort_handle())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_mcp::resources::{ObjectWatcher, ResourceUri};

    /// A cluster-scoped URI and a namespaced one must both resolve to a watch
    /// without panicking. The watch itself needs an API server, so the handle
    /// is expected to end in an error — what matters is that resolution and
    /// spawning work and nothing panics.
    #[tokio::test]
    async fn spawns_a_watch_for_both_scopes_without_panicking() {
        let cache = srelens_kube::client_cache::ClientCache::new(
            std::path::PathBuf::from("/nonexistent/kubeconfig"),
        );
        let w = CacheWatcher::new(cache);
        for uri in ["k8s://c/ns/Pod/web-0", "k8s://c/-/Node/node-1"] {
            let parsed = ResourceUri::parse(uri).unwrap();
            let handle = w.watch(&parsed, Box::new(|| {}), Box::new(|_| {})).expect("spawns");
            handle.abort();
        }
    }

    /// The #195 contract: a watch that can never run (here: the kubeconfig
    /// doesn't exist, so client resolution fails) must report through
    /// `on_dead` with the underlying reason — that callback is how the stdio
    /// loop evicts the registry entry.
    #[tokio::test]
    async fn reports_death_for_an_unresolvable_context() {
        let cache = srelens_kube::client_cache::ClientCache::new(
            std::path::PathBuf::from("/nonexistent/kubeconfig"),
        );
        let w = CacheWatcher::new(cache);
        let parsed = ResourceUri::parse("k8s://c/ns/Pod/web-0").unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let _handle = w
            .watch(
                &parsed,
                Box::new(|| {}),
                Box::new(move |reason| {
                    let _ = tx.send(reason);
                }),
            )
            .expect("spawns");
        let reason = tokio::time::timeout(std::time::Duration::from_secs(10), rx)
            .await
            .expect("on_dead fires")
            .expect("reason arrives");
        assert!(!reason.is_empty(), "the death carries its reason");
    }

    #[tokio::test]
    async fn refuses_a_kind_it_cannot_resolve() {
        let cache = srelens_kube::client_cache::ClientCache::new(
            std::path::PathBuf::from("/nonexistent/kubeconfig"),
        );
        let w = CacheWatcher::new(cache);
        let parsed = ResourceUri::parse("k8s://c/ns/Nonsense/x").unwrap();
        assert!(w.watch(&parsed, Box::new(|| {}), Box::new(|_| {})).is_err());
    }

    /// `ResourceUri::parse` maps the `-` sentinel to `None`, so a namespaced
    /// kind given `-` must be rejected rather than silently watched with
    /// `Api::all_with` — which would match same-named objects in every
    /// namespace (see `watch_object`'s documented contract).
    #[tokio::test]
    async fn refuses_a_namespaced_kind_given_no_namespace() {
        let cache = srelens_kube::client_cache::ClientCache::new(
            std::path::PathBuf::from("/nonexistent/kubeconfig"),
        );
        let w = CacheWatcher::new(cache);
        let parsed = ResourceUri::parse("k8s://c/-/Pod/web-0").unwrap();
        let err = w.watch(&parsed, Box::new(|| {}), Box::new(|_| {})).unwrap_err();
        assert!(err.contains("namespaced"), "got: {err}");
    }

    /// The mirror-image mismatch: a cluster-scoped kind given a namespace must
    /// also be rejected, not silently scoped down to that namespace.
    #[tokio::test]
    async fn refuses_a_cluster_scoped_kind_given_a_namespace() {
        let cache = srelens_kube::client_cache::ClientCache::new(
            std::path::PathBuf::from("/nonexistent/kubeconfig"),
        );
        let w = CacheWatcher::new(cache);
        let parsed = ResourceUri::parse("k8s://c/ns/Node/node-1").unwrap();
        let err = w.watch(&parsed, Box::new(|| {}), Box::new(|_| {})).unwrap_err();
        assert!(err.contains("cluster-scoped"), "got: {err}");
    }

    /// `k8s://…/Secret/…` must never be watched: notifications carry no
    /// content, but firing one still leaks the fact that a secret changed, and
    /// the design's curation decision is that Secrets are not addressable at
    /// all — not merely gated on the read path. The assertion checks the
    /// message names Secrets specifically, so this can't pass for the wrong
    /// reason (an unresolvable kind also returns `Err`).
    #[tokio::test]
    async fn refuses_to_watch_a_secret() {
        let cache = srelens_kube::client_cache::ClientCache::new(
            std::path::PathBuf::from("/nonexistent/kubeconfig"),
        );
        let w = CacheWatcher::new(cache);
        let parsed = ResourceUri::parse("k8s://c/ns/Secret/db-creds").unwrap();
        let err = w.watch(&parsed, Box::new(|| {}), Box::new(|_| {})).unwrap_err();
        assert!(err.contains("Secret") && err.contains("not addressable"), "got: {err}");
    }
}
