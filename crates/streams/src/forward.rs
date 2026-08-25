//! Port-forward core: binds a local loopback port, pipes it to a pod (or a
//! service's backing pod) via kube-rs, and tracks the running forwards.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use srelens_kube::client_cache::ClientCache;
use srelens_kube::forward;
use tokio::task::JoinHandle;

use crate::sink::EventSink;

struct Forward {
    handle: JoinHandle<()>,
    /// The one counter for this forward, shared with the serving task and its
    /// reporter. Created with the forward, never per attempt — see
    /// `reconnect_loop`.
    traffic: Arc<forward::TrafficCounter>,
    /// Everything about this forward that can't change after `start`. The
    /// byte total isn't among them, so `list` reads that from `traffic`
    /// rather than from a copy that goes stale the moment a packet crosses.
    fixed: FixedFacts,
    /// This forward's terminal state, written once by its own task. See
    /// [`Ended`].
    ended: Ended,
}

/// Whether a forward's task has finished, and why.
///
/// Unset while the tunnel is running; set exactly once, by the task itself,
/// at the moment it gives up — `None` for a loop that ended without an error,
/// `Some(reason)` for one that exhausted its retries.
///
/// **This is what a reloading client has instead of the event it missed.** A
/// forward that gives up emits `forward:closed:<id>` and then stays in the
/// manager's map until someone calls `stop`, because only the client knows
/// whether its reader has seen the bad news yet. A page that reloads after
/// that event has no way to learn it happened: the event is gone and the
/// frontend store that recorded it died with the page. Without this, `list`
/// described a dead tunnel exactly like a live one and the reloaded page drew
/// it green.
///
/// A `OnceLock` rather than a `Mutex<Option<..>>` because it is written once
/// and read on every listing, and "set" is itself the fact being reported —
/// `get()` returning `None` means still running, and there is no second flag
/// that could disagree with it.
type Ended = Arc<OnceLock<Option<String>>>;

impl Forward {
    /// True once this forward's own task has stopped for good.
    fn has_ended(&self) -> bool {
        self.ended.get().is_some()
    }

    /// This forward as `list` reports it, byte total and terminal state read
    /// live rather than from copies taken at start.
    fn entry(&self) -> ForwardEntry {
        let ended = self.ended.get();
        ForwardEntry {
            id: self.fixed.id,
            context: self.fixed.context.clone(),
            namespace: self.fixed.namespace.clone(),
            kind: self.fixed.kind.clone(),
            name: self.fixed.name.clone(),
            remote_port: self.fixed.remote_port,
            local_port: self.fixed.local_port,
            started_at: self.fixed.started_at,
            bytes: self.traffic.total(),
            ended: ended.is_some(),
            error: ended.and_then(|reason| reason.clone()),
        }
    }
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

/// How often a forward reports its byte total. A per-packet event would
/// flood the channel on a busy tunnel — the same reasoning that made the log
/// stream report status on transitions rather than once per line. A second is
/// slow enough to be free and fast enough that a number on screen still looks
/// live.
const TRAFFIC_INTERVAL: Duration = Duration::from_secs(1);

fn backoff_for(attempt: u32) -> std::time::Duration {
    let multiplier = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
    let ms = BASE_BACKOFF_MS
        .saturating_mul(multiplier)
        .min(MAX_BACKOFF_MS);
    std::time::Duration::from_millis(ms)
}

/// Wall-clock milliseconds since the epoch. Milliseconds rather than a
/// `SystemTime` because this crosses to a JavaScript frontend, which dates
/// things that way and would otherwise have to reassemble one from parts.
fn epoch_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
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

/// The channel a forward reports its byte total on. A named function rather
/// than an inline `format!` because this string is a contract with the
/// frontend store, and a typo in it would be silent on both sides.
fn traffic_channel(id: u64) -> String {
    format!("forward:traffic:{id}")
}

fn traffic_payload(bytes: u64) -> serde_json::Value {
    serde_json::json!({ "bytes": bytes })
}

/// Emit `forward:traffic:<id>` on `interval`, and only when the total moved.
/// An idle tunnel therefore says nothing at all rather than repeating the
/// same number once a second forever. Never returns: it is driven inside the
/// forward's own task (see `start`), so it stops exactly when the forward
/// does — including when a `stop(id)` aborts that task mid-tick.
async fn report_traffic(
    sink: &dyn EventSink,
    channel: &str,
    traffic: &forward::TrafficCounter,
    interval: Duration,
) {
    let mut reported = 0u64;
    loop {
        tokio::time::sleep(interval).await;
        let total = traffic.total();
        if total != reported {
            reported = total;
            sink.emit(channel, traffic_payload(total));
        }
    }
}

/// What one connect-and-serve attempt did: whether it reached a live target
/// (which is what resets the consecutive-failure count, see
/// `next_reconnect_state`) and how the serving ended.
struct AttemptOutcome {
    established: bool,
    result: Result<(), String>,
}

type AttemptFuture = Pin<Box<dyn Future<Output = AttemptOutcome> + Send>>;

/// One attempt, handed the forward's traffic counter to count into.
/// Production hands over `serve_once`; tests substitute a closure, which is
/// the only way to drive the reconnect loop without a live cluster.
type Attempt = Box<dyn FnMut(Arc<forward::TrafficCounter>) -> AttemptFuture + Send>;

/// Everything one attempt needs to resolve and serve the target, behind one
/// `Arc` so an attempt clones a pointer instead of five strings.
struct AttemptCtx {
    cache: Arc<ClientCache>,
    sink: Arc<dyn EventSink>,
    listener: tokio::net::TcpListener,
    status_channel: String,
    context: String,
    namespace: String,
    kind: String,
    name: String,
    remote_port: u16,
}

/// Resolve the target, then serve it until the connection drops. Emits
/// `active` itself rather than leaving it to the loop, because the moment
/// worth reporting is when the tunnel becomes usable — not after it ends.
async fn serve_once(ctx: Arc<AttemptCtx>, traffic: Arc<forward::TrafficCounter>) -> AttemptOutcome {
    fn failed(error: String) -> AttemptOutcome {
        AttemptOutcome {
            established: false,
            result: Err(error),
        }
    }

    // Re-resolve Service targets every attempt so a replacement pod is picked
    // up; a Pod target's name never changes.
    let resolved = if ctx.kind.eq_ignore_ascii_case("service") {
        forward::resolve_service_target(
            ctx.cache.clone(),
            &ctx.context,
            &ctx.namespace,
            &ctx.name,
            Some(i32::from(ctx.remote_port)),
        )
        .await
    } else {
        Ok((ctx.name.clone(), ctx.remote_port))
    };
    let (pod, target_port) = match resolved {
        Ok(target) => target,
        Err(e) => return failed(e),
    };

    let api = match forward::connect_pod_api(ctx.cache.clone(), &ctx.context, &ctx.namespace).await
    {
        Ok(api) => api,
        Err(e) => return failed(e),
    };

    // Building the Api handle does no I/O, so it isn't evidence the target
    // works. Probe readiness first: only a pod that's actually present +
    // Running counts as "established". Otherwise a permanently-dead target
    // would reset the failure counter to 1 every attempt and never reach the
    // give-up threshold.
    if !forward::pod_is_ready(&api, &pod).await {
        return failed(format!("target pod {pod} is not ready"));
    }

    // `active` always reports attempt:0 — reaching "active" means there's no
    // consecutive-failure streak to report; see `next_reconnect_state` for
    // how a later failure of THIS session is counted (always restarts at 1).
    ctx.sink
        .emit(&ctx.status_channel, status_payload("active", 0, None));
    let result = forward::serve_pod_forward(&ctx.listener, api, pod, target_port, traffic).await;
    AttemptOutcome {
        established: true,
        result,
    }
}

/// Serve, and on failure back off and serve again, until an attempt ends
/// cleanly or `MAX_ATTEMPTS` consecutive failures give up. Returns the error
/// it gave up on, if any.
///
/// `traffic` is the FORWARD's counter, taken as a parameter and cloned into
/// each attempt on purpose. A counter created inside this loop would reset a
/// tunnel's running total on every reconnect, and nothing would reveal it
/// until a reconnect happened: a busy tunnel would quietly read as a fresh
/// one every time its pod restarted.
async fn reconnect_loop(
    sink: &dyn EventSink,
    status_channel: &str,
    traffic: Arc<forward::TrafficCounter>,
    mut attempt: Attempt,
) -> Option<String> {
    let mut consecutive_failures: u32 = 0;
    loop {
        let outcome = attempt(traffic.clone()).await;

        let err = match outcome.result {
            // An attempt only returns on error; a clean Ok is not expected in
            // practice, but treat it as a terminal, error-free close rather
            // than looping forever.
            Ok(()) => break None,
            Err(e) => e,
        };

        let decision = next_reconnect_state(outcome.established, consecutive_failures);
        consecutive_failures = decision.attempt;

        if decision.give_up {
            sink.emit(
                status_channel,
                status_payload("failed", decision.attempt, Some(&err)),
            );
            break Some(err);
        }

        sink.emit(
            status_channel,
            status_payload("reconnecting", decision.attempt, Some(&err)),
        );
        tokio::time::sleep(backoff_for(decision.attempt)).await;
    }
}

/// Owns running port-forwards (keyed by numeric id).
pub struct ForwardManager {
    cache: Arc<ClientCache>,
    next_id: AtomicU64,
    forwards: Mutex<HashMap<u64, Forward>>,
}

/// What `start` returns: the forward's id, the actual local port it bound to
/// (the OS picks one when the caller passes no preference), and the epoch
/// millis it was stamped with. That stamp is the SAME value `list` reports
/// for this forward later — read once here, off `FixedFacts.started_at` — so
/// a row this session started and a row recovered after a reload agree on
/// the tunnel's age instead of a caller having to ask `list` a second time
/// just to date what it already started.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardInfo {
    pub id: u64,
    pub local_port: u16,
    pub started_at: u64,
}

/// The parts of a forward that are settled once and never change again.
#[derive(Debug, Clone)]
struct FixedFacts {
    id: u64,
    context: String,
    namespace: String,
    kind: String,
    name: String,
    remote_port: u16,
    local_port: u16,
    started_at: u64,
}

/// One forward the manager is holding, as `list` reports it — everything a
/// client needs to draw a row for a tunnel it did not start itself, including
/// one that has already died (`ended`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardEntry {
    pub id: u64,
    pub context: String,
    pub namespace: String,
    /// "Pod" or "Service".
    pub kind: String,
    pub name: String,
    pub remote_port: u16,
    pub local_port: u16,
    /// Epoch milliseconds, stamped when the forward was created. A client
    /// that rehydrates after a reload dates the tunnel from this, so its age
    /// is the tunnel's real age rather than how long that client has known
    /// about it — two different facts that would otherwise share a column.
    pub started_at: u64,
    /// Bytes moved since the forward started, read live from its counter.
    pub bytes: u64,
    /// True once this forward's task has given up — see [`Ended`]. The
    /// manager still holds it, and still reports it, precisely so a client
    /// that was not listening when it died can draw the row that says so.
    pub ended: bool,
    /// Why it gave up, when the loop had a reason to give. `None` both for a
    /// forward that is still running and for one that ended without an error.
    pub error: Option<String>,
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
    /// point). Alongside it, `forward:traffic:<id>` carries the running byte
    /// total once a second whenever it has moved.
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
        let listener = forward::bind_local(local_port.unwrap_or(0))
            .await
            .map_err(|e| e.to_string())?;
        let bound = listener.local_addr().map_err(|e| e.to_string())?.port();

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let closed_channel = format!("forward:closed:{id}");
        let status_channel = format!("forward:status:{id}");
        let traffic_channel = traffic_channel(id);

        // One counter for the life of the forward, created here rather than
        // inside the reconnect loop so a reconnect adds to a tunnel's total
        // instead of restarting it.
        let traffic = Arc::new(forward::TrafficCounter::default());

        let ctx = Arc::new(AttemptCtx {
            cache: self.cache.clone(),
            sink: sink.clone(),
            listener,
            status_channel: status_channel.clone(),
            context: context.clone(),
            namespace: namespace.clone(),
            kind: kind.clone(),
            name: name.clone(),
            remote_port,
        });
        let attempt: Attempt = Box::new(move |traffic| Box::pin(serve_once(ctx.clone(), traffic)));

        let loop_traffic = traffic.clone();
        let ended: Ended = Arc::new(OnceLock::new());
        let task_ended = ended.clone();
        let handle = tokio::spawn(async move {
            let final_error = tokio::select! {
                gave_up = reconnect_loop(
                    sink.as_ref(),
                    &status_channel,
                    loop_traffic.clone(),
                    attempt,
                ) => gave_up,
                // Never completes on its own. It lives in this task so it
                // shares the forward's lifetime exactly: the forward ending
                // drops it, and a `stop(id)` aborting this task stops the
                // reporting with it.
                () = report_traffic(
                    sink.as_ref(),
                    &traffic_channel,
                    loop_traffic.as_ref(),
                    TRAFFIC_INTERVAL,
                ) => None,
            };

            // Recorded BEFORE the event goes out, so a client that reacts
            // to `forward:closed` by listing cannot be told the forward is
            // still live by the very call it made because it heard it wasn't.
            let _ = task_ended.set(final_error.clone());
            sink.emit(
                &closed_channel,
                serde_json::to_value(final_error).unwrap_or(serde_json::Value::Null),
            );
        });

        // Stamped once, here, and read back for both the response and the
        // stored fact — never a second call to `epoch_millis()` for the same
        // forward, so `start`'s answer and a later `list` can't disagree.
        let started_at = epoch_millis();

        self.forwards.lock().unwrap().insert(
            id,
            Forward {
                handle,
                traffic,
                fixed: FixedFacts {
                    id,
                    context,
                    namespace,
                    kind,
                    name,
                    remote_port,
                    local_port: bound,
                    started_at,
                },
                ended,
            },
        );
        Ok(ForwardInfo {
            id,
            local_port: bound,
            started_at,
        })
    }

    /// Stop a port-forward and abort its task.
    pub fn stop(&self, id: u64) {
        if let Some(f) = self.forwards.lock().unwrap().remove(&id) {
            f.handle.abort();
        }
    }

    /// What the manager is holding, oldest id first. This is what closes the
    /// web leak: the frontend store is module-level and dies with a browser
    /// reload, while this manager does not, so without it a user reloads into
    /// an empty table with live tunnels behind it and no way to stop them.
    /// Ordered because a HashMap hands out its values in whatever order it
    /// likes, and a table that reshuffles on every poll is unreadable.
    ///
    /// **Holding, not running.** A forward that gave up is still listed, with
    /// `ended` set and its reason attached, because the reload this exists
    /// for is exactly the case that missed the `forward:closed` event — see
    /// [`Ended`]. `local_port` and `active_count` answer the other
    /// question, "is this tunnel carrying traffic", and both say no.
    pub fn list(&self) -> Vec<ForwardEntry> {
        let mut entries: Vec<ForwardEntry> = self
            .forwards
            .lock()
            .unwrap()
            .values()
            .map(Forward::entry)
            .collect();
        entries.sort_by_key(|e| e.id);
        entries
    }

    /// The bound loopback port for a LIVE forward id (used by the web
    /// reverse proxy), or None if the id is unknown, already stopped, or has
    /// given up. The last of those matters: the listener is dropped with the
    /// task that owned it, so the port a dead forward was bound to is free
    /// for anything on the machine to take — proxying to it is at best a
    /// connection to nothing.
    pub fn local_port(&self, id: u64) -> Option<u16> {
        self.forwards
            .lock()
            .unwrap()
            .get(&id)
            .filter(|f| !f.has_ended())
            .map(|f| f.fixed.local_port)
    }

    /// How many port-forwards are currently running. Used to keep a user's
    /// environment alive across a WebSocket disconnect while they still have
    /// forwards in use (proxied over plain HTTP, not the WS). One that gave
    /// up is not in use, however long its row stays on someone's screen.
    pub fn active_count(&self) -> usize {
        self.forwards
            .lock()
            .unwrap()
            .values()
            .filter(|f| !f.has_ended())
            .count()
    }

    /// Register a forward id → local port directly (no live cluster).
    /// Intended for tests of downstream consumers such as the web reverse
    /// proxy, which need a fake forward without standing up a real cluster.
    /// Returns the forward's traffic counter so a caller can move fake bytes
    /// through it.
    pub fn insert_test_forward(&self, id: u64, local_port: u16) -> Arc<forward::TrafficCounter> {
        // A never-completing handle stands in for the real serve loop.
        let handle = tokio::spawn(async { std::future::pending::<()>().await });
        let traffic = Arc::new(forward::TrafficCounter::default());
        self.forwards.lock().unwrap().insert(
            id,
            Forward {
                handle,
                traffic: traffic.clone(),
                fixed: FixedFacts {
                    id,
                    context: "test".into(),
                    namespace: "test".into(),
                    kind: "Pod".into(),
                    name: "test".into(),
                    remote_port: 0,
                    local_port,
                    started_at: epoch_millis(),
                },
                // Never set: the stand-in handle above never completes, which
                // is what a live forward looks like.
                ended: Arc::new(OnceLock::new()),
            },
        );
        traffic
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
    //
    // The traffic reporting has the same seam. `reconnect_loop` and
    // `report_traffic` are driven directly below, which is what proves the
    // counter survives a reconnect and that reports land on the tick, but
    // the wiring `start` does between them cannot be observed end-to-end
    // without a cluster: against an empty cache the forward gives up after
    // ~400ms, so the production one-second tick never arrives. What is
    // pinned instead is the channel name (`traffic_channel`), which is the
    // part of that wiring the frontend depends on.

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

    /// The reporter's tempo in tests. Production reports once a second, and a
    /// test that waited on that would be a second slower for every tick it
    /// needed to observe.
    const TEST_TICK: std::time::Duration = std::time::Duration::from_millis(25);

    fn spawn_reporter(
        sink: Arc<TestSink>,
        channel: &str,
        traffic: Arc<forward::TrafficCounter>,
    ) -> JoinHandle<()> {
        let channel = channel.to_string();
        tokio::spawn(async move {
            report_traffic(sink.as_ref(), &channel, &traffic, TEST_TICK).await;
        })
    }

    fn reported_bytes(sink: &TestSink, channel: &str) -> Vec<u64> {
        sink.payloads_for(channel)
            .iter()
            .map(|p| {
                p.get("bytes")
                    .and_then(|b| b.as_u64())
                    .expect("every traffic payload carries a numeric `bytes`")
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_reconnecting_forward_reports_the_sum_of_everything_it_moved() {
        // The trap this test exists for: a counter created per ATTEMPT would
        // reset a busy tunnel's total every time it reconnected, and nothing
        // would show it until a reconnect happened. Three attempts move 100,
        // 150 and 50 bytes; the tunnel has moved 300, not 50.
        let sink = Arc::new(TestSink::default());
        let traffic = Arc::new(forward::TrafficCounter::default());
        let calls = Arc::new(AtomicU64::new(0));
        let seen = calls.clone();
        let attempt: Attempt = Box::new(move |traffic: Arc<forward::TrafficCounter>| {
            let call = seen.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                traffic.add([100u64, 150, 50][call.min(2) as usize]);
                AttemptOutcome {
                    established: true,
                    // The first two attempts drop the connection, which is
                    // what forces the reconnects this test is about.
                    result: if call < 2 {
                        Err(format!("attempt {call} dropped"))
                    } else {
                        Ok(())
                    },
                }
            })
        });

        let reporter = spawn_reporter(sink.clone(), "forward:traffic:7", traffic.clone());
        let final_error =
            reconnect_loop(sink.as_ref(), "forward:status:7", traffic.clone(), attempt).await;
        assert_eq!(final_error, None, "the third attempt ends cleanly");
        // Let a tick land after the last attempt so the final total is out.
        tokio::time::sleep(TEST_TICK * 3).await;
        reporter.abort();

        assert_eq!(calls.load(Ordering::SeqCst), 3, "three attempts ran");
        let reconnecting = sink.payloads_for("forward:status:7");
        assert_eq!(
            reconnecting.len(),
            2,
            "two dropped attempts means two reconnects actually happened"
        );
        assert_eq!(
            traffic.total(),
            300,
            "one counter across all three attempts"
        );
        assert_eq!(
            reported_bytes(&sink, "forward:traffic:7").last().copied(),
            Some(300),
            "the last report is the tunnel's running total, not the last attempt's share"
        );
    }

    #[test]
    fn traffic_is_reported_on_the_forwards_own_channel() {
        // The name the frontend store subscribes to, spelled out here rather
        // than rebuilt from the same format string it is being checked
        // against.
        assert_eq!(traffic_channel(7), "forward:traffic:7");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_idle_forward_reports_no_traffic_at_all() {
        let sink = Arc::new(TestSink::default());
        let traffic = Arc::new(forward::TrafficCounter::default());
        let reporter = spawn_reporter(sink.clone(), "forward:traffic:1", traffic);
        tokio::time::sleep(TEST_TICK * 4).await;
        reporter.abort();
        assert!(
            sink.payloads_for("forward:traffic:1").is_empty(),
            "a tunnel nothing has crossed stays silent"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn traffic_is_reported_again_only_when_the_total_changed() {
        let sink = Arc::new(TestSink::default());
        let traffic = Arc::new(forward::TrafficCounter::default());
        let reporter = spawn_reporter(sink.clone(), "forward:traffic:2", traffic.clone());

        traffic.add(10);
        tokio::time::sleep(TEST_TICK * 2).await;
        assert_eq!(reported_bytes(&sink, "forward:traffic:2"), vec![10]);

        // Several ticks with nothing crossing: the channel must stay quiet.
        tokio::time::sleep(TEST_TICK * 4).await;
        assert_eq!(
            reported_bytes(&sink, "forward:traffic:2"),
            vec![10],
            "an unchanged total is not news"
        );

        traffic.add(5);
        tokio::time::sleep(TEST_TICK * 2).await;
        reporter.abort();
        assert_eq!(
            reported_bytes(&sink, "forward:traffic:2"),
            vec![10, 15],
            "the report carries the running total, not the delta"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_busy_tunnel_reports_on_the_tick_not_on_every_packet() {
        // A per-packet event would flood the channel; the timer is the whole
        // point. 40 writes spread over roughly two ticks must coalesce into
        // roughly two reports.
        let sink = Arc::new(TestSink::default());
        let traffic = Arc::new(forward::TrafficCounter::default());
        let reporter = spawn_reporter(sink.clone(), "forward:traffic:3", traffic.clone());

        let writer = traffic.clone();
        for _ in 0..40 {
            writer.add(1);
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        tokio::time::sleep(TEST_TICK * 2).await;
        reporter.abort();

        let reports = reported_bytes(&sink, "forward:traffic:3");
        assert!(
            reports.len() <= 5,
            "40 writes should coalesce onto the tick, got {} reports: {reports:?}",
            reports.len()
        );
        assert_eq!(
            reports.last().copied(),
            Some(40),
            "the last report is the full total"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_returns_the_live_forward_with_its_target_and_ports() {
        let manager = ForwardManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        let info = manager
            .start(
                sink,
                "ctx-a".into(),
                "ns-a".into(),
                "Service".into(),
                "web".into(),
                8080,
                None,
            )
            .await
            .expect("bind succeeds locally");

        let listed = manager.list();
        assert_eq!(listed.len(), 1);
        let entry = &listed[0];
        assert_eq!(entry.id, info.id);
        assert_eq!(entry.context, "ctx-a");
        assert_eq!(entry.namespace, "ns-a");
        assert_eq!(entry.kind, "Service");
        assert_eq!(entry.name, "web");
        assert_eq!(entry.remote_port, 8080);
        assert_eq!(entry.local_port, info.local_port);
        assert_eq!(entry.bytes, 0, "nothing has crossed it yet");
        manager.stop(info.id);
    }

    #[test]
    fn a_forward_entry_serialises_the_names_the_frontend_reads() {
        // A rehydrating store reads these keys by name, so the casing is a
        // contract rather than a detail of how the struct happens to be
        // written.
        let entry = ForwardEntry {
            id: 4,
            context: "ctx".into(),
            namespace: "ns".into(),
            kind: "Pod".into(),
            name: "api".into(),
            remote_port: 8080,
            local_port: 51234,
            started_at: 1_700_000_000_000,
            bytes: 2048,
            ended: false,
            error: None,
        };
        assert_eq!(
            serde_json::to_value(&entry).expect("a forward entry is serialisable"),
            serde_json::json!({
                "id": 4,
                "context": "ctx",
                "namespace": "ns",
                "kind": "Pod",
                "name": "api",
                "remotePort": 8080,
                "localPort": 51234,
                "startedAt": 1_700_000_000_000u64,
                "bytes": 2048,
                "ended": false,
                "error": null,
            })
        );

        // The dead form, whose two extra keys are the whole of what a
        // reloading client has to go on.
        let gone = ForwardEntry {
            ended: true,
            error: Some("pod web-1 not found".into()),
            ..entry
        };
        let json = serde_json::to_value(&gone).expect("a forward entry is serialisable");
        assert_eq!(json["ended"], serde_json::json!(true));
        assert_eq!(json["error"], serde_json::json!("pod web-1 not found"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_drops_a_forward_the_user_stopped() {
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
        assert_eq!(manager.list().len(), 1);
        manager.stop(info.id);
        assert!(
            manager.list().is_empty(),
            "a stopped tunnel is not still forwarding"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_returns_the_same_started_at_that_list_later_reports() {
        // One value, stamped once: whatever `start` hands back for this
        // forward must be the exact number `list` reports for it later, not
        // a second reading that merely happens to be close.
        let manager = ForwardManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        let before = epoch_millis();
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

        assert!(
            info.started_at >= before,
            "start's own started_at {} predates the call at {before}",
            info.started_at
        );
        let entry = manager.list().remove(0);
        assert_eq!(
            info.started_at, entry.started_at,
            "the start response and list must agree on the same stamp"
        );
        manager.stop(info.id);
    }

    #[test]
    fn forward_info_serialises_started_at_alongside_id_and_local_port() {
        // What the desktop command and the server command hand back to the
        // frontend; a rename or a dropped field here is a silent contract
        // break with `packages/core`.
        let info = ForwardInfo {
            id: 4,
            local_port: 51234,
            started_at: 1_700_000_000_000,
        };
        assert_eq!(
            serde_json::to_value(&info).expect("a forward info is serialisable"),
            serde_json::json!({
                "id": 4,
                "localPort": 51234,
                "startedAt": 1_700_000_000_000u64,
            })
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_stamps_the_start_time_when_the_forward_was_created() {
        // A rehydrating client dates the forward from this, so it has to be
        // when the tunnel started rather than when someone asked about it.
        let manager = ForwardManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        let before = epoch_millis();
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

        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        let asked_at = epoch_millis();
        let entry = manager.list().remove(0);
        assert!(
            entry.started_at >= before,
            "started_at {} predates the start call at {before}",
            entry.started_at
        );
        assert!(
            entry.started_at + 40 < asked_at,
            "started_at {} looks like the time of the list call ({asked_at}), not of the start",
            entry.started_at
        );
        manager.stop(info.id);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_reports_the_live_byte_total() {
        let manager = ForwardManager::new(ClientCache::new_many(vec![]));
        let traffic = manager.insert_test_forward(1, 12345);
        assert_eq!(manager.list()[0].bytes, 0);
        traffic.add(4096);
        assert_eq!(
            manager.list()[0].bytes,
            4096,
            "bytes are read from the forward's counter, not from a copy taken at start"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_is_ordered_by_id() {
        // A table that reshuffles every time it is polled is unreadable, and
        // a HashMap hands out its values in whatever order it likes.
        let manager = ForwardManager::new(ClientCache::new_many(vec![]));
        for id in [3u64, 1, 2] {
            manager.insert_test_forward(id, 10000 + id as u16);
        }
        let ids: Vec<u64> = manager.list().iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }
    /// Wait until a forward's `forward:closed:<id>` has fired — the moment
    /// its task has given up for good. The task records the terminal state
    /// BEFORE it emits, so once this returns the manager's own answers about
    /// the forward are settled and nothing below is racing the loop.
    async fn wait_for_close(sink: &TestSink, id: u64) {
        let channel = format!("forward:closed:{id}");
        for _ in 0..200 {
            if !sink.payloads_for(&channel).is_empty() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("forward:closed:{id} never arrived");
    }

    async fn start_doomed(manager: &ForwardManager, sink: Arc<TestSink>) -> ForwardInfo {
        // Empty cache: every connect attempt fails, so the loop walks its
        // retries and gives up in a few hundred milliseconds.
        manager
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
            .expect("bind succeeds locally")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_reports_a_forward_that_gave_up_as_ended_with_its_reason() {
        // The half of the vanishing-tunnel defect that no amount of frontend
        // bookkeeping can reach. A forward that exhausts its retries emits
        // `forward:closed:<id>` and then STAYS in this map until someone
        // calls `stop`. `list` used to describe it exactly like a live one,
        // so a page that reloaded after that event had already fired adopted
        // a dead tunnel as `active` — a green row for a tunnel that cannot
        // carry a byte, and in web mode a `/pf/<id>/` URL that will never
        // answer. The frontend's dropped-id set cannot help: it is
        // module-level JavaScript that the reload wiped.
        let manager = ForwardManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        let info = start_doomed(&manager, sink.clone()).await;

        // Said before AND after, because "ended" asserted only at the end
        // passes just as well for a `list` that hardcodes it.
        let trying = manager.list();
        assert_eq!(trying.len(), 1);
        assert!(!trying[0].ended, "a forward still retrying has not ended");
        assert_eq!(trying[0].error, None);

        wait_for_close(&sink, info.id).await;

        let listed = manager.list();
        assert_eq!(
            listed.len(),
            1,
            "the entry stays, so a client that reloaded still learns the tunnel died"
        );
        assert!(listed[0].ended, "a forward that gave up is listed as ended");
        assert!(
            listed[0].error.is_some(),
            "and with the reason the loop gave up, so the row can say why"
        );

        manager.stop(info.id);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_forward_that_gave_up_is_neither_routable_nor_counted() {
        // `list` keeps reporting a dead forward on purpose; the other two
        // readers of the map must not. The web reverse proxy resolves
        // `/pf/<id>/` through `local_port`, and the loopback port a dead
        // forward was bound to is released the moment its task ends — so
        // routing to it reaches nothing at best, and whatever took the port
        // next at worst. `active_count` keeps a user's environment alive for
        // forwards "in use", which a dead one is not.
        let manager = ForwardManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        let info = start_doomed(&manager, sink.clone()).await;

        assert_eq!(manager.local_port(info.id), Some(info.local_port));
        assert_eq!(manager.active_count(), 1);

        wait_for_close(&sink, info.id).await;

        assert_eq!(
            manager.local_port(info.id),
            None,
            "the proxy must not route to a port nothing is listening on"
        );
        assert_eq!(
            manager.active_count(),
            0,
            "a tunnel that gave up is not a tunnel in use"
        );
        // And it is still listed — the two answers are deliberately different.
        assert_eq!(manager.list().len(), 1);

        manager.stop(info.id);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stopping_a_forward_that_gave_up_forgets_it_for_good() {
        // How a reader gets a dead row off their screen permanently. The
        // frontend dismisses it by calling `stop_port_forward`, because a
        // dropped-id set in the page cannot survive the reload that would
        // otherwise raise the row again from this listing.
        let manager = ForwardManager::new(ClientCache::new_many(vec![]));
        let sink = Arc::new(TestSink::default());
        let info = start_doomed(&manager, sink.clone()).await;
        wait_for_close(&sink, info.id).await;
        assert_eq!(manager.list().len(), 1);

        manager.stop(info.id);

        assert!(
            manager.list().is_empty(),
            "a dismissed tunnel does not come back on the next listing"
        );
    }
}
