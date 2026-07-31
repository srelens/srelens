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
    local_port: u16,
}

/// Reconnect policy: a handful of attempts with a short capped exponential
/// backoff, small enough that tests exercising the give-up path stay fast.
const MAX_ATTEMPTS: u32 = 4;
const BASE_BACKOFF_MS: u64 = 15;
const MAX_BACKOFF_MS: u64 = 150;

fn backoff_for(attempt: u32) -> std::time::Duration {
    let multiplier = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
    let ms = BASE_BACKOFF_MS
        .saturating_mul(multiplier)
        .min(MAX_BACKOFF_MS);
    std::time::Duration::from_millis(ms)
}

fn status_payload(state: &str, attempt: u32, error: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "state": state,
        "attempt": attempt,
        "error": error,
    })
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
    /// "Service"; a Service is re-resolved to a backing pod and target port
    /// on every (re)connect attempt, so a replaced pod is picked up. Returns
    /// the id + bound local port (bound once, up front, and kept stable
    /// across reconnects). The task then loops: serve until the connection
    /// drops, emit `forward:status:<id>` (`active` on establish,
    /// `reconnecting` with backoff between attempts, `failed` after
    /// `MAX_ATTEMPTS`), and emit `forward:closed:<id>` when it gives up (a
    /// user `stop(id)` aborts the task outright and never reaches that
    /// point).
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

        let listener = forward::bind_local(local_port.unwrap_or(0))
            .await
            .map_err(|e| e.to_string())?;
        let bound = listener.local_addr().map_err(|e| e.to_string())?.port();

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let closed_channel = format!("forward:closed:{id}");
        let status_channel = format!("forward:status:{id}");
        let handle = tokio::spawn(async move {
            let mut attempt: u32 = 0;
            let final_error = loop {
                attempt += 1;

                // Re-resolve Service targets every attempt so a replacement
                // pod is picked up; a Pod target's name never changes.
                let resolved = if kind.eq_ignore_ascii_case("service") {
                    forward::resolve_service_target(
                        cache.clone(),
                        &context,
                        &namespace,
                        &name,
                        Some(i32::from(remote_port)),
                    )
                    .await
                } else {
                    Ok((name.clone(), remote_port))
                };

                let attempt_result = match resolved {
                    Ok((pod, target_port)) => {
                        match forward::connect_pod_api(cache.clone(), &context, &namespace).await {
                            Ok(api) => {
                                sink.emit(&status_channel, status_payload("active", attempt, None));
                                forward::serve_pod_forward(&listener, api, pod, target_port).await
                            }
                            Err(e) => Err(e),
                        }
                    }
                    Err(e) => Err(e),
                };

                let err = match attempt_result {
                    // The accept loop only returns on error; a clean Ok is
                    // not expected in practice, but treat it as a terminal,
                    // error-free close rather than looping forever.
                    Ok(()) => break None,
                    Err(e) => e,
                };

                if attempt >= MAX_ATTEMPTS {
                    sink.emit(
                        &status_channel,
                        status_payload("failed", attempt, Some(&err)),
                    );
                    break Some(err);
                }

                sink.emit(
                    &status_channel,
                    status_payload("reconnecting", attempt, Some(&err)),
                );
                tokio::time::sleep(backoff_for(attempt)).await;
            };

            sink.emit(
                &closed_channel,
                serde_json::to_value(final_error).unwrap_or(serde_json::Value::Null),
            );
        });

        self.forwards
            .lock()
            .unwrap()
            .insert(id, Forward { handle, local_port: bound });
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

    /// The bound loopback port for a live forward id (used by the web
    /// reverse proxy), or None if the id is unknown or already stopped.
    pub fn local_port(&self, id: u64) -> Option<u16> {
        self.forwards.lock().unwrap().get(&id).map(|f| f.local_port)
    }

    /// How many port-forwards are currently running. Used to keep a user's
    /// environment alive across a WebSocket disconnect while they still have
    /// forwards in use (proxied over plain HTTP, not the WS).
    pub fn active_count(&self) -> usize {
        self.forwards.lock().unwrap().len()
    }

    /// Register a forward id → local port directly (no live cluster).
    /// Intended for tests of downstream consumers such as the web reverse
    /// proxy, which need a fake forward without standing up a real cluster.
    pub fn insert_test_forward(&self, id: u64, local_port: u16) {
        // A never-completing handle stands in for the real serve loop.
        let handle = tokio::spawn(async { std::future::pending::<()>().await });
        self.forwards
            .lock()
            .unwrap()
            .insert(id, Forward { handle, local_port });
    }

    /// Abort every running port-forward (used when a user's environment is
    /// dropped).
    pub fn shutdown_all(&self) {
        let mut forwards = self.forwards.lock().unwrap();
        for (_, forward) in forwards.drain() {
            forward.handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TestSink;

    #[tokio::test(flavor = "multi_thread")]
    async fn forward_retries_then_marks_failed() {
        // Empty cache: connecting the pod API client fails on every attempt
        // (there's no context to resolve), so the reconnect loop should walk
        // through `reconnecting` statuses and give up with `failed` after
        // MAX_ATTEMPTS, without ever reporting `active`.
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

        let status_channel = format!("forward:status:{}", info.id);
        let mut failed_payload = None;
        for _ in 0..200 {
            if let Some(p) = sink
                .payloads_for(&status_channel)
                .into_iter()
                .find(|p| p.get("state").and_then(|s| s.as_str()) == Some("failed"))
            {
                failed_payload = Some(p);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let failed_payload = failed_payload.expect("forward:status:<id> never reached 'failed'");
        assert_eq!(failed_payload["attempt"], MAX_ATTEMPTS);
        assert!(failed_payload["error"].is_string());

        let statuses = sink.payloads_for(&status_channel);
        assert!(
            statuses
                .iter()
                .any(|p| p.get("state").and_then(|s| s.as_str()) == Some("reconnecting")),
            "expected at least one 'reconnecting' status before giving up"
        );
        assert!(
            !statuses
                .iter()
                .any(|p| p.get("state").and_then(|s| s.as_str()) == Some("active")),
            "connect never succeeds against an empty cache, so 'active' should never fire"
        );

        let closed_channel = format!("forward:closed:{}", info.id);
        for _ in 0..50 {
            if !sink.payloads_for(&closed_channel).is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            !sink.payloads_for(&closed_channel).is_empty(),
            "forward:closed:<id> never fired after giving up"
        );

        manager.stop(info.id);
    }

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

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_all_aborts_forwards() {
        let manager = ForwardManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        let info = manager
            .start(
                sink,
                "nope".into(),
                "ns".into(),
                "Pod".into(),
                "pod-a".into(),
                8080,
                None,
            )
            .await
            .expect("bind succeeds locally");

        manager.shutdown_all(); // no panic; subsequent stop is a no-op
        assert!(manager.forwards.lock().unwrap().is_empty());
        manager.stop(info.id);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn local_port_lookup_matches_start_and_clears_on_stop() {
        let manager = ForwardManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        let info = manager
            .start(sink, "nope".into(), "ns".into(), "Pod".into(), "pod-a".into(), 8080, None)
            .await
            .expect("bind succeeds locally");
        assert_eq!(manager.local_port(info.id), Some(info.local_port));
        assert_eq!(manager.local_port(9999), None);
        manager.stop(info.id);
        assert_eq!(manager.local_port(info.id), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn active_count_tracks_inserts_and_stop() {
        let manager = ForwardManager::new(ClientCache::new_many(vec![]));
        assert_eq!(manager.active_count(), 0);
        manager.insert_test_forward(1, 12345);
        assert_eq!(manager.active_count(), 1);
        manager.stop(1);
        assert_eq!(manager.active_count(), 0);
    }
}
