//! Live-watch core: spawns kube-rs resource watches and pushes full sorted
//! snapshots to an EventSink on the caller-provided channel.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use srelens_kube::client_cache::ClientCache;
use tokio::task::JoinHandle;

use crate::sink::EventSink;

/// Owns the running watch tasks (keyed by channel).
pub struct WatchManager {
    cache: Arc<ClientCache>,
    tasks: Mutex<HashMap<String, JoinHandle<()>>>,
}

/// Expands to the per-kind dispatch: every arm clones the sink/channel for the
/// rows and status callbacks and awaits the given watch function. Each watch
/// emits either a snapshot (JSON array) or a `{status}` object on the same
/// channel; subscribers distinguish by shape.
macro_rules! dispatch_watch {
    ($kind:expr, $cache:expr, $context:expr, $namespace:expr, $sink:expr, $channel:expr;
     $($name:literal => $watch_fn:path),+ $(,)?) => {
        match $kind.as_str() {
            $(
                $name => {
                    let (rows_sink, rows_ch) = ($sink.clone(), $channel.clone());
                    let (st_sink, st_ch) = ($sink.clone(), $channel.clone());
                    $watch_fn(
                        $cache,
                        $context,
                        $namespace,
                        move |rows| {
                            if let Ok(v) = serde_json::to_value(rows) {
                                rows_sink.emit(&rows_ch, v);
                            }
                        },
                        move |st: srelens_kube::watch::WatchStatus| {
                            st_sink.emit(&st_ch, serde_json::json!({ "status": st.as_str() }));
                        },
                    )
                    .await
                }
            )+
            other => Err(format!("kind not watchable: {other}")),
        }
    };
}

impl WatchManager {
    pub fn new(cache: Arc<ClientCache>) -> Self {
        Self {
            cache,
            tasks: Mutex::new(HashMap::new()),
        }
    }

    /// Stop a running watch by its channel.
    pub fn stop(&self, channel: &str) {
        if let Some(handle) = self.tasks.lock().unwrap().remove(channel) {
            handle.abort();
        }
    }

    /// Start watching a watchable resource kind in a namespace, emitting each
    /// full sorted snapshot on `channel`. The subscriber attaches to `channel`
    /// first, then calls this, so the initial snapshot can't race the listener.
    pub async fn start(
        &self,
        sink: Arc<dyn EventSink>,
        context: String,
        namespace: String,
        kind: String,
        channel: String,
        kubeconfig_paths: Vec<PathBuf>,
    ) -> Result<String, String> {
        self.stop(&channel);

        // The watched context may live in a pasted/added kubeconfig the cache
        // hasn't been told about yet (its `set_paths` can race this call).
        // Register those paths WITHOUT clearing cached clients so the watch
        // can resolve the context instead of failing with "failed to load
        // current context".
        if !kubeconfig_paths.is_empty() {
            self.cache.ensure_paths(kubeconfig_paths).await;
        }

        let cache = self.cache.clone();
        let emit_channel = channel.clone();

        let handle = tokio::spawn(async move {
            let result = dispatch_watch!(
                kind, cache, context, namespace, sink, emit_channel;
                "pods" => srelens_kube::watch::watch_pods,
                "deployments" => srelens_kube::watch::watch_deployments,
                "statefulsets" => srelens_kube::watch::watch_statefulsets,
                "daemonsets" => srelens_kube::watch::watch_daemonsets,
                "jobs" => srelens_kube::watch::watch_jobs,
                "cronjobs" => srelens_kube::watch::watch_cronjobs,
                "configmaps" => srelens_kube::watch::watch_configmaps,
                "secrets" => srelens_kube::watch::watch_secrets,
                "resourcequotas" => srelens_kube::watch::watch_resourcequotas,
                "limitranges" => srelens_kube::watch::watch_limitranges,
                "services" => srelens_kube::watch::watch_services,
                "ingresses" => srelens_kube::watch::watch_ingresses,
                "endpointslices" => srelens_kube::watch::watch_endpointslices,
                "networkpolicies" => srelens_kube::watch::watch_networkpolicies,
                "persistentvolumeclaims" => srelens_kube::watch::watch_pvcs,
                "persistentvolumes" => srelens_kube::watch::watch_persistentvolumes,
                "storageclasses" => srelens_kube::watch::watch_storageclasses,
                "serviceaccounts" => srelens_kube::watch::watch_serviceaccounts,
                "roles" => srelens_kube::watch::watch_roles,
                "clusterroles" => srelens_kube::watch::watch_clusterroles,
                "rolebindings" => srelens_kube::watch::watch_rolebindings,
                "clusterrolebindings" => srelens_kube::watch::watch_clusterrolebindings,
                "events" => srelens_kube::watch::watch_events,
            );
            if let Err(msg) = result {
                eprintln!("resource watch error: {msg}");
                // Surface the failure on the same channel so a permanent
                // (403/401) error stops the perpetual "Loading" state.
                sink.emit(&emit_channel, serde_json::json!({ "error": msg }));
            }
        });

        self.tasks.lock().unwrap().insert(channel.clone(), handle);
        Ok(channel)
    }

    /// Abort every running watch (used when a user's environment is dropped).
    pub fn shutdown_all(&self) {
        let mut tasks = self.tasks.lock().unwrap();
        for (_, handle) in tasks.drain() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TestSink;

    #[tokio::test(flavor = "multi_thread")]
    async fn unwatchable_kind_emits_error_on_channel() {
        let manager = WatchManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        let channel = manager
            .start(
                sink.clone(),
                "ctx".into(),
                "ns".into(),
                "bogus".into(),
                "watch:1".into(),
                vec![],
            )
            .await
            .expect("start returns the channel");
        assert_eq!(channel, "watch:1");

        for _ in 0..50 {
            if sink
                .payloads_for("watch:1")
                .iter()
                .any(|v| v["error"] == serde_json::json!("kind not watchable: bogus"))
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("error event never arrived on the sink");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_all_stops_watches() {
        let manager = WatchManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        manager
            .start(
                sink,
                "c".into(),
                "n".into(),
                "pods".into(),
                "w".into(),
                vec![],
            )
            .await
            .unwrap();
        manager.shutdown_all(); // no panic; subsequent stop is a no-op
        manager.stop("w");
    }
}
