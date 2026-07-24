//! Port-forward core: binds a local loopback port, pipes it to a pod (or a
//! service's backing pod) via kube-rs, and tracks the running forwards.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use srelens_kube::client_cache::ClientCache;
use srelens_kube::forward;
use tokio::task::JoinHandle;

use crate::sink::EventSink;

struct Forward {
    handle: JoinHandle<()>,
}

/// Owns running port-forwards (keyed by numeric id).
pub struct ForwardManager {
    cache: Arc<ClientCache>,
    next_id: AtomicU64,
    forwards: Mutex<HashMap<u64, Forward>>,
}

/// What `start` returns: the forward's id and the actual local port it bound
/// to (the OS picks one when the caller passes no preference).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardInfo {
    pub id: u64,
    pub local_port: u16,
}

impl ForwardManager {
    pub fn new(cache: Arc<ClientCache>) -> Self {
        Self {
            cache,
            next_id: AtomicU64::new(1),
            forwards: Mutex::new(HashMap::new()),
        }
    }

    /// Start forwarding a local port to a Pod or Service. `kind` is "Pod" or
    /// "Service"; a Service is resolved to a backing pod and target port
    /// first. Returns the id + bound local port; a `forward:closed:<id>`
    /// event fires (with an optional error string) if the forward loop ends
    /// on its own.
    pub async fn start(
        &self,
        sink: Arc<dyn EventSink>,
        context: String,
        namespace: String,
        kind: String,
        name: String,
        remote_port: u16,
        local_port: Option<u16>,
    ) -> Result<ForwardInfo, String> {
        let cache = self.cache.clone();

        // Resolve a Service down to a concrete pod + container port.
        let (pod, target_port) = if kind.eq_ignore_ascii_case("service") {
            forward::resolve_service_target(
                cache.clone(),
                &context,
                &namespace,
                &name,
                Some(i32::from(remote_port)),
            )
            .await?
        } else {
            (name, remote_port)
        };

        let listener = forward::bind_local(local_port.unwrap_or(0)).await?;
        let bound = listener.local_addr().map_err(|e| e.to_string())?.port();

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let closed_channel = format!("forward:closed:{id}");
        let handle = tokio::spawn(async move {
            let result =
                forward::serve_pod_forward(listener, cache, context, namespace, pod, target_port)
                    .await;
            sink.emit(
                &closed_channel,
                serde_json::to_value(result.err()).unwrap_or(serde_json::Value::Null),
            );
        });

        self.forwards.lock().unwrap().insert(id, Forward { handle });
        Ok(ForwardInfo {
            id,
            local_port: bound,
        })
    }

    /// Stop a port-forward and abort its task.
    pub fn stop(&self, id: u64) {
        if let Some(f) = self.forwards.lock().unwrap().remove(&id) {
            f.handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TestSink;

    #[tokio::test(flavor = "multi_thread")]
    async fn pod_forward_binds_and_reports_close() {
        // Empty cache: the bind succeeds (it's purely local), then the serve
        // loop fails to build a client and must emit forward:closed:<id> with
        // an error string.
        let manager = ForwardManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        let info = manager
            .start(
                sink.clone(),
                "nope".into(),
                "ns".into(),
                "Pod".into(),
                "pod-a".into(),
                8080,
                None,
            )
            .await
            .expect("bind succeeds locally");
        assert!(info.local_port > 0);

        let closed_channel = format!("forward:closed:{}", info.id);
        for _ in 0..100 {
            if !sink.payloads_for(&closed_channel).is_empty() {
                manager.stop(info.id);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("forward:closed event never arrived");
    }
}
