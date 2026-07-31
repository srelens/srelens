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
/// `MAX_ATTEMPTS` counts CONSECUTIVE failed attempts since the last
/// successful establish — see `next_reconnect_state` — so a forward that's
/// been happily active for hours and then reconnects once doesn't inherit a
/// stale attempt count from an earlier flaky period: the first reconnect
/// after ANY successful establish always reports `attempt:1`, never a
/// continuation of whatever count came before that establish.
///
/// 5 (not 4) so `MAX_BACKOFF_MS`'s cap is actually reachable: sleeps only
/// happen for attempts before the last one (1..=MAX_ATTEMPTS-1), and
/// `backoff_for(4)` is the first attempt whose uncapped value (240ms)
/// exceeds the 150ms cap.
const MAX_ATTEMPTS: u32 = 5;
const BASE_BACKOFF_MS: u64 = 15;
const MAX_BACKOFF_MS: u64 = 150;

fn backoff_for(attempt: u32) -> std::time::Duration {
    let multiplier = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
    let ms = BASE_BACKOFF_MS
        .saturating_mul(multiplier)
        .min(MAX_BACKOFF_MS);
    std::time::Duration::from_millis(ms)
}

/// Outcome of one failed connect/serve attempt: whether the reconnect loop
/// should give up, and the `attempt` number to report on the
/// `forward:status:<id>` event for this failure (`reconnecting` when
/// `give_up` is false, `failed` when it's true).
struct ReconnectDecision {
    give_up: bool,
    attempt: u32,
}

/// Pure state transition for the reconnect loop's attempt bookkeeping.
/// `attempt` always counts CONSECUTIVE failures since the last successful
/// establish:
///
/// | `established_this_session` | `prev_consecutive_failures` | → `attempt` |
/// |-----------------------------|-------------------------------|-------------|
/// | `false` (never established this session) | `n`             | `n + 1`     |
/// | `true` (established, then failed)         | any (ignored)   | `1`         |
///
/// A session that establishes and later fails ALWAYS restarts the
/// consecutive-failure count at 1 — it never continues from whatever count
/// preceded the successful establish, since that would double-count an
/// earlier flaky period against a session that went on to work for a while.
/// A session that never establishes just increments as usual. `give_up` is
/// `attempt >= MAX_ATTEMPTS` either way.
fn next_reconnect_state(
    established_this_session: bool,
    prev_consecutive_failures: u32,
) -> ReconnectDecision {
    let attempt = if established_this_session {
        1
    } else {
        prev_consecutive_failures + 1
    };
    ReconnectDecision {
        give_up: attempt >= MAX_ATTEMPTS,
        attempt,
    }
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
            let mut consecutive_failures: u32 = 0;
            let final_error = loop {
                let mut established_this_session = false;

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
                                // Building the Api handle does no I/O, so it isn't
                                // evidence the target works. Probe readiness first:
                                // only a pod that's actually present + Running counts
                                // as "established". Otherwise a permanently-dead
                                // target would reset the failure counter to 1 every
                                // attempt and never reach the give-up threshold.
                                if forward::pod_is_ready(&api, &pod).await {
                                    established_this_session = true;
                                    // `active` always reports attempt:0 — reaching
                                    // "active" means there's no consecutive-failure
                                    // streak to report; see `next_reconnect_state`
                                    // for how a later failure of THIS session is
                                    // counted (always restarts at 1).
                                    sink.emit(&status_channel, status_payload("active", 0, None));
                                    forward::serve_pod_forward(&listener, api, pod, target_port).await
                                } else {
                                    Err(format!("target pod {pod} is not ready"))
                                }
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

                let decision = next_reconnect_state(established_this_session, consecutive_failures);
                consecutive_failures = decision.attempt;

                if decision.give_up {
                    sink.emit(
                        &status_channel,
                        status_payload("failed", decision.attempt, Some(&err)),
                    );
                    break Some(err);
                }

                sink.emit(
                    &status_channel,
                    status_payload("reconnecting", decision.attempt, Some(&err)),
                );
                tokio::time::sleep(backoff_for(decision.attempt)).await;
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

    #[test]
    fn next_reconnect_state_never_established_counts_up_then_gives_up() {
        // A session that never reaches "active" just counts consecutive
        // failures 1, 2, 3, ... and gives up exactly at MAX_ATTEMPTS.
        let mut consecutive_failures = 0;
        for expected in 1..MAX_ATTEMPTS {
            let decision = next_reconnect_state(false, consecutive_failures);
            assert_eq!(decision.attempt, expected);
            assert!(
                !decision.give_up,
                "attempt {expected} should not give up yet"
            );
            consecutive_failures = decision.attempt;
        }
        let decision = next_reconnect_state(false, consecutive_failures);
        assert_eq!(decision.attempt, MAX_ATTEMPTS);
        assert!(
            decision.give_up,
            "the MAX_ATTEMPTS-th failure should give up"
        );
    }

    #[test]
    fn next_reconnect_state_established_then_failed_resets_to_one() {
        // No matter how many consecutive failures preceded a successful
        // establish, the first failure AFTER that establish is attempt:1 —
        // it never continues counting from whatever came before the success.
        for prev in [0, 1, MAX_ATTEMPTS - 1, MAX_ATTEMPTS, 100] {
            let decision = next_reconnect_state(true, prev);
            assert_eq!(decision.attempt, 1, "prev_consecutive_failures={prev}");
            assert!(!decision.give_up, "a single failure should never give up");
        }
    }

    #[test]
    fn next_reconnect_state_repeated_establish_fail_cycles_never_accumulate() {
        // Simulate many establish -> fail cycles in a row (e.g. a pod that
        // keeps getting recreated and immediately deleted again): the
        // attempt count must reset to 1 every single time, never climbing
        // toward MAX_ATTEMPTS just because it happened before.
        let mut consecutive_failures = 0;
        for _ in 0..(MAX_ATTEMPTS * 3) {
            let decision = next_reconnect_state(true, consecutive_failures);
            assert_eq!(decision.attempt, 1);
            assert!(!decision.give_up);
            consecutive_failures = decision.attempt;
        }
    }

    #[test]
    fn next_reconnect_state_give_up_threshold_is_exactly_max_attempts() {
        assert!(!next_reconnect_state(false, MAX_ATTEMPTS - 2).give_up);
        assert!(next_reconnect_state(false, MAX_ATTEMPTS - 1).give_up);
    }

    // NOTE on coverage: the tests above exhaustively prove the pure
    // transition (`next_reconnect_state`) that the loop relies on, but not
    // the full end-to-end path — "forward is active against a real pod,
    // that pod is deleted, the streams loop reconnects, emits active again,
    // then a later failure streak counts 1..MAX_ATTEMPTS instead of
    // continuing from whatever count preceded the successful establish".
    // Driving that deterministically would need `connect_pod_api` /
    // `serve_pod_forward` to succeed without a real cluster, which needs a
    // fake kube apiserver (e.g. a mock HTTP service behind
    // `kube::Client::new`) — no such test harness exists in this repo today
    // and building one would pull in a new dev-dependency (tower/hyper test
    // helpers), which is out of scope here. A separate follow-up issue
    // tracks building that cluster harness. See the task report for this
    // gap.

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
