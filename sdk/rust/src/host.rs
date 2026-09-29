//! Calls from the sidecar to srelens (`host/read`, `host/resource`,
//! `host/action`). srelens works on at most 8 of a sidecar's calls at once
//! and stops a sidecar with 16 unanswered, so at most
//! [`HOST_CALLS_IN_FLIGHT`] are sent at once and the rest wait here.
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
    method, CallContext, CancelParams, HostActionParams, HostReadParams, HostResourceParams,
    Notification, Request, RequestId, Response, RpcError,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};

use crate::outbox::Outbox;
use crate::HostError;

pub(crate) const HOST_CALLS_IN_FLIGHT: usize = 8;

struct Waiting {
    answer: oneshot::Sender<Result<Value, RpcError>>,
    _place: OwnedSemaphorePermit,
}

/// The way to srelens from a handler: `ctx.host()`.
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
}

impl Host {
    pub(crate) fn new(outbox: Outbox) -> Host {
        Host {
            inner: Arc::new(Inner {
                outbox,
                next: AtomicU64::new(0),
                waiting: Mutex::new(Some(HashMap::new())),
                places: Arc::new(Semaphore::new(HOST_CALLS_IN_FLIGHT)),
            }),
        }
    }

    /// Read one of the app's declared readers, or one of its `network.http`
    /// requests, on the cluster `context` names.
    pub async fn read(&self, context: &CallContext, capability: &str) -> Result<Value, HostError> {
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
        if self.inner.outbox.send(&request).await.is_err() {
            guard.id = None;
            if let Some(waiting) = self.inner.waiting.lock().expect("not poisoned").as_mut() {
                waiting.remove(&id);
            }
            return Err(HostError::Disconnected);
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
        self.inner.places.close();
        *self.inner.waiting.lock().expect("not poisoned") = None;
    }
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
///   nothing.
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
        let outbox = self.host.inner.outbox.clone();
        let cancel = Notification::new(
            method::CANCEL,
            serde_json::to_value(CancelParams {
                id: RequestId::String(id),
            })
            .expect("plain JSON"),
        );
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = outbox.send(&cancel).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[tokio::test]
    async fn a_call_dropped_before_its_request_is_queued_frees_its_slot() {
        let (outbox, _writer) = Outbox::new();
        // Fill the outbox's queue so a further send blocks: nothing drains it
        // (the writer is never run).
        for n in 0..64u64 {
            outbox.send(&json!(n)).await.unwrap();
        }
        let host = Host::new(outbox);
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
}
