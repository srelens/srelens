//! Calls from the sidecar to srelens (`host/read`, `host/resource`,
//! `host/action`). srelens works on at most 8 of a sidecar's calls at once
//! and stops a sidecar with 16 unanswered, so at most
//! [`HOST_CALLS_IN_FLIGHT`] are sent at once and the rest wait here. A call
//! whose future is dropped is cancelled at srelens, and its place is kept
//! until srelens answers it, which it always does (`-32800`).

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
    waiting: Mutex<HashMap<String, Waiting>>,
    places: Arc<Semaphore>,
}

impl Host {
    pub(crate) fn new(outbox: Outbox) -> Host {
        Host {
            inner: Arc::new(Inner {
                outbox,
                next: AtomicU64::new(0),
                waiting: Mutex::new(HashMap::new()),
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
        self.inner.waiting.lock().expect("not poisoned").insert(
            id.clone(),
            Waiting {
                answer,
                _place: place,
            },
        );
        let mut guard = CancelOnDrop {
            host: self.clone(),
            id: Some(id.clone()),
        };
        let request = Request::new(RequestId::String(id.clone()), method, params);
        if self.inner.outbox.send(&request).await.is_err() {
            guard.id = None;
            self.inner.waiting.lock().expect("not poisoned").remove(&id);
            return Err(HostError::Disconnected);
        }
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
        let waiting = self.inner.waiting.lock().expect("not poisoned").remove(&id);
        if let Some(waiting) = waiting {
            let _ = waiting.answer.send(response.outcome);
        }
    }

    /// The session ended: every waiting call is answered `Disconnected`.
    pub(crate) fn disconnect(&self) {
        self.inner.places.close();
        self.inner.waiting.lock().expect("not poisoned").clear();
    }
}

/// Cancels a call at srelens when its future is dropped before the answer.
///
/// At session end, `Session::end` disconnects the host before it aborts the
/// running handler tasks. Aborting a task drops its pending host-call
/// futures, each of which would otherwise spawn a `$/cancelRequest` send here
/// -- racing the session's own Close marker on the way out, so a call left
/// unanswered by design (the drained-output check in `tests/common/mod.rs`)
/// would fail at random. Once the host is disconnected its semaphore is
/// closed, so a drop after that point sends nothing.
struct CancelOnDrop {
    host: Host,
    id: Option<String>,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else { return };
        if self.host.inner.places.is_closed() {
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
