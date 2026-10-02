//! Calls from the sidecar to srelens (`host/read`, `host/resource`,
//! `host/action`). Each is checked first: a field srelens would refuse is
//! refused here (`HostError::InvalidCall`), taking no slot and sending
//! nothing. srelens works on at most [`HOST_CALLS_IN_FLIGHT`] (8) of a
//! sidecar's calls at once, or fewer when `initialize`'s
//! `maxConcurrentRequests` is lower (`call_slots`), and stops a sidecar with
//! 16 unanswered, so at most that many are sent at once and the rest wait
//! here.
//!
//! Whether a dropped call's request had already reached the wire decides
//! what happens to its slot. Dropped before that (the outbox's queue was
//! full): srelens never saw the id, a cancel for it would answer nothing,
//! and holding the slot for a call srelens does not know about would leak
//! it forever -- so the slot is freed at once. Dropped after: the slot is
//! kept until srelens answers, including `-32800` for a cancelled call, and
//! a `$/cancelRequest` is sent so that answer comes sooner.

use serde_json::Value;
use srelens_sidecar_protocol::{
    method, shape, CallContext, CancelParams, HostActionParams, HostReadParams, HostResourceParams,
    InitializeLimits, Notification, Request, RequestId, Response, RpcError,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};

use crate::outbox::{Outbox, Unsent};
use crate::HostError;

pub(crate) const HOST_CALLS_IN_FLIGHT: usize = 8;

/// How many of a sidecar's calls may be in flight at once, given the limits
/// `initialize` brought: the host's `maxConcurrentRequests`, and never more
/// than [`HOST_CALLS_IN_FLIGHT`], which is what srelens works on at once. A
/// limit of 0 is read as 8, not as none (a semaphore with no permit would
/// hold every call forever), and so is one too big for a `usize`.
fn call_slots(limits: &InitializeLimits) -> usize {
    match usize::try_from(limits.max_concurrent_requests) {
        Ok(0) | Err(_) => HOST_CALLS_IN_FLIGHT,
        Ok(n) => n.min(HOST_CALLS_IN_FLIGHT),
    }
}

struct Waiting {
    answer: oneshot::Sender<Result<Value, RpcError>>,
    _place: OwnedSemaphorePermit,
}

/// The way to srelens from a handler: `ctx.host()`.
///
/// Every call's fields are checked before it is sent: one srelens would
/// refuse fails the call with [`HostError::InvalidCall`], naming the field.
#[derive(Clone)]
pub struct Host {
    inner: Arc<Inner>,
}

struct Inner {
    outbox: Outbox,
    next: AtomicU64,
    /// `None` once the session has ended: every waiter answered
    /// `Disconnected` by the drop of its `answer` sender, and a call that
    /// arrives after refuses at once rather than queuing anything.
    waiting: Mutex<Option<HashMap<String, Waiting>>>,
    places: Arc<Semaphore>,
    /// Tests only: where the next task sending a dropped call's cancel
    /// waits, once it has room for the cancel and before it checks the host
    /// and queues it (see [`Host::hold_next_cancel`]).
    #[cfg(test)]
    cancel_gate: Mutex<Option<CancelGate>>,
    /// Tests only: run as [`Host::disconnect`] starts, before it wakes any
    /// call (see [`Host::on_disconnect`]).
    #[cfg(test)]
    on_disconnect: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl Host {
    /// The way to srelens for a session whose host brought `limits`.
    pub(crate) fn new(outbox: Outbox, limits: &InitializeLimits) -> Host {
        Host {
            inner: Arc::new(Inner {
                outbox,
                next: AtomicU64::new(0),
                waiting: Mutex::new(Some(HashMap::new())),
                places: Arc::new(Semaphore::new(call_slots(limits))),
                #[cfg(test)]
                cancel_gate: Mutex::new(None),
                #[cfg(test)]
                on_disconnect: Mutex::new(None),
            }),
        }
    }

    /// Read one of the app's declared readers, or one of its `network.http`
    /// requests, on the cluster `context` names.
    pub async fn read(&self, context: &CallContext, capability: &str) -> Result<Value, HostError> {
        check(context, &[Field::identifier("capability", capability)])?;
        let params = HostReadParams {
            context: context.clone(),
            capability: capability.to_owned(),
        };
        self.call(
            method::HOST_READ,
            serde_json::to_value(params).expect("plain JSON"),
        )
        .await
    }

    /// Inspect object `name` of a declared custom-resource reader.
    pub async fn resource(
        &self,
        context: &CallContext,
        capability: &str,
        name: &str,
    ) -> Result<Value, HostError> {
        check(
            context,
            &[
                Field::identifier("capability", capability),
                Field::object_name("name", name),
            ],
        )?;
        let params = HostResourceParams {
            context: context.clone(),
            capability: capability.to_owned(),
            name: name.to_owned(),
        };
        self.call(
            method::HOST_RESOURCE,
            serde_json::to_value(params).expect("plain JSON"),
        )
        .await
    }

    /// Run one of the app's declared actions on object `name`, as read
    /// (`uid`, `resource_version`). srelens asks a person first.
    pub async fn action(
        &self,
        context: &CallContext,
        capability: &str,
        name: &str,
        action: &str,
        uid: &str,
        resource_version: &str,
    ) -> Result<Value, HostError> {
        check(
            context,
            &[
                Field::identifier("capability", capability),
                Field::object_name("name", name),
                Field::identifier("action", action),
                Field::token("uid", uid),
                Field::token("resourceVersion", resource_version),
            ],
        )?;
        let params = HostActionParams {
            context: context.clone(),
            capability: capability.to_owned(),
            name: name.to_owned(),
            action: action.to_owned(),
            uid: uid.to_owned(),
            resource_version: resource_version.to_owned(),
        };
        self.call(
            method::HOST_ACTION,
            serde_json::to_value(params).expect("plain JSON"),
        )
        .await
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, HostError> {
        let place = self
            .inner
            .places
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| HostError::Disconnected)?;
        let id = format!("c-{}", self.inner.next.fetch_add(1, Ordering::Relaxed) + 1);
        let (answer, answered) = oneshot::channel();
        {
            let mut waiting = self.inner.waiting.lock().expect("not poisoned");
            // The session may have ended between acquiring `place` and
            // taking this lock: `disconnect` takes it too, and whichever of
            // the two runs first decides this call's fate.
            let Some(waiting) = waiting.as_mut() else {
                return Err(HostError::Disconnected);
            };
            waiting.insert(
                id.clone(),
                Waiting {
                    answer,
                    _place: place,
                },
            );
        }
        let mut guard = CancelOnDrop {
            host: self.clone(),
            id: Some(id.clone()),
            queued: false,
        };
        let request = Request::new(RequestId::String(id.clone()), method, params);
        // Cancel-safe: a `send` dropped before it resolves queues nothing,
        // which is exactly why `guard.queued` is only set after it returns.
        if let Err(unsent) = self.inner.outbox.send(&request).await {
            guard.id = None;
            if let Some(waiting) = self.inner.waiting.lock().expect("not poisoned").as_mut() {
                waiting.remove(&id);
            }
            return Err(match unsent {
                Unsent::TooLarge(bytes) => HostError::TooLarge(bytes),
                Unsent::Closed => HostError::Disconnected,
            });
        }
        guard.queued = true;
        let outcome = answered.await;
        guard.id = None;
        match outcome {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(error)) => Err(HostError::from_rpc(error)),
            Err(_) => Err(HostError::Disconnected),
        }
    }

    /// srelens answered one of the sidecar's calls.
    pub(crate) fn answered(&self, response: Response) {
        let RequestId::String(id) = response.id else {
            return;
        };
        let waiting = self
            .inner
            .waiting
            .lock()
            .expect("not poisoned")
            .as_mut()
            .and_then(|waiting| waiting.remove(&id));
        if let Some(waiting) = waiting {
            let _ = waiting.answer.send(response.outcome);
        }
    }

    /// The session ended: every waiting call is answered `Disconnected` (by
    /// the drop of its `answer` sender, once nothing else references the
    /// map), and a call already past its `acquire_owned` but not yet holding
    /// the lock below refuses at once instead of queuing anything.
    pub(crate) fn disconnect(&self) {
        #[cfg(test)]
        {
            let hook = self
                .inner
                .on_disconnect
                .lock()
                .expect("not poisoned")
                .take();
            if let Some(hook) = hook {
                hook();
            }
        }
        self.inner.places.close();
        *self.inner.waiting.lock().expect("not poisoned") = None;
    }
}

// The shapes srelens's broker holds a call's fields to, in its words
// (`crates/plugin-host/src/sidecar/broker.rs`).
const IDENTIFIER_RULE: &str = "1 to 64 ASCII letters, digits and hyphens, as the manifest names it";
const OBJECT_NAME_RULE: &str =
    "a Kubernetes object name: 1 to 253 ASCII letters, digits, dots and hyphens";
const TOKEN_RULE: &str = "1 to 128 printable ASCII characters, as the object carries it";

/// A string field of a call, with the shape srelens holds it to.
struct Field<'a> {
    /// The field's name on the wire.
    name: &'static str,
    value: &'a str,
    fits: fn(&str) -> bool,
    rule: &'static str,
}

impl<'a> Field<'a> {
    fn identifier(name: &'static str, value: &'a str) -> Field<'a> {
        Field {
            name,
            value,
            fits: shape::is_identifier,
            rule: IDENTIFIER_RULE,
        }
    }

    fn object_name(name: &'static str, value: &'a str) -> Field<'a> {
        Field {
            name,
            value,
            fits: shape::is_object_name,
            rule: OBJECT_NAME_RULE,
        }
    }

    fn token(name: &'static str, value: &'a str) -> Field<'a> {
        Field {
            name,
            value,
            fits: shape::is_token,
            rule: TOKEN_RULE,
        }
    }
}

/// Refuse a call srelens would refuse, before it takes a slot or is queued:
/// `context` first, then `fields` in order, naming the first that does not
/// fit. srelens's own answer for such a call is `-32602`, after a round trip
/// and one of the call slots.
///
/// There is nothing to check for `MAX_CALL_FIELD_BYTES`: it caps a call's
/// `id` and `method`, not a field, and the SDK writes both itself (`c-N`, and
/// one of the `method::HOST_*` names).
fn check(context: &CallContext, fields: &[Field<'_>]) -> Result<(), HostError> {
    context
        .validate()
        .map_err(|why| HostError::InvalidCall(why.to_string()))?;
    for field in fields {
        if !(field.fits)(field.value) {
            return Err(HostError::InvalidCall(format!(
                "`{}` must be {}",
                field.name, field.rule
            )));
        }
    }
    Ok(())
}

/// Cancels a call at srelens when its future is dropped before the answer,
/// unless its request never reached the wire.
///
/// `queued` is false until `call`'s `outbox.send` returns `Ok`; that send is
/// cancel-safe, so a drop while it is still pending queued nothing. What
/// dropping does then depends on `queued`:
///
/// - not queued: srelens never saw this id, so a cancel for it would answer
///   nothing -- srelens ignores a cancel for an id it does not know, and
///   never answers it either. This removes the call's own `waiting` entry
///   instead, freeing its slot at once; left in place, it would be leaked
///   for the rest of the session.
/// - queued: the slot is kept -- srelens is working on the call and will
///   answer it, including `-32800` for a cancelled one -- and a
///   `$/cancelRequest` is sent, but only if the id is still in `waiting`
///   (srelens may have already answered it, racing this drop) and the host
///   is not disconnected. At session end, `Session::end` disconnects the
///   host before it aborts the running handler tasks. Aborting a task drops
///   its pending host-call futures, each of which would otherwise spawn a
///   `$/cancelRequest` send here -- racing the session's own Close marker on
///   the way out, so a call left unanswered by design (the drained-output
///   check in `tests/common/mod.rs`) would fail at random. Once the host is
///   disconnected its semaphore is closed, so a drop after that point sends
///   nothing. A drop before it spawns the task that sends the cancel, which
///   may run, or be descheduled, until after the session has answered
///   `shutdown`. That task waits for room in the queue first, holding no
///   lock, then checks that the host is connected and queues the cancel as
///   one step, under the `waiting` lock `disconnect` takes. The session
///   answers `shutdown` only once `disconnect` has returned, so the cancel is
///   queued ahead of that answer or not at all.
struct CancelOnDrop {
    host: Host,
    id: Option<String>,
    queued: bool,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else { return };
        if !self.queued {
            if let Some(waiting) = self
                .host
                .inner
                .waiting
                .lock()
                .expect("not poisoned")
                .as_mut()
            {
                waiting.remove(&id);
            }
            return;
        }
        if self.host.inner.places.is_closed() {
            return;
        }
        let still_waiting = self
            .host
            .inner
            .waiting
            .lock()
            .expect("not poisoned")
            .as_ref()
            .is_some_and(|waiting| waiting.contains_key(&id));
        if !still_waiting {
            return;
        }
        let host = self.host.clone();
        let cancel = Notification::new(
            method::CANCEL,
            serde_json::to_value(CancelParams {
                id: RequestId::String(id),
            })
            .expect("plain JSON"),
        );
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                // This task runs when the handlers' runtime gets to it, which
                // may be after the session answered `shutdown`: its last
                // line. Room first, holding no lock, since that can wait.
                let Ok(room) = host.inner.outbox.room().await else {
                    return;
                };
                #[cfg(test)]
                let _done = host.wait_at_cancel_gate().await;
                // Then, as one step under the lock `disconnect` takes, which
                // returns before the session answers `shutdown`: the host is
                // still connected and the cancel is queued ahead of that
                // answer, or it is not and the room is given back.
                let waiting = host.inner.waiting.lock().expect("not poisoned");
                if waiting.is_some() {
                    let _ = room.send(&cancel);
                }
            });
        }
    }
}

/// Tests only: holds the task that sends a dropped call's cancel at one
/// point, so a test can run the session's end while it waits there.
#[cfg(test)]
struct CancelGate {
    /// Told when the task reaches the gate.
    reached: oneshot::Sender<()>,
    /// The task waits here until the test sends, or drops this.
    release: oneshot::Receiver<()>,
    /// Dropped once the task has finished, whether it sent or not.
    done: oneshot::Sender<()>,
}

#[cfg(test)]
impl Host {
    /// Hold the next cancel task at its gate. Returns its `reached`, the
    /// sender that releases it, and its `done` (an error once it finished).
    pub(crate) fn hold_next_cancel(
        &self,
    ) -> (
        oneshot::Receiver<()>,
        oneshot::Sender<()>,
        oneshot::Receiver<()>,
    ) {
        let (reached, reached_rx) = oneshot::channel();
        let (release_tx, release) = oneshot::channel();
        let (done, done_rx) = oneshot::channel();
        *self.inner.cancel_gate.lock().expect("not poisoned") = Some(CancelGate {
            reached,
            release,
            done,
        });
        (reached_rx, release_tx, done_rx)
    }

    /// Run `hook` as the next [`Host::disconnect`] starts.
    pub(crate) fn on_disconnect(&self, hook: impl FnOnce() + Send + 'static) {
        *self.inner.on_disconnect.lock().expect("not poisoned") = Some(Box::new(hook));
    }

    /// Wait at the gate, if a test set one; the guard to hold until done.
    async fn wait_at_cancel_gate(&self) -> Option<oneshot::Sender<()>> {
        let gate = self
            .inner
            .cancel_gate
            .lock()
            .expect("not poisoned")
            .take()?;
        let _ = gate.reached.send(());
        let _ = gate.release.await;
        Some(gate.done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    fn limits(max_concurrent_requests: u64) -> InitializeLimits {
        InitializeLimits {
            request_timeout_ms: 30_000,
            max_concurrent_requests,
            max_streams: 5,
            memory_bytes: 256 * 1024 * 1024,
            cpus: 1.0,
            data_bytes: 1 << 30,
            data_entries: 100_000,
        }
    }

    #[test]
    fn the_hosts_limit_sets_the_slots_but_never_above_eight() {
        assert_eq!(call_slots(&limits(1)), 1);
        assert_eq!(call_slots(&limits(2)), 2);
        assert_eq!(call_slots(&limits(8)), 8);
        assert_eq!(call_slots(&limits(20)), 8);
        assert_eq!(call_slots(&limits(u64::MAX)), 8);
    }

    #[test]
    fn a_limit_of_zero_means_eight_not_no_slot_at_all() {
        // A semaphore with no permit would hold every call forever.
        assert_eq!(call_slots(&limits(0)), 8);
    }

    #[tokio::test]
    async fn a_call_dropped_before_its_request_is_queued_frees_its_slot() {
        let (outbox, _writer) = Outbox::new();
        // Fill the outbox's queue so a further send blocks: nothing drains it
        // (the writer is never run).
        for n in 0..64u64 {
            outbox.send(&json!(n)).await.unwrap();
        }
        let host = Host::new(outbox, &limits(HOST_CALLS_IN_FLIGHT as u64));
        let context = CallContext::new("kind-dev", Some("team")).unwrap();
        let outcome =
            tokio::time::timeout(Duration::from_millis(20), host.read(&context, "apps")).await;
        assert!(
            outcome.is_err(),
            "expected the call to still be waiting for room in the queue"
        );
        assert_eq!(host.inner.places.available_permits(), HOST_CALLS_IN_FLIGHT);
        assert!(host
            .inner
            .waiting
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .is_empty());
    }

    // `read`, `resource` and `action` check every field first, and every
    // field has a short cap, so a call that gets past them is far under the
    // message limit. `call` still holds the limit, as the backstop, and this
    // is the only way to reach it.
    #[tokio::test]
    async fn a_call_over_the_message_limit_is_refused_as_too_large_and_frees_its_slot() {
        let (outbox, _writer) = Outbox::new();
        let host = Host::new(outbox, &limits(HOST_CALLS_IN_FLIGHT as u64));
        let params = json!({"data": "x".repeat(5 * 1024 * 1024)});
        let error = host
            .call(method::HOST_READ, params)
            .await
            .expect_err("a call over the limit cannot be sent");
        assert!(
            matches!(error, HostError::TooLarge(bytes) if bytes > 5 * 1024 * 1024),
            "{error:?}"
        );
        assert!(error.to_string().contains("4 MiB"), "{error}");
        assert_eq!(host.inner.places.available_permits(), HOST_CALLS_IN_FLIGHT);
        assert!(host
            .inner
            .waiting
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .is_empty());
    }
}
