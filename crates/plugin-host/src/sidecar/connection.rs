//! One running sidecar's JSON-RPC session: numbering requests, matching
//! answers to them, and enforcing the request and stream limits.
//!
//! The supervisor owns the process; this owns the conversation. It never
//! blocks on the sidecar: every request carries its own deadline, a caller
//! that stops waiting (drops the future) cancels its request, and when the
//! process ends every caller still waiting is told why at once.

use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, OwnedSemaphorePermit, Semaphore};

use super::protocol::{self, code, method, Incoming, RpcError, Violation};
use super::tunnel::{self, OpenTunnel, Tunnel, TUNNEL_LANE};
use super::Limits;
use srelens_sidecar_protocol::{
    CancelParams, HostConnectParams, HostConnectResult, RequestId, StreamCancelParams,
    StreamOpenParams, MAX_TUNNELS,
};

/// Frames a stream may have waiting for its reader. A sidecar that gets this
/// far ahead of the host has its stream stopped rather than buffered.
pub const STREAM_BUFFER: usize = 64;

/// Answers to a sidecar's own calls that may be waiting, being worked out or
/// not yet written to its stdin. A sidecar that makes one more call than this
/// without reading the answers is stopped: otherwise one that never reads
/// its stdin could grow the host's memory without limit.
pub const ANSWER_BUFFER: usize = 16;

pub use srelens_sidecar_protocol::MAX_CALL_FIELD_BYTES;

/// Why a request did not get an answer.
#[derive(Debug, Clone, PartialEq)]
pub enum RequestError {
    /// The method is one of the host's own (`protocol::is_reserved`).
    Reserved { method: String },
    /// As many requests as the limit allows are already in flight.
    Busy { limit: usize },
    /// As many streams as the limit allows are already open.
    TooManyStreams { limit: usize },
    /// No answer came within the limit. The request was cancelled.
    TimedOut { method: String, after: Duration },
    /// The sidecar answered with an error.
    Failed(RpcError),
    /// The sidecar is not running now: starting, restarting, disabled,
    /// refused or stopped. The text says which.
    Unavailable(String),
    /// The process ended, or was stopped, while this was waiting.
    Ended(String),
}

/// A duration as a person reads it: "30 s", "250 ms".
pub(crate) fn human(duration: Duration) -> String {
    if duration.subsec_millis() == 0 && duration.as_secs() > 0 {
        format!("{} s", duration.as_secs())
    } else {
        format!("{} ms", duration.as_millis())
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestError::Reserved { method } => write!(
                f,
                "`{method}` is one of srelens's own sidecar methods and cannot be requested"
            ),
            RequestError::Busy { limit } => write!(
                f,
                "The extension already has {limit} requests in flight, the most srelens sends one extension at once; try again when one finishes"
            ),
            RequestError::TooManyStreams { limit } => write!(
                f,
                "The extension already has {limit} open streams, the most one extension may have; close one first"
            ),
            RequestError::TimedOut { method, after } => write!(
                f,
                "The extension did not answer `{method}` within {}",
                human(*after)
            ),
            RequestError::Failed(error) => {
                write!(f, "The extension answered with an error: {}", error.message)
            }
            RequestError::Unavailable(why) | RequestError::Ended(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for RequestError {}

/// What a stream delivers. Exactly one of `Closed` and `Failed` comes last,
/// and nothing after it: the same rule as the host's stream frames
/// (`docs/extensions/streams.md`).
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Data(Value),
    /// The sidecar ended the stream.
    Closed,
    /// The stream failed: the sidecar said so, the host stopped it, or the
    /// process ended. Never to be shown as a stream that simply ended.
    Failed(String),
}

/// Host capabilities a sidecar may call: [`super::CapabilityBroker`] (#573),
/// or [`NoBroker`], which refuses every call. A call is dropped unfinished
/// when the sidecar cancels it or the session ends.
pub trait Broker: Send + Sync + 'static {
    fn call<'a>(
        &'a self,
        method: &'a str,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send + 'a>>;

    /// Open a connection to a pod for `host/connect` (#1091). Refused unless
    /// the broker carries tunnels.
    fn connect<'a>(
        &'a self,
        _ask: HostConnectParams,
    ) -> Pin<Box<dyn Future<Output = Result<Tunnel, RpcError>> + Send + 'a>> {
        Box::pin(async {
            Err(RpcError::new(
                code::METHOD_NOT_FOUND,
                "srelens opens no pod connections for this sidecar",
            ))
        })
    }
}

/// Refuses every call a sidecar makes: for a sidecar the host gives no
/// capabilities, and for tests.
pub struct NoBroker;

impl Broker for NoBroker {
    fn call<'a>(
        &'a self,
        method: &'a str,
        _params: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send + 'a>> {
        Box::pin(async move {
            Err(RpcError::new(
                code::METHOD_NOT_FOUND,
                format!("srelens takes no calls from this sidecar; `{method}` was refused"),
            ))
        })
    }
}

/// What the writer sends to the sidecar's stdin.
#[derive(Debug)]
pub(crate) enum Outgoing {
    Line(String),
    /// Close stdin: the end of a graceful stop.
    Close,
}

/// What the writer reads from. The host's own messages (requests, lifecycle
/// calls, cancellations) come first and are never held back by answers to the
/// sidecar's calls. They need no bound of their own: they are limited by the
/// host's request and stream limits, and a sidecar that stops reading its stdin
/// stops answering its health check, which ends the session within
/// `health_interval` plus `health_timeout`.
pub(crate) struct Outbox {
    host: mpsc::UnboundedReceiver<Outgoing>,
    answers: mpsc::Receiver<String>,
    tunnels: mpsc::Receiver<String>,
}

/// How the reader stopped.
#[derive(Debug)]
pub(crate) enum ReadEnd {
    /// stdout closed.
    Eof,
    /// The sidecar broke the protocol.
    Violation(Violation),
}

type Answer = Result<Value, RequestError>;

/// What the broker did for one of the sidecar's calls.
enum Done {
    Answer(Value),
    /// `host/connect` opened tunnel `.0`, holding place `.2`.
    Tunnel(u64, Tunnel, OwnedSemaphorePermit),
}

struct OpenStream {
    events: mpsc::Sender<StreamEvent>,
    _slot: OwnedSemaphorePermit,
}

#[derive(Default)]
pub(super) struct State {
    api_version: Option<semver::Version>,
    /// The id the next request gets; every id below it has been sent.
    next_id: u64,
    pending: HashMap<u64, oneshot::Sender<Answer>>,
    /// The id the next stream gets; every id below it has been opened.
    next_stream: u64,
    open: HashMap<u64, OpenStream>,
    /// The sidecar's own calls being answered, by their id as JSON text (so
    /// `1` and `"1"` differ), each with the way to cancel it: taken when it is
    /// cancelled, the entry itself removed once its answer is queued.
    calls: HashMap<String, Option<oneshot::Sender<()>>>,
    /// Why the session ended, once it has.
    ended: Option<String>,
    /// The highest tunnel id the sidecar has used; each is used once.
    last_tunnel: u64,
    pub(super) tunnels: HashMap<u64, OpenTunnel>,
}

struct Inner {
    limits: Limits,
    out: mpsc::UnboundedSender<Outgoing>,
    answers: mpsc::Sender<String>,
    /// The tunnel lane: lowest of the three, and bounded.
    tunnels: mpsc::Sender<String>,
    state: Mutex<State>,
    requests: Arc<Semaphore>,
    streams: Arc<Semaphore>,
    callbacks: Arc<Semaphore>,
    /// Places for tunnels, open or being connected.
    tunnel_slots: Arc<Semaphore>,
}

#[derive(Clone)]
pub(crate) struct Connection {
    inner: Arc<Inner>,
}

impl Connection {
    pub(crate) fn set_api_version(&self, version: semver::Version) {
        self.state().api_version = Some(version);
    }
    /// A session under `limits`, and the lines to write to the sidecar.
    pub(crate) fn new(limits: Limits) -> (Connection, Outbox) {
        let (out, host) = mpsc::unbounded_channel();
        let (answers, answered) = mpsc::channel(ANSWER_BUFFER);
        let (tunnels, tunnelled) = mpsc::channel(TUNNEL_LANE);
        let inner = Inner {
            answers,
            tunnels,
            requests: Arc::new(Semaphore::new(limits.max_concurrent_requests)),
            streams: Arc::new(Semaphore::new(limits.max_streams)),
            callbacks: Arc::new(Semaphore::new(limits.max_concurrent_requests)),
            tunnel_slots: Arc::new(Semaphore::new(MAX_TUNNELS)),
            limits,
            out,
            state: Mutex::new(State {
                next_id: 1,
                next_stream: 1,
                ..State::default()
            }),
        };
        (
            Connection {
                inner: Arc::new(inner),
            },
            Outbox {
                host,
                answers: answered,
                tunnels: tunnelled,
            },
        )
    }

    pub(super) fn state(&self) -> std::sync::MutexGuard<'_, State> {
        // Nothing panics while holding it; a poisoned lock still holds a
        // consistent table.
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn send(&self, line: String) {
        // Fails only once the writer is gone, when the session is ending.
        let _ = self.inner.out.send(Outgoing::Line(line));
    }

    /// Close the sidecar's stdin once everything queued before is written.
    pub(crate) fn close_stdin(&self) {
        let _ = self.inner.out.send(Outgoing::Close);
    }

    /// An app request: refused if it names a host method, or when the limit
    /// is reached; cancelled at the request timeout or when dropped.
    pub(crate) async fn request(&self, name: &str, params: Value) -> Answer {
        if protocol::is_reserved(name) {
            return Err(RequestError::Reserved {
                method: name.to_owned(),
            });
        }
        let _permit = self.request_permit()?;
        self.call(name, &params, self.inner.limits.request_timeout)
            .await
    }

    fn request_permit(&self) -> Result<OwnedSemaphorePermit, RequestError> {
        self.inner
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| RequestError::Busy {
                limit: self.inner.limits.max_concurrent_requests,
            })
    }

    /// A request outside the limit on concurrent requests, with its own
    /// deadline: the lifecycle methods, which must get through even when
    /// every app request is stuck.
    pub(crate) async fn call(&self, name: &str, params: &Value, timeout: Duration) -> Answer {
        let (id, answer) = {
            let mut state = self.state();
            if let Some(why) = &state.ended {
                return Err(RequestError::Ended(why.clone()));
            }
            let id = state.next_id;
            state.next_id += 1;
            let (tx, rx) = oneshot::channel();
            state.pending.insert(id, tx);
            (id, rx)
        };
        // Cancels the request if this future is dropped or times out. An
        // answered request has already left the table, so it does nothing then.
        let _guard = PendingGuard {
            connection: self,
            id,
        };
        self.send(protocol::request(id, name, params));
        match tokio::time::timeout(timeout, answer).await {
            Ok(Ok(answer)) => answer,
            // The table was dropped without an answer: the session ended.
            Ok(Err(_)) => Err(RequestError::Ended(self.ended_reason())),
            Err(_) => Err(RequestError::TimedOut {
                method: name.to_owned(),
                after: timeout,
            }),
        }
    }

    fn ended_reason(&self) -> String {
        self.state()
            .ended
            .clone()
            .unwrap_or_else(|| "The extension stopped".to_owned())
    }

    /// Open a stream: the sidecar is asked `stream/open` for `name`, and once
    /// it accepts, its frames arrive on the returned stream.
    pub(crate) async fn open_stream(
        &self,
        name: &str,
        params: Value,
    ) -> Result<SidecarStream, RequestError> {
        if protocol::is_reserved(name) {
            return Err(RequestError::Reserved {
                method: name.to_owned(),
            });
        }
        let slot = self
            .inner
            .streams
            .clone()
            .try_acquire_owned()
            .map_err(|_| RequestError::TooManyStreams {
                limit: self.inner.limits.max_streams,
            })?;
        let _permit = self.request_permit()?;
        let (events, receiver) = mpsc::channel(STREAM_BUFFER);
        let stream = {
            let mut state = self.state();
            if let Some(why) = &state.ended {
                return Err(RequestError::Ended(why.clone()));
            }
            let stream = state.next_stream;
            state.next_stream += 1;
            state.open.insert(
                stream,
                OpenStream {
                    events,
                    _slot: slot,
                },
            );
            stream
        };
        let guard = StreamGuard {
            connection: self.clone(),
            stream,
        };
        let params = protocol::params(&StreamOpenParams {
            stream,
            method: name.to_owned(),
            params,
        });
        match self
            .call(
                method::STREAM_OPEN,
                &params,
                self.inner.limits.request_timeout,
            )
            .await
        {
            Ok(_) => Ok(SidecarStream {
                id: stream,
                events: receiver,
                _guard: guard,
            }),
            Err(RequestError::Failed(error)) => {
                // Refused: it never opened, so there is nothing to cancel.
                // With its entry gone, the guard sends nothing.
                self.state().open.remove(&stream);
                drop(guard);
                Err(RequestError::Failed(error))
            }
            // Timed out or ended: the guard cancels it in case it did open.
            Err(other) => Err(other),
        }
    }

    /// Act on one message the sidecar wrote. A request is answered by
    /// `broker` on a task of its own.
    pub(crate) fn handle(
        &self,
        incoming: Incoming,
        broker: &Arc<dyn Broker>,
    ) -> Result<(), Violation> {
        match incoming {
            Incoming::Response { id, outcome } => {
                let mut state = self.state();
                match state.pending.remove(&id) {
                    Some(waiter) => {
                        let _ = waiter.send(outcome.map_err(RequestError::Failed));
                    }
                    // A request that was cancelled or timed out: the answer
                    // crossed the cancellation, which is normal.
                    None if id < state.next_id => {}
                    None => {
                        return Err(Violation(format!(
                            "answered request {id}, which srelens never sent"
                        )))
                    }
                }
                Ok(())
            }
            Incoming::Request { id, method, params } => {
                if id
                    .as_str()
                    .is_some_and(|id| id.len() > MAX_CALL_FIELD_BYTES)
                {
                    return Err(Violation(format!(
                        "sent a call whose id is longer than {MAX_CALL_FIELD_BYTES} bytes"
                    )));
                }
                if method.len() > MAX_CALL_FIELD_BYTES {
                    return Err(Violation(format!(
                        "sent a call whose method is longer than {MAX_CALL_FIELD_BYTES} bytes"
                    )));
                }
                self.answer_call(id, method, params, broker.clone())
            }
            Incoming::Notification { method, params } => match method.as_str() {
                // The sidecar cancelling one of its own calls. One already
                // answered, or never made, crossed the answer: ignored.
                method::CANCEL => {
                    if let Some(id) = params.get("id") {
                        let cancel = self
                            .state()
                            .calls
                            .get_mut(&id.to_string())
                            .and_then(Option::take);
                        if let Some(cancel) = cancel {
                            let _ = cancel.send(());
                        }
                    }
                    Ok(())
                }
                method::STREAM_DATA => {
                    let stream = self.stream_of(&params)?;
                    let Some(data) = params.get("data") else {
                        return Err(Violation("sent a stream/data frame with no data".into()));
                    };
                    self.deliver(stream, data.clone());
                    Ok(())
                }
                method::STREAM_CLOSE => {
                    let stream = self.stream_of(&params)?;
                    self.finish(stream, StreamEvent::Closed);
                    Ok(())
                }
                method::STREAM_ERROR => {
                    let stream = self.stream_of(&params)?;
                    let Some(message) = params.get("message").and_then(Value::as_str) else {
                        return Err(Violation(
                            "sent a stream/error frame with no message".into(),
                        ));
                    };
                    self.finish(stream, StreamEvent::Failed(message.to_owned()));
                    Ok(())
                }
                method::TUNNEL_DATA | method::TUNNEL_CREDIT | method::TUNNEL_CLOSE => {
                    self.tunnel_frame(&method, params)
                }
                // Nothing else is sent to the host: a method from a newer SDK.
                _ => Ok(()),
            },
        }
    }

    /// The stream a frame names, if the host ever opened it.
    fn stream_of(&self, params: &Value) -> Result<u64, Violation> {
        let stream = params
            .get("stream")
            .and_then(Value::as_u64)
            .ok_or_else(|| Violation("sent a stream frame with no stream id".into()))?;
        if stream == 0 || stream >= self.state().next_stream {
            return Err(Violation(format!(
                "sent a frame for stream {stream}, which srelens never opened"
            )));
        }
        Ok(stream)
    }

    fn deliver(&self, stream: u64, data: Value) {
        let mut state = self.state();
        // A stream that already ended or was cancelled: the frame crossed
        // the cancellation.
        let Some(open) = state.open.get(&stream) else {
            return;
        };
        // The last place is kept for the terminal event, so the reader is
        // always told why a stream ended.
        if open.events.capacity() > 1 {
            let _ = open.events.try_send(StreamEvent::Data(data));
            return;
        }
        if let Some(open) = state.open.remove(&stream) {
            let _ = open.events.try_send(StreamEvent::Failed(
                "The extension sent stream data faster than srelens read it, so srelens stopped the stream".into(),
            ));
        }
        drop(state);
        self.send(protocol::notification(
            method::STREAM_CANCEL,
            &protocol::params(&StreamCancelParams { stream }),
        ));
    }

    fn finish(&self, stream: u64, event: StreamEvent) {
        if let Some(open) = self.state().open.remove(&stream) {
            let _ = open.events.try_send(event);
        }
    }

    /// Answer the sidecar's call `id`. Its answer has a place in the bounded
    /// answer queue before any work starts, so the queue never grows past
    /// [`ANSWER_BUFFER`], whatever the broker does or how slowly the sidecar
    /// reads.
    fn answer_call(
        &self,
        id: Value,
        name: String,
        params: Value,
        broker: Arc<dyn Broker>,
    ) -> Result<(), Violation> {
        // An id is how the sidecar cancels a call and matches its answer, so
        // two in flight at once could not be told apart.
        let key = id.to_string();
        if self.state().calls.contains_key(&key) {
            return Err(Violation(format!(
                "sent call {key} while srelens is still answering its earlier call with that id"
            )));
        }
        let Ok(place) = self.inner.answers.clone().try_reserve_owned() else {
            return Err(Violation(format!(
                "is not reading its standard input: {ANSWER_BUFFER} answers to its calls are waiting for it"
            )));
        };
        let needs = match name.as_str() {
            method::HOST_CONNECT => Some(semver::Version::new(0, 3, 0)),
            method::HOST_BINDING_AVAILABILITY | method::HOST_RUN_JOB => {
                Some(semver::Version::new(0, 2, 0))
            }
            method::HOST_READ if params.get("cursor").is_some() => {
                Some(semver::Version::new(0, 2, 0))
            }
            _ => None,
        };
        if let Some(needs) = needs {
            if !self
                .state()
                .api_version
                .as_ref()
                .is_some_and(|version| version >= &needs)
            {
                let error = RpcError::new(
                    code::METHOD_NOT_FOUND,
                    format!("This callback requires negotiated sidecar API {needs}"),
                );
                place.send(protocol::response(&id, &Err(error)));
                return Ok(());
            }
        }
        let connect = if name == method::HOST_CONNECT {
            match self.admit_tunnel(&params) {
                Ok(admitted) => Some(admitted),
                Err(error) => {
                    place.send(protocol::response(&id, &Err(error)));
                    return Ok(());
                }
            }
        } else {
            None
        };
        let Ok(permit) = self.inner.callbacks.clone().try_acquire_owned() else {
            let error = RpcError::new(
                code::INTERNAL_ERROR,
                format!(
                    "srelens is already answering {} calls from this extension; try again when one finishes",
                    self.inner.limits.max_concurrent_requests
                ),
            );
            place.send(protocol::response(&id, &Err(error)));
            return Ok(());
        };
        let (cancel, cancelled) = oneshot::channel();
        {
            let mut state = self.state();
            if state.ended.is_some() {
                // Nothing is waiting for the answer; the call is not made.
                return Ok(());
            }
            state.calls.insert(key.clone(), Some(cancel));
        }
        let connection = self.clone();
        let lane = self.inner.tunnels.clone();
        tokio::spawn(async move {
            let cancelled_error =
                || RpcError::new(code::REQUEST_CANCELLED, format!("`{name}` was cancelled"));
            let work = async {
                match connect {
                    Some((ask, slot)) => {
                        let tunnel = ask.tunnel;
                        let opened = broker.connect(ask).await;
                        opened.map(|opened| Done::Tunnel(tunnel, opened, slot))
                    }
                    None => broker.call(&name, params).await.map(Done::Answer),
                }
            };
            let outcome = tokio::select! {
                // A cancellation already accepted wins over an answer ready
                // in the same moment: the sidecar was told it is cancelled.
                biased;
                // Cancelled by the sidecar, or the session ended.
                _ = cancelled => Err(cancelled_error()),
                outcome = work => outcome,
            };
            drop(permit);
            // Settled under the lock the reader takes a cancellation under,
            // and checks a new call's id under: if it took this call's
            // cancellation before now, even after the answer above was
            // chosen, that is the answer; and a call reusing the id is
            // admitted only once this answer is queued ahead of its own.
            let mut state = connection.state();
            let accepted = matches!(state.calls.remove(&key), Some(None));
            let outcome = match (accepted, outcome) {
                // A connection that opened anyway is dropped here, so a
                // cancelled connect leaves nothing.
                (true, _) => Err(cancelled_error()),
                (false, Ok(Done::Answer(answer))) => Ok(answer),
                (false, Ok(Done::Tunnel(tunnel, opened, slot))) => {
                    let result = HostConnectResult {
                        pod: opened.pod.clone(),
                    };
                    let open = tunnel::open(&connection, lane, tunnel, opened, slot);
                    state.tunnels.insert(tunnel, open);
                    Ok(protocol::params(&result))
                }
                (false, Err(error)) => Err(error),
            };
            place.send(protocol::response(&id, &outcome));
            drop(state);
        });
        Ok(())
    }

    /// Admit `host/connect`'s tunnel: its id is used up whatever happens
    /// next, and it takes a tunnel place before the broker is asked.
    fn admit_tunnel(
        &self,
        params: &Value,
    ) -> Result<(HostConnectParams, OwnedSemaphorePermit), RpcError> {
        let ask = serde_json::from_value::<HostConnectParams>(params.clone())
            .map_err(|e| RpcError::new(code::INVALID_PARAMS, format!("`host/connect`: {e}")))?;
        {
            let mut state = self.state();
            if ask.tunnel <= state.last_tunnel {
                return Err(RpcError::new(
                    code::INVALID_PARAMS,
                    format!(
                        "Tunnel {} is not greater than {}, the last tunnel this extension used",
                        ask.tunnel, state.last_tunnel
                    ),
                ));
            }
            state.last_tunnel = ask.tunnel;
        }
        let slot = self
            .inner
            .tunnel_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                RpcError::new(
                    code::INTERNAL_ERROR,
                    format!("The extension already has {MAX_TUNNELS} open tunnels, the most one extension may have; close one first"),
                )
            })?;
        Ok((ask, slot))
    }

    /// End the session: every request still waiting gets `why`, every open
    /// stream fails with it, every call of the sidecar's is cancelled, every
    /// tunnel's connection to its pod is dropped, and nothing new is admitted.
    pub(crate) fn end(&self, why: &str) {
        let (pending, open, calls, tunnels) = {
            let mut state = self.state();
            if state.ended.is_none() {
                state.ended = Some(why.to_owned());
            }
            (
                std::mem::take(&mut state.pending),
                std::mem::take(&mut state.open),
                // Each call's cancellation is taken and its entry left: a
                // worker whose answer was already chosen then finds it
                // accepted, and answers that the call was cancelled. The
                // worker removes the entry when it queues that answer.
                state
                    .calls
                    .values_mut()
                    .filter_map(Option::take)
                    .collect::<Vec<_>>(),
                std::mem::take(&mut state.tunnels),
            )
        };
        // Each pump is stopped, wherever it waits, as its entry drops.
        drop(tunnels);
        // No one will read the answers to the sidecar's calls: stop them. A
        // broker call that has begun a write finishes it on its own task.
        for cancel in calls {
            let _ = cancel.send(());
        }
        for (_, waiter) in pending {
            let _ = waiter.send(Err(RequestError::Ended(why.to_owned())));
        }
        for (_, stream) in open {
            let _ = stream.events.try_send(StreamEvent::Failed(why.to_owned()));
        }
    }

    pub(crate) fn is_ended(&self) -> bool {
        self.state().ended.is_some()
    }

    /// App requests in flight and streams open now, for the Inspector (#575).
    pub(crate) fn load(&self) -> (usize, usize) {
        let in_flight =
            self.inner.limits.max_concurrent_requests - self.inner.requests.available_permits();
        (in_flight, self.state().open.len())
    }

    /// Read the sidecar's stdout until it closes or breaks the protocol.
    pub(crate) async fn read<R>(&self, stdout: R, broker: Arc<dyn Broker>) -> ReadEnd
    where
        R: tokio::io::AsyncRead + Unpin,
    {
        let mut lines = protocol::BoundedLines::new(
            tokio::io::BufReader::new(stdout),
            protocol::MAX_MESSAGE_BYTES,
        );
        loop {
            let line = match lines.next_line().await {
                Ok(Some(protocol::Line::Bytes(line))) => line,
                Ok(Some(protocol::Line::TooLong)) => {
                    return ReadEnd::Violation(Violation(format!(
                        "wrote a message longer than {} bytes",
                        protocol::MAX_MESSAGE_BYTES
                    )))
                }
                // A read error on a pipe is the pipe going away.
                Ok(None) | Err(_) => return ReadEnd::Eof,
            };
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let handled =
                protocol::parse(&line).and_then(|incoming| self.handle(incoming, &broker));
            if let Err(violation) = handled {
                return ReadEnd::Violation(violation);
            }
        }
    }
}

/// Write the outbox to the sidecar's stdin until the session closes it: the
/// host's own messages first, then answers to the sidecar's calls, then
/// tunnel frames.
pub(crate) async fn write<W>(mut outbox: Outbox, mut stdin: W)
where
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt;
    let mut answers_open = true;
    let mut tunnels_open = true;
    loop {
        let mut line = tokio::select! {
            biased;
            outgoing = outbox.host.recv() => match outgoing {
                Some(Outgoing::Line(line)) => line,
                Some(Outgoing::Close) | None => break,
            },
            answer = outbox.answers.recv(), if answers_open => match answer {
                Some(line) => line,
                None => {
                    answers_open = false;
                    continue;
                }
            },
            frame = outbox.tunnels.recv(), if tunnels_open => match frame {
                Some(line) => line,
                None => {
                    tunnels_open = false;
                    continue;
                }
            },
        };
        line.push('\n');
        // A failed write is the process going away; its exit says why.
        if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
            break;
        }
    }
    let _ = stdin.shutdown().await;
}

/// Cancels a request whose caller stopped waiting.
struct PendingGuard<'a> {
    connection: &'a Connection,
    id: u64,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        let still_pending = {
            let mut state = self.connection.state();
            state.pending.remove(&self.id).is_some() && state.ended.is_none()
        };
        if still_pending {
            self.connection.send(protocol::notification(
                method::CANCEL,
                &protocol::params(&CancelParams {
                    id: RequestId::from(self.id),
                }),
            ));
        }
    }
}

/// Cancels a stream whose reader went away before it ended.
struct StreamGuard {
    connection: Connection,
    stream: u64,
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        let was_open = {
            let mut state = self.connection.state();
            state.open.remove(&self.stream).is_some() && state.ended.is_none()
        };
        if was_open {
            self.connection.send(protocol::notification(
                method::STREAM_CANCEL,
                &protocol::params(&StreamCancelParams {
                    stream: self.stream,
                }),
            ));
        }
    }
}

/// An open stream. Dropping it cancels it, and frees its place under the
/// stream limit, as the terminal event does.
pub struct SidecarStream {
    id: u64,
    events: mpsc::Receiver<StreamEvent>,
    _guard: StreamGuard,
}

impl SidecarStream {
    /// The id the host gave it, which its frames carry.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The next event, or `None` once the terminal one has been read.
    pub async fn next(&mut self) -> Option<StreamEvent> {
        self.events.recv().await
    }
}

impl fmt::Debug for SidecarStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SidecarStream")
            .field("id", &self.id)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::time::Instant;

    fn limits() -> Limits {
        Limits::default()
    }

    fn broker() -> Arc<dyn Broker> {
        Arc::new(NoBroker)
    }

    /// The next line the host wrote, as JSON. On the paused clock a line
    /// that never comes fails at once, at the virtual deadline, instead of
    /// hanging the suite.
    async fn sent(lines: &mut Outbox) -> Value {
        let next = tokio::time::timeout(Duration::from_secs(24 * 3600), lines.host.recv()).await;
        match next {
            Ok(Some(Outgoing::Line(line))) => serde_json::from_str(&line).expect("JSON"),
            Ok(other) => panic!("expected a line, got {other:?}"),
            Err(_) => panic!("the host never wrote the line"),
        }
    }

    /// The stream's next event, failing at a virtual deadline like `sent`.
    async fn next(stream: &mut SidecarStream) -> Option<StreamEvent> {
        tokio::time::timeout(Duration::from_secs(24 * 3600), stream.next())
            .await
            .expect("the stream's next event never came")
    }

    fn answer(connection: &Connection, id: u64, result: Value) -> Result<(), Violation> {
        connection.handle(
            Incoming::Response {
                id,
                outcome: Ok(result),
            },
            &broker(),
        )
    }

    fn frame(connection: &Connection, method: &str, params: Value) -> Result<(), Violation> {
        connection.handle(
            Incoming::Notification {
                method: method.into(),
                params,
            },
            &broker(),
        )
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_is_answered_by_the_response_with_its_id() {
        let (connection, mut lines) = Connection::new(limits());
        let asking = tokio::spawn({
            let connection = connection.clone();
            async move { connection.request("scan", json!({"image": "x"})).await }
        });
        let request = sent(&mut lines).await;
        assert_eq!(request["method"], "scan");
        assert_eq!(request["params"], json!({"image": "x"}));
        answer(
            &connection,
            request["id"].as_u64().unwrap(),
            json!({"found": 3}),
        )
        .unwrap();
        assert_eq!(asking.await.unwrap(), Ok(json!({"found": 3})));
    }

    #[tokio::test(start_paused = true)]
    async fn an_error_answer_is_the_sidecars_error() {
        let (connection, mut lines) = Connection::new(limits());
        let asking = tokio::spawn({
            let connection = connection.clone();
            async move { connection.request("scan", json!({})).await }
        });
        let id = sent(&mut lines).await["id"].as_u64().unwrap();
        connection
            .handle(
                Incoming::Response {
                    id,
                    outcome: Err(RpcError::new(-32000, "image not found")),
                },
                &broker(),
            )
            .unwrap();
        let error = asking.await.unwrap().unwrap_err();
        assert_eq!(
            error,
            RequestError::Failed(RpcError::new(-32000, "image not found"))
        );
        assert!(error.to_string().contains("image not found"), "{error}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_with_no_answer_times_out_at_the_limit_and_is_cancelled() {
        let (connection, mut lines) = Connection::new(limits());
        let started = Instant::now();
        let asking = tokio::spawn({
            let connection = connection.clone();
            async move { connection.request("scan", json!({})).await }
        });
        let id = sent(&mut lines).await["id"].clone();
        let error = asking.await.unwrap().unwrap_err();
        assert_eq!(started.elapsed(), Duration::from_secs(30));
        assert_eq!(
            error,
            RequestError::TimedOut {
                method: "scan".into(),
                after: Duration::from_secs(30)
            }
        );
        assert_eq!(
            error.to_string(),
            "The extension did not answer `scan` within 30 s"
        );
        let cancel = sent(&mut lines).await;
        assert_eq!(cancel["method"], "$/cancelRequest");
        assert_eq!(cancel["params"], json!({"id": id}));
        assert!(
            cancel.get("id").is_none(),
            "a cancellation is a notification"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_caller_that_stops_waiting_cancels_its_request_and_frees_its_place() {
        let (connection, mut lines) = Connection::new(limits());
        let mut callers = Vec::new();
        for _ in 0..8 {
            let connection = connection.clone();
            callers.push(tokio::spawn(async move {
                connection.request("scan", json!({})).await
            }));
        }
        let mut ids = Vec::new();
        for _ in 0..8 {
            ids.push(sent(&mut lines).await["id"].as_u64().unwrap());
        }
        let ninth = connection.request("scan", json!({})).await.unwrap_err();
        assert_eq!(ninth, RequestError::Busy { limit: 8 });
        assert!(
            ninth.to_string().contains("8 requests in flight"),
            "{ninth}"
        );

        callers.remove(0).abort();
        let cancel = sent(&mut lines).await;
        assert_eq!(cancel["method"], "$/cancelRequest");
        assert_eq!(cancel["params"]["id"], ids[0]);

        let admitted = tokio::spawn({
            let connection = connection.clone();
            async move { connection.request("scan", json!({})).await }
        });
        let request = sent(&mut lines).await;
        assert_eq!(
            request["method"], "scan",
            "the freed place admits the next request"
        );
        answer(&connection, request["id"].as_u64().unwrap(), json!(1)).unwrap();
        assert_eq!(admitted.await.unwrap(), Ok(json!(1)));
    }

    #[tokio::test(start_paused = true)]
    async fn an_answer_that_crosses_a_cancellation_is_dropped_quietly() {
        let (connection, mut lines) = Connection::new(limits());
        let caller = tokio::spawn({
            let connection = connection.clone();
            async move { connection.request("scan", json!({})).await }
        });
        let id = sent(&mut lines).await["id"].as_u64().unwrap();
        caller.abort();
        let _ = caller.await;
        assert_eq!(answer(&connection, id, json!("late")), Ok(()));
    }

    #[tokio::test(start_paused = true)]
    async fn an_answer_to_a_request_never_sent_breaks_the_protocol() {
        let (connection, _lines) = Connection::new(limits());
        let Err(Violation(why)) = answer(&connection, 99, json!(1)) else {
            panic!("an answer to request 99 was accepted");
        };
        assert!(why.contains("never sent"), "{why}");
    }

    #[tokio::test(start_paused = true)]
    async fn lifecycle_calls_get_through_when_every_request_place_is_taken() {
        let (connection, mut lines) = Connection::new(limits());
        let mut callers = Vec::new();
        for _ in 0..8 {
            let connection = connection.clone();
            callers.push(tokio::spawn(async move {
                connection.request("scan", json!({})).await
            }));
        }
        for _ in 0..8 {
            sent(&mut lines).await;
        }
        let health = tokio::spawn({
            let connection = connection.clone();
            async move {
                connection
                    .call("health", &json!({}), Duration::from_secs(10))
                    .await
            }
        });
        let request = sent(&mut lines).await;
        assert_eq!(request["method"], "health");
        answer(&connection, request["id"].as_u64().unwrap(), json!({})).unwrap();
        assert_eq!(health.await.unwrap(), Ok(json!({})));
    }

    #[tokio::test(start_paused = true)]
    async fn an_app_request_cannot_name_a_host_method() {
        let (connection, _lines) = Connection::new(limits());
        for name in ["shutdown", "initialize", "stream/data", "$/cancelRequest"] {
            let error = connection.request(name, json!({})).await.unwrap_err();
            assert_eq!(
                error,
                RequestError::Reserved {
                    method: name.into()
                }
            );
            let error = connection.open_stream(name, json!({})).await.unwrap_err();
            assert_eq!(
                error,
                RequestError::Reserved {
                    method: name.into()
                }
            );
        }
    }

    /// Open a stream, answering its `stream/open` as a sidecar that accepts.
    async fn opened(connection: &Connection, lines: &mut Outbox, name: &str) -> SidecarStream {
        let opening = tokio::spawn({
            let connection = connection.clone();
            let name = name.to_owned();
            async move { connection.open_stream(&name, json!({"n": 1})).await }
        });
        let request = sent(lines).await;
        assert_eq!(request["method"], "stream/open");
        assert_eq!(request["params"]["method"], name);
        assert_eq!(request["params"]["params"], json!({"n": 1}));
        answer(connection, request["id"].as_u64().unwrap(), json!({})).unwrap();
        opening.await.unwrap().expect("the stream opens")
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_delivers_its_data_then_one_terminal_event_and_nothing_after() {
        let (connection, mut lines) = Connection::new(limits());
        let mut stream = opened(&connection, &mut lines, "watch").await;
        let id = stream.id();
        frame(
            &connection,
            "stream/data",
            json!({"stream": id, "data": {"n": 1}}),
        )
        .unwrap();
        frame(
            &connection,
            "stream/data",
            json!({"stream": id, "data": null}),
        )
        .unwrap();
        frame(&connection, "stream/close", json!({"stream": id})).unwrap();
        // Frames after the end are late, not violations.
        frame(&connection, "stream/data", json!({"stream": id, "data": 3})).unwrap();
        assert_eq!(
            next(&mut stream).await,
            Some(StreamEvent::Data(json!({"n": 1})))
        );
        assert_eq!(
            next(&mut stream).await,
            Some(StreamEvent::Data(Value::Null))
        );
        assert_eq!(next(&mut stream).await, Some(StreamEvent::Closed));
        assert_eq!(next(&mut stream).await, None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_stream_says_why() {
        let (connection, mut lines) = Connection::new(limits());
        let mut stream = opened(&connection, &mut lines, "watch").await;
        let id = stream.id();
        frame(
            &connection,
            "stream/error",
            json!({"stream": id, "message": "registry unreachable"}),
        )
        .unwrap();
        assert_eq!(
            next(&mut stream).await,
            Some(StreamEvent::Failed("registry unreachable".into()))
        );
        assert_eq!(next(&mut stream).await, None);
    }

    #[tokio::test(start_paused = true)]
    async fn the_sixth_stream_is_refused_and_an_ended_one_frees_its_place() {
        let (connection, mut lines) = Connection::new(limits());
        let mut streams = Vec::new();
        for _ in 0..5 {
            streams.push(opened(&connection, &mut lines, "watch").await);
        }
        let sixth = connection
            .open_stream("watch", json!({}))
            .await
            .unwrap_err();
        assert_eq!(sixth, RequestError::TooManyStreams { limit: 5 });
        assert!(sixth.to_string().contains("5 open streams"), "{sixth}");

        let first = streams[0].id();
        frame(&connection, "stream/close", json!({"stream": first})).unwrap();
        let again = opened(&connection, &mut lines, "watch").await;
        assert!(again.id() > streams[4].id());
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_a_stream_cancels_it_and_frees_its_place() {
        let (connection, mut lines) = Connection::new(limits());
        let mut streams = Vec::new();
        for _ in 0..5 {
            streams.push(opened(&connection, &mut lines, "watch").await);
        }
        let dropped = streams.remove(2);
        let id = dropped.id();
        drop(dropped);
        let cancel = sent(&mut lines).await;
        assert_eq!(cancel["method"], "stream/cancel");
        assert_eq!(cancel["params"], json!({"stream": id}));
        opened(&connection, &mut lines, "watch").await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_open_frees_its_place_and_is_not_cancelled() {
        let limits = Limits {
            max_streams: 1,
            ..Limits::default()
        };
        let (connection, mut lines) = Connection::new(limits);
        let opening = tokio::spawn({
            let connection = connection.clone();
            async move { connection.open_stream("watch", json!({})).await }
        });
        let id = sent(&mut lines).await["id"].as_u64().unwrap();
        connection
            .handle(
                Incoming::Response {
                    id,
                    outcome: Err(RpcError::new(-32602, "no such watch")),
                },
                &broker(),
            )
            .unwrap();
        let error = opening.await.unwrap().unwrap_err();
        assert_eq!(
            error,
            RequestError::Failed(RpcError::new(-32602, "no such watch"))
        );
        opened(&connection, &mut lines, "watch").await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_frame_for_a_stream_never_opened_breaks_the_protocol() {
        let (connection, mut lines) = Connection::new(limits());
        let stream = opened(&connection, &mut lines, "watch").await;
        for params in [
            json!({"stream": stream.id() + 1, "data": 1}),
            json!({"stream": 0, "data": 1}),
            json!({"data": 1}),
            json!({"stream": "1", "data": 1}),
        ] {
            let Err(Violation(why)) = frame(&connection, "stream/data", params.clone()) else {
                panic!("{params} was accepted");
            };
            assert!(why.contains("stream"), "{why}");
        }
        let missing = frame(&connection, "stream/data", json!({"stream": stream.id()}));
        assert!(missing.is_err(), "a data frame needs its data");
        let wordless = frame(&connection, "stream/error", json!({"stream": stream.id()}));
        assert!(wordless.is_err(), "an error frame needs its message");
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_that_outruns_its_reader_is_stopped_and_told_why() {
        let (connection, mut lines) = Connection::new(limits());
        let mut stream = opened(&connection, &mut lines, "watch").await;
        let id = stream.id();
        for n in 0..STREAM_BUFFER + 10 {
            frame(&connection, "stream/data", json!({"stream": id, "data": n})).unwrap();
        }
        let cancel = sent(&mut lines).await;
        assert_eq!(cancel["method"], "stream/cancel");
        assert_eq!(cancel["params"]["stream"], id);
        let mut data = 0;
        let last = loop {
            match next(&mut stream).await {
                Some(StreamEvent::Data(_)) => data += 1,
                other => break other,
            }
        };
        assert_eq!(data, STREAM_BUFFER - 1);
        let Some(StreamEvent::Failed(why)) = last else {
            panic!("the stream ended with {last:?}");
        };
        assert!(why.contains("faster than srelens read it"), "{why}");
        assert_eq!(next(&mut stream).await, None);
    }

    #[tokio::test(start_paused = true)]
    async fn ending_the_session_answers_every_waiting_request_and_stream_at_once() {
        let (connection, mut lines) = Connection::new(limits());
        let mut stream = opened(&connection, &mut lines, "watch").await;
        let started = Instant::now();
        let asking = tokio::spawn({
            let connection = connection.clone();
            async move { connection.request("scan", json!({})).await }
        });
        sent(&mut lines).await;
        let why = "The extension process exited unexpectedly: it exited with status 1";
        connection.end(why);
        assert_eq!(asking.await.unwrap(), Err(RequestError::Ended(why.into())));
        assert_eq!(
            started.elapsed(),
            Duration::ZERO,
            "no one waits for a timeout"
        );
        assert_eq!(
            next(&mut stream).await,
            Some(StreamEvent::Failed(why.into()))
        );
        assert_eq!(next(&mut stream).await, None);
        let after = connection.request("scan", json!({})).await;
        assert_eq!(after, Err(RequestError::Ended(why.into())));
        assert!(connection.is_ended());
    }

    #[tokio::test]
    async fn new_callbacks_require_negotiated_protocol_0_2() {
        let (connection, mut lines) = Connection::new(limits());
        connection.set_api_version(semver::Version::new(0, 1, 0));
        for (name, params) in [
            (method::HOST_BINDING_AVAILABILITY, json!({})),
            (method::HOST_RUN_JOB, json!({})),
            (method::HOST_READ, json!({"cursor":""})),
        ] {
            connection
                .handle(
                    Incoming::Request {
                        id: json!(name),
                        method: name.into(),
                        params,
                    },
                    &broker(),
                )
                .unwrap();
            let answer: Value = serde_json::from_str(&lines.answers.recv().await.unwrap()).unwrap();
            assert_eq!(
                answer["error"]["code"],
                code::METHOD_NOT_FOUND,
                "{name}: {answer}"
            );
            assert!(answer["error"]["message"].as_str().unwrap().contains("0.2"));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_from_the_sidecar_is_refused_until_there_is_a_broker() {
        let (connection, mut lines) = Connection::new(limits());
        connection
            .handle(
                Incoming::Request {
                    id: json!("c-1"),
                    method: "k8s.listPods".into(),
                    params: json!({}),
                },
                &broker(),
            )
            .unwrap();
        let reply = lines.answers.recv().await.expect("an answer");
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["id"], "c-1");
        assert_eq!(reply["error"]["code"], code::METHOD_NOT_FOUND);
        let message = reply["error"]["message"].as_str().unwrap();
        assert!(message.contains("k8s.listPods"), "{message}");
    }

    fn call_from_sidecar(
        connection: &Connection,
        id: Value,
        method: &str,
    ) -> Result<(), Violation> {
        connection.handle(
            Incoming::Request {
                id,
                method: method.into(),
                params: json!({}),
            },
            &broker(),
        )
    }

    #[tokio::test(start_paused = true)]
    async fn a_sidecar_that_does_not_read_the_answers_to_its_calls_is_stopped_not_buffered() {
        // Nothing drains what the host writes, as when a sidecar never reads
        // its stdin: each call's answer would otherwise wait in memory.
        let (connection, _lines) = Connection::new(limits());
        let mut refused = None;
        for n in 0..1_000 {
            if let Err(violation) = call_from_sidecar(&connection, json!(n), "k8s.listPods") {
                refused = Some((n, violation));
                break;
            }
            // Let each broker call finish and queue its answer.
            tokio::task::yield_now().await;
        }
        let Some((n, Violation(why))) = refused else {
            panic!("a thousand unread answers were queued");
        };
        assert!(n <= 64, "stopped only after {n} unread answers");
        assert!(why.contains("not reading"), "{why}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_whose_id_or_method_is_too_long_to_echo_breaks_the_protocol() {
        let (connection, _lines) = Connection::new(limits());
        let long = "x".repeat(4096);
        let Err(Violation(why)) = call_from_sidecar(&connection, json!(long), "m") else {
            panic!("a 4 KiB id was accepted");
        };
        assert!(why.contains("id"), "{why}");
        let Err(Violation(why)) = call_from_sidecar(&connection, json!("c-1"), &long) else {
            panic!("a 4 KiB method was accepted");
        };
        assert!(why.contains("method"), "{why}");
        assert_eq!(
            call_from_sidecar(&connection, json!("c-2"), "k8s.listPods"),
            Ok(())
        );
    }

    /// A broker whose calls never finish on their own, and which counts the
    /// ones dropped unfinished.
    struct Pending {
        dropped: Arc<std::sync::atomic::AtomicUsize>,
    }

    struct Dropped(Arc<std::sync::atomic::AtomicUsize>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    impl Broker for Pending {
        fn call<'a>(
            &'a self,
            _method: &'a str,
            _params: Value,
        ) -> Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send + 'a>> {
            let dropped = Dropped(self.dropped.clone());
            Box::pin(async move {
                let _dropped = dropped;
                std::future::pending::<()>().await;
                Ok(Value::Null)
            })
        }
    }

    fn pending() -> (Arc<dyn Broker>, Arc<std::sync::atomic::AtomicUsize>) {
        let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let broker: Arc<dyn Broker> = Arc::new(Pending {
            dropped: dropped.clone(),
        });
        (broker, dropped)
    }

    fn call_with(
        connection: &Connection,
        broker: &Arc<dyn Broker>,
        id: Value,
    ) -> Result<(), Violation> {
        connection.handle(
            Incoming::Request {
                id,
                method: "host/read".into(),
                params: json!({}),
            },
            broker,
        )
    }

    fn cancel_call(connection: &Connection, broker: &Arc<dyn Broker>, id: Value) {
        connection
            .handle(
                Incoming::Notification {
                    method: method::CANCEL.into(),
                    params: json!({ "id": id }),
                },
                broker,
            )
            .expect("a cancellation is not a violation");
    }

    /// The next answer to one of the sidecar's calls, failing at a virtual
    /// deadline like `sent`.
    async fn answered(lines: &mut Outbox) -> Value {
        let next = tokio::time::timeout(Duration::from_secs(24 * 3600), lines.answers.recv());
        let line = next.await.expect("no answer came").expect("an answer");
        serde_json::from_str(&line).expect("JSON")
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_the_sidecar_cancels_is_answered_as_cancelled_and_its_work_dropped() {
        let (connection, mut lines) = Connection::new(limits());
        let (broker, dropped) = pending();
        call_with(&connection, &broker, json!("c-1")).unwrap();
        tokio::task::yield_now().await;
        cancel_call(&connection, &broker, json!("c-1"));
        let reply = answered(&mut lines).await;
        assert_eq!(reply["id"], "c-1");
        assert_eq!(reply["error"]["code"], code::REQUEST_CANCELLED);
        assert_eq!(dropped.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// A cancellation srelens accepted is what the sidecar is told, even when
    /// the broker's answer was ready in the same moment: polled in random
    /// order, the two would each win half the time.
    #[tokio::test(start_paused = true)]
    async fn an_accepted_cancellation_wins_over_an_answer_ready_at_the_same_time() {
        for n in 0..64 {
            let (connection, mut lines) = Connection::new(limits());
            // NoBroker answers at once: its answer is ready when first polled.
            call_from_sidecar(&connection, json!(n), "host/read").unwrap();
            cancel_call(&connection, &broker(), json!(n));
            let reply = answered(&mut lines).await;
            assert_eq!(
                reply["error"]["code"],
                code::REQUEST_CANCELLED,
                "round {n}: {reply}"
            );
        }
    }

    /// A broker whose answer is ready in the same moment the reader accepts
    /// the sidecar's cancellation of it: it hands the connection the
    /// `$/cancelRequest` itself, as the reader would, then answers.
    struct CancelledAsItAnswers {
        connection: Connection,
        id: Value,
    }

    impl Broker for CancelledAsItAnswers {
        fn call<'a>(
            &'a self,
            _method: &'a str,
            _params: Value,
        ) -> Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send + 'a>> {
            Box::pin(async move {
                cancel_call(&self.connection, &broker(), self.id.clone());
                Ok(json!({"rows": []}))
            })
        }
    }

    /// Past the point the broker's answer was chosen, a cancellation the
    /// reader accepts before the answer is queued still wins: which answer the
    /// sidecar gets is settled under the lock the cancellation is taken under.
    #[tokio::test(start_paused = true)]
    async fn a_cancellation_accepted_after_the_answer_is_chosen_but_before_it_is_queued_wins() {
        let (connection, mut lines) = Connection::new(limits());
        let broker: Arc<dyn Broker> = Arc::new(CancelledAsItAnswers {
            connection: connection.clone(),
            id: json!("c-1"),
        });
        call_with(&connection, &broker, json!("c-1")).unwrap();
        let reply = answered(&mut lines).await;
        assert_eq!(reply["id"], "c-1");
        assert_eq!(reply["error"]["code"], code::REQUEST_CANCELLED, "{reply}");
    }

    /// A broker whose answer is ready in the same moment the session ends.
    struct EndedAsItAnswers {
        connection: Connection,
    }

    impl Broker for EndedAsItAnswers {
        fn call<'a>(
            &'a self,
            _method: &'a str,
            _params: Value,
        ) -> Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send + 'a>> {
            Box::pin(async move {
                self.connection.end("The extension was stopped");
                Ok(json!({"rows": []}))
            })
        }
    }

    /// The session ending cancels every call still being answered, one whose
    /// answer was already chosen included: `end` takes each call's
    /// cancellation and leaves its entry, so the worker sees it accepted.
    #[tokio::test(start_paused = true)]
    async fn a_call_whose_answer_was_chosen_as_the_session_ended_is_answered_as_cancelled() {
        let (connection, mut lines) = Connection::new(limits());
        let broker: Arc<dyn Broker> = Arc::new(EndedAsItAnswers {
            connection: connection.clone(),
        });
        call_with(&connection, &broker, json!("c-1")).unwrap();
        let reply = answered(&mut lines).await;
        assert_eq!(reply["id"], "c-1");
        assert_eq!(reply["error"]["code"], code::REQUEST_CANCELLED, "{reply}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_id_still_being_answered_cannot_be_used_again_until_it_is() {
        let (connection, mut lines) = Connection::new(limits());
        let (broker, _) = pending();
        call_with(&connection, &broker, json!(1)).unwrap();
        // The number 1 and the string "1" are different ids.
        call_with(&connection, &broker, json!("1")).unwrap();
        let Err(Violation(why)) = call_with(&connection, &broker, json!(1)) else {
            panic!("a second call 1 was taken while the first was in flight");
        };
        assert!(why.contains("still answering"), "{why}");
        cancel_call(&connection, &broker, json!(1));
        assert_eq!(answered(&mut lines).await["id"], 1);
        assert_eq!(call_with(&connection, &broker, json!(1)), Ok(()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_cancellation_for_a_call_already_answered_or_never_made_is_ignored() {
        let (connection, mut lines) = Connection::new(limits());
        call_from_sidecar(&connection, json!("c-1"), "host/read").unwrap();
        answered(&mut lines).await;
        cancel_call(&connection, &broker(), json!("c-1"));
        cancel_call(&connection, &broker(), json!("never"));
        let quiet = tokio::time::timeout(Duration::from_secs(1), lines.answers.recv()).await;
        assert!(quiet.is_err(), "a second answer was written: {quiet:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn ending_the_session_cancels_every_call_in_flight() {
        let (connection, _lines) = Connection::new(limits());
        let (broker, dropped) = pending();
        for id in 0..3 {
            call_with(&connection, &broker, json!(id)).unwrap();
        }
        tokio::task::yield_now().await;
        connection.end("The extension process exited unexpectedly");
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(dropped.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn the_reader_stops_at_the_first_line_that_breaks_the_protocol() {
        let (connection, _lines) = Connection::new(limits());
        let stdout: &[u8] = b"\n{\"jsonrpc\":\"2.0\",\"method\":\"x\"}\nnot json\n{\"jsonrpc\":\"2.0\",\"method\":\"y\"}\n";
        let ReadEnd::Violation(Violation(why)) = connection.read(stdout, broker()).await else {
            panic!("the reader did not stop");
        };
        assert!(why.contains("not JSON"), "{why}");
        let eof = connection
            .read(&b"{\"jsonrpc\":\"2.0\",\"method\":\"x\"}\n"[..], broker())
            .await;
        assert!(matches!(eof, ReadEnd::Eof), "{eof:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn the_writer_writes_one_line_per_message_and_closes_stdin_when_told() {
        let (connection, lines) = Connection::new(limits());
        let (stdin, mut sidecar) = tokio::io::duplex(1024);
        let writer = tokio::spawn(write(lines, stdin));
        connection.send("{\"a\":1}".into());
        connection.send("{\"b\":2}".into());
        connection.close_stdin();
        writer.await.unwrap();
        let mut read = String::new();
        tokio::io::AsyncReadExt::read_to_string(&mut sidecar, &mut read)
            .await
            .unwrap();
        assert_eq!(read, "{\"a\":1}\n{\"b\":2}\n");
    }

    // Pod tunnels (#1091).

    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;
    use srelens_sidecar_protocol::{
        HostConnectParams, MAX_TUNNELS, MAX_TUNNEL_CHUNK_BYTES, TUNNEL_WINDOW_BYTES,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

    const WINDOW: usize = TUNNEL_WINDOW_BYTES as usize;

    /// The pod's end of a tunnel, and the way to end the tunnel as the
    /// registry would when the pod leaves the scope.
    struct Upstream {
        pod: DuplexStream,
        end: oneshot::Sender<String>,
    }

    /// Connects each tunnel to an in-memory pipe of `size` bytes, handing the
    /// test the pod's end. Refuses the pod named `gone`.
    struct Pods {
        upstreams: mpsc::UnboundedSender<Upstream>,
        size: usize,
    }

    fn pipe(size: usize, pod: String, upstreams: &mpsc::UnboundedSender<Upstream>) -> Tunnel {
        let (host, far) = tokio::io::duplex(size);
        let (end, ended) = oneshot::channel::<String>();
        let _ = upstreams.send(Upstream { pod: far, end });
        Tunnel {
            io: Box::new(host),
            pod,
            ended: Box::pin(async move {
                match ended.await {
                    Ok(why) => why,
                    Err(_) => std::future::pending().await,
                }
            }),
        }
    }

    impl Broker for Pods {
        fn call<'a>(
            &'a self,
            method: &'a str,
            params: Value,
        ) -> Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send + 'a>> {
            NoBroker.call(method, params)
        }

        fn connect<'a>(
            &'a self,
            ask: HostConnectParams,
        ) -> Pin<Box<dyn Future<Output = Result<Tunnel, RpcError>> + Send + 'a>> {
            Box::pin(async move {
                let pod = ask.pod.expect("tests name a pod");
                if pod == "hang" {
                    std::future::pending::<()>().await;
                }
                if pod == "gone" {
                    return Err(RpcError::new(
                        code::CAPABILITY_FAILED,
                        "Pod team/gone is not running",
                    ));
                }
                Ok(pipe(self.size, pod, &self.upstreams))
            })
        }
    }

    fn pods(size: usize) -> (Arc<dyn Broker>, mpsc::UnboundedReceiver<Upstream>) {
        let (upstreams, opened) = mpsc::unbounded_channel();
        (Arc::new(Pods { upstreams, size }), opened)
    }

    fn tunnel_session() -> (Connection, Outbox) {
        let (connection, lines) = Connection::new(limits());
        connection.set_api_version(semver::Version::new(0, 3, 0));
        (connection, lines)
    }

    fn ask_connect(connection: &Connection, broker: &Arc<dyn Broker>, tunnel: u64, pod: &str) {
        connection
            .handle(
                Incoming::Request {
                    id: json!(format!("c{tunnel}")),
                    method: method::HOST_CONNECT.into(),
                    params: json!({"context": {"clusterId": "prod", "namespace": "team"},
                        "capability": "kafka", "pod": pod, "tunnel": tunnel}),
                },
                broker,
            )
            .expect("a connect is not a violation");
    }

    /// The answer to `host/connect` for `tunnel`.
    async fn connect(
        connection: &Connection,
        lines: &mut Outbox,
        broker: &Arc<dyn Broker>,
        tunnel: u64,
        pod: &str,
    ) -> Value {
        ask_connect(connection, broker, tunnel, pod);
        let reply = answered(lines).await;
        assert_eq!(reply["id"], format!("c{tunnel}"));
        reply
    }

    /// Open tunnel `tunnel` to kafka-0, returning the pod's end.
    async fn opened_tunnel(
        connection: &Connection,
        lines: &mut Outbox,
        broker: &Arc<dyn Broker>,
        upstreams: &mut mpsc::UnboundedReceiver<Upstream>,
        tunnel: u64,
    ) -> Upstream {
        let reply = connect(connection, lines, broker, tunnel, "kafka-0").await;
        assert_eq!(reply["result"], json!({"pod": "kafka-0"}), "{reply}");
        upstreams.try_recv().expect("the broker connected")
    }

    /// The next tunnel frame the host queued, failing at a virtual deadline
    /// like `sent`.
    async fn tunnel_frame(lines: &mut Outbox) -> Value {
        let next = tokio::time::timeout(Duration::from_secs(24 * 3600), lines.tunnels.recv());
        let line = next.await.expect("no tunnel frame came").expect("a frame");
        serde_json::from_str(&line).expect("JSON")
    }

    /// No tunnel frame comes while everything else runs as far as it can.
    async fn no_tunnel_frame(lines: &mut Outbox) {
        let quiet = tokio::time::timeout(Duration::from_secs(1), lines.tunnels.recv()).await;
        assert!(quiet.is_err(), "a tunnel frame was queued: {quiet:?}");
    }

    fn decoded(frame: &Value) -> Vec<u8> {
        assert_eq!(frame["method"], "tunnel/data", "{frame}");
        BASE64
            .decode(frame["params"]["data"].as_str().expect("data"))
            .expect("base64")
    }

    fn data(tunnel: u64, bytes: &[u8]) -> Value {
        json!({"tunnel": tunnel, "data": BASE64.encode(bytes)})
    }

    /// The pod's end reads to EOF: srelens dropped its end of the connection.
    async fn reads_eof(pod: &mut DuplexStream) {
        let mut rest = Vec::new();
        tokio::time::timeout(Duration::from_secs(24 * 3600), pod.read_to_end(&mut rest))
            .await
            .expect("the connection to the pod was never dropped")
            .expect("in-memory reads succeed");
    }

    fn open_tunnels(connection: &Connection) -> usize {
        connection.state().tunnels.len()
    }

    #[tokio::test(start_paused = true)]
    async fn connect_needs_api_0_3() {
        let (connection, mut lines) = Connection::new(limits());
        connection.set_api_version(semver::Version::new(0, 2, 0));
        let (broker, mut upstreams) = pods(1024);
        let reply = connect(&connection, &mut lines, &broker, 1, "kafka-0").await;
        assert_eq!(reply["error"]["code"], code::METHOD_NOT_FOUND, "{reply}");
        assert_eq!(
            reply["error"]["message"],
            "This callback requires negotiated sidecar API 0.3.0"
        );
        assert!(upstreams.try_recv().is_err(), "the broker was asked");
    }

    #[tokio::test(start_paused = true)]
    async fn connect_answers_the_pod() {
        let (connection, mut lines) = tunnel_session();
        let (broker, mut upstreams) = pods(1024);
        let _upstream = opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, 1).await;
        assert_eq!(open_tunnels(&connection), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_tunnel_id_must_grow() {
        let (connection, mut lines) = tunnel_session();
        let (broker, mut upstreams) = pods(1024);
        let _five = opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, 5).await;
        for again in [5, 4] {
            let reply = connect(&connection, &mut lines, &broker, again, "kafka-0").await;
            assert_eq!(reply["error"]["code"], code::INVALID_PARAMS, "{reply}");
        }
        let refused = connect(&connection, &mut lines, &broker, 6, "gone").await;
        assert_eq!(
            refused["error"]["code"],
            code::CAPABILITY_FAILED,
            "{refused}"
        );
        let reused = connect(&connection, &mut lines, &broker, 6, "kafka-0").await;
        assert_eq!(reused["error"]["code"], code::INVALID_PARAMS, "{reused}");
        let _seven = opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, 7).await;
        assert_eq!(open_tunnels(&connection), 2);
    }

    /// A broker whose connection opens in the same moment the reader accepts
    /// the sidecar's cancellation of the connect.
    struct CancelledAsItConnects {
        connection: Connection,
        upstreams: mpsc::UnboundedSender<Upstream>,
    }

    impl Broker for CancelledAsItConnects {
        fn call<'a>(
            &'a self,
            method: &'a str,
            params: Value,
        ) -> Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send + 'a>> {
            NoBroker.call(method, params)
        }

        fn connect<'a>(
            &'a self,
            ask: HostConnectParams,
        ) -> Pin<Box<dyn Future<Output = Result<Tunnel, RpcError>> + Send + 'a>> {
            Box::pin(async move {
                cancel_call(
                    &self.connection,
                    &broker(),
                    json!(format!("c{}", ask.tunnel)),
                );
                Ok(pipe(1024, "kafka-0".into(), &self.upstreams))
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_cancelled_connect_leaves_no_tunnel() {
        let (connection, mut lines) = tunnel_session();
        let (upstreams, mut opened) = mpsc::unbounded_channel();
        let broker: Arc<dyn Broker> = Arc::new(CancelledAsItConnects {
            connection: connection.clone(),
            upstreams,
        });
        let reply = connect(&connection, &mut lines, &broker, 1, "kafka-0").await;
        assert_eq!(reply["error"]["code"], code::REQUEST_CANCELLED, "{reply}");
        assert_eq!(open_tunnels(&connection), 0);
        let mut upstream = opened.try_recv().expect("the broker connected");
        reads_eof(&mut upstream.pod).await;
    }

    #[tokio::test(start_paused = true)]
    async fn the_seventeenth_tunnel_is_refused() {
        let (connection, mut lines) = tunnel_session();
        let (broker, mut upstreams) = pods(1024);
        let mut open = Vec::new();
        for tunnel in 1..=MAX_TUNNELS as u64 {
            open.push(
                opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, tunnel).await,
            );
        }
        let reply = connect(&connection, &mut lines, &broker, 17, "kafka-0").await;
        assert_eq!(reply["error"]["code"], code::INTERNAL_ERROR, "{reply}");
        assert!(upstreams.try_recv().is_err(), "the broker was asked");
        assert_eq!(open_tunnels(&connection), MAX_TUNNELS);
        // Each of the 16 still carries bytes.
        for (n, upstream) in open.iter_mut().enumerate() {
            upstream.pod.write_all(b"x").await.unwrap();
            let frame = tunnel_frame(&mut lines).await;
            assert_eq!(decoded(&frame), b"x", "tunnel {}", n + 1);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn connects_in_flight_count_against_the_tunnel_limit() {
        let (connection, mut lines) = tunnel_session();
        let (broker, mut upstreams) = pods(1024);
        let mut open = Vec::new();
        for tunnel in 1..=9 {
            open.push(
                opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, tunnel).await,
            );
        }
        // Seven more whose pods never answer: within the eight call places.
        for tunnel in 10..=16 {
            ask_connect(&connection, &broker, tunnel, "hang");
        }
        tokio::task::yield_now().await;
        let reply = connect(&connection, &mut lines, &broker, 17, "kafka-0").await;
        assert_eq!(reply["error"]["code"], code::INTERNAL_ERROR, "{reply}");
        let message = reply["error"]["message"].as_str().unwrap();
        assert!(message.contains("16 open tunnels"), "{message}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_tunnel_frame_with_no_tunnel_id_breaks_the_protocol() {
        let (connection, _lines) = tunnel_session();
        for params in [json!({}), json!({"tunnel": 0}), json!({"tunnel": "1"})] {
            let Err(Violation(why)) = frame(&connection, "tunnel/close", params.clone()) else {
                panic!("{params} was accepted");
            };
            assert!(why.contains("tunnel id"), "{why}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn pod_bytes_arrive_in_chunks() {
        let (connection, mut lines) = tunnel_session();
        let (broker, mut upstreams) = pods(256 * 1024);
        let mut upstream = opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, 1).await;
        let sent: Vec<u8> = (0..100 * 1024).map(|n| (n % 251) as u8).collect();
        upstream.pod.write_all(&sent).await.unwrap();
        let mut got = Vec::new();
        while got.len() < sent.len() {
            let frame = tunnel_frame(&mut lines).await;
            assert_eq!(frame["params"]["tunnel"], 1);
            let chunk = decoded(&frame);
            assert!(
                chunk.len() <= MAX_TUNNEL_CHUNK_BYTES,
                "{} bytes",
                chunk.len()
            );
            assert!(!chunk.is_empty());
            got.extend(chunk);
        }
        assert_eq!(got, sent);
    }

    /// The pod's bytes framed until no more come.
    async fn framed(lines: &mut Outbox) -> usize {
        let mut bytes = 0;
        while let Ok(Some(line)) =
            tokio::time::timeout(Duration::from_secs(1), lines.tunnels.recv()).await
        {
            bytes += decoded(&serde_json::from_str(&line).unwrap()).len();
        }
        bytes
    }

    #[tokio::test(start_paused = true)]
    async fn the_host_stops_at_the_window() {
        let (connection, mut lines) = tunnel_session();
        let (broker, mut upstreams) = pods(1024 * 1024);
        let mut upstream = opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, 1).await;
        upstream
            .pod
            .write_all(&vec![7; WINDOW + 2 * 65536])
            .await
            .unwrap();
        assert_eq!(framed(&mut lines).await, WINDOW);
        frame(
            &connection,
            "tunnel/credit",
            json!({"tunnel": 1, "bytes": 65536}),
        )
        .unwrap();
        assert_eq!(framed(&mut lines).await, 65536);
    }

    #[tokio::test(start_paused = true)]
    async fn sidecar_bytes_reach_the_pod_and_earn_credit() {
        let (connection, mut lines) = tunnel_session();
        let (broker, mut upstreams) = pods(1024);
        let mut upstream = opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, 1).await;
        frame(&connection, "tunnel/data", data(1, b"hello")).unwrap();
        let mut read = [0; 5];
        upstream.pod.read_exact(&mut read).await.unwrap();
        assert_eq!(&read, b"hello");
        let credit = tunnel_frame(&mut lines).await;
        assert_eq!(credit["method"], "tunnel/credit");
        assert_eq!(credit["params"], json!({"tunnel": 1, "bytes": 5}));
    }

    /// Tunnel `tunnel` was closed by srelens, saying why: the frame and the
    /// dropped connection to the pod. The session goes on.
    async fn closed_saying(
        connection: &Connection,
        lines: &mut Outbox,
        upstream: &mut Upstream,
        tunnel: u64,
        says: &str,
    ) {
        let close = tunnel_frame(lines).await;
        assert_eq!(close["method"], "tunnel/close", "{close}");
        assert_eq!(close["params"]["tunnel"], tunnel);
        let message = close["params"]["message"].as_str().expect("a message");
        assert!(message.contains(says), "{message}");
        reads_eof(&mut upstream.pod).await;
        assert!(!connection.is_ended());
        assert!(!connection.state().tunnels.contains_key(&tunnel));
    }

    #[tokio::test(start_paused = true)]
    async fn sending_past_the_window_closes_the_tunnel() {
        let (connection, mut lines) = tunnel_session();
        // The pod reads nothing, so nothing is credited back.
        let (broker, mut upstreams) = pods(1024);
        let mut upstream = opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, 1).await;
        let chunk = vec![1; MAX_TUNNEL_CHUNK_BYTES];
        for _ in 0..WINDOW / MAX_TUNNEL_CHUNK_BYTES {
            assert_eq!(frame(&connection, "tunnel/data", data(1, &chunk)), Ok(()));
        }
        assert_eq!(open_tunnels(&connection), 1, "the window itself is allowed");
        assert_eq!(frame(&connection, "tunnel/data", data(1, b"!")), Ok(()));
        closed_saying(&connection, &mut lines, &mut upstream, 1, "window").await;
    }

    #[tokio::test(start_paused = true)]
    async fn an_oversized_or_undecodable_chunk_closes_the_tunnel() {
        let (connection, mut lines) = tunnel_session();
        let (broker, mut upstreams) = pods(1024 * 1024);
        let mut big = opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, 1).await;
        let oversized = data(1, &vec![0; MAX_TUNNEL_CHUNK_BYTES + 1]);
        assert_eq!(frame(&connection, "tunnel/data", oversized), Ok(()));
        closed_saying(&connection, &mut lines, &mut big, 1, "65536").await;

        for (tunnel, params) in [
            // Base64-shaped, but its last symbol has bits set past the bytes.
            (2, json!({"tunnel": 2, "data": "aGl="})),
            (3, json!({"tunnel": 3, "data": "not base64"})),
            (4, json!({"tunnel": 4})),
        ] {
            let mut upstream =
                opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, tunnel).await;
            assert_eq!(frame(&connection, "tunnel/data", params), Ok(()));
            closed_saying(&connection, &mut lines, &mut upstream, tunnel, "base64").await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn credit_past_the_window_closes_the_tunnel() {
        let (connection, mut lines) = tunnel_session();
        let (broker, mut upstreams) = pods(1024 * 1024);
        // The host has sent nothing yet, so its whole window is still open.
        let mut first = opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, 1).await;
        assert_eq!(
            frame(
                &connection,
                "tunnel/credit",
                json!({"tunnel": 1, "bytes": 1})
            ),
            Ok(())
        );
        closed_saying(&connection, &mut lines, &mut first, 1, "credit").await;

        // Credit for what was sent is fine; one byte more is not.
        let mut second = opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, 2).await;
        second.pod.write_all(b"abc").await.unwrap();
        assert_eq!(decoded(&tunnel_frame(&mut lines).await), b"abc");
        assert_eq!(
            frame(
                &connection,
                "tunnel/credit",
                json!({"tunnel": 2, "bytes": 3})
            ),
            Ok(())
        );
        assert_eq!(open_tunnels(&connection), 1);
        assert_eq!(
            frame(
                &connection,
                "tunnel/credit",
                json!({"tunnel": 2, "bytes": 1})
            ),
            Ok(())
        );
        closed_saying(&connection, &mut lines, &mut second, 2, "credit").await;

        // More than a window in one frame.
        let mut third = opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, 3).await;
        let over = json!({"tunnel": 3, "bytes": WINDOW + 1});
        assert_eq!(frame(&connection, "tunnel/credit", over), Ok(()));
        closed_saying(&connection, &mut lines, &mut third, 3, "credit").await;
    }

    #[tokio::test(start_paused = true)]
    async fn either_side_closes() {
        let (connection, mut lines) = tunnel_session();
        let (broker, mut upstreams) = pods(1024);
        let mut first = opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, 1).await;
        assert_eq!(
            frame(&connection, "tunnel/close", json!({"tunnel": 1})),
            Ok(())
        );
        reads_eof(&mut first.pod).await;
        assert_eq!(open_tunnels(&connection), 0);
        no_tunnel_frame(&mut lines).await;

        let second = opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, 2).await;
        drop(second.pod);
        let close = tunnel_frame(&mut lines).await;
        assert_eq!(close["method"], "tunnel/close");
        assert_eq!(
            close["params"],
            json!({"tunnel": 2}),
            "an EOF is not a failure"
        );
        assert_eq!(open_tunnels(&connection), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn ended_closes_with_its_reason() {
        let (connection, mut lines) = tunnel_session();
        let (broker, mut upstreams) = pods(1024);
        let mut upstream = opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, 1).await;
        let end = std::mem::replace(&mut upstream.end, oneshot::channel().0);
        end.send("Pod team/kafka-0 left the scope".into()).unwrap();
        let close = tunnel_frame(&mut lines).await;
        assert_eq!(
            close["params"],
            json!({"tunnel": 1, "message": "Pod team/kafka-0 left the scope"})
        );
        reads_eof(&mut upstream.pod).await;
        assert_eq!(open_tunnels(&connection), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn frames_for_no_tunnel_are_dropped() {
        let (connection, mut lines) = tunnel_session();
        for (method, params) in [
            ("tunnel/data", data(9, b"late")),
            ("tunnel/credit", json!({"tunnel": 9, "bytes": 5})),
            ("tunnel/close", json!({"tunnel": 9})),
        ] {
            assert_eq!(frame(&connection, method, params), Ok(()), "{method}");
        }
        no_tunnel_frame(&mut lines).await;
        assert!(lines.host.try_recv().is_err(), "the host wrote something");
        assert!(!connection.is_ended());
    }

    #[tokio::test(start_paused = true)]
    async fn health_is_written_before_tunnel_frames() {
        let (connection, lines) = tunnel_session();
        for n in 0..TUNNEL_LANE {
            connection
                .inner
                .tunnels
                .try_send(format!("{{\"frame\":{n}}}"))
                .expect("the lane has room");
        }
        assert!(
            connection.inner.tunnels.try_send("{}".into()).is_err(),
            "the lane holds {TUNNEL_LANE} frames"
        );
        let health = tokio::spawn({
            let connection = connection.clone();
            async move {
                connection
                    .call("health", &json!({}), Duration::from_secs(10))
                    .await
            }
        });
        tokio::task::yield_now().await;
        let (stdin, sidecar) = tokio::io::duplex(64 * 1024);
        tokio::spawn(write(lines, stdin));
        let mut first = String::new();
        tokio::io::AsyncBufReadExt::read_line(&mut tokio::io::BufReader::new(sidecar), &mut first)
            .await
            .unwrap();
        let first: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(first["method"], "health", "{first}");
        health.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn a_reader_that_stops_holds_no_more_than_the_lane() {
        let (connection, mut lines) = tunnel_session();
        // A small pipe, so each read of the pod is a small frame.
        let (broker, mut upstreams) = pods(1024);
        let upstream = opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, 1).await;
        let written = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let pod = tokio::spawn({
            let written = written.clone();
            let mut pod = upstream.pod;
            async move {
                loop {
                    pod.write_all(&[3; 1024]).await.unwrap();
                    written.fetch_add(1024, std::sync::atomic::Ordering::SeqCst);
                }
            }
        });
        // Nothing drains the lane: the writer is stalled.
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(lines.tunnels.len(), TUNNEL_LANE);
        let written = written.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            written <= WINDOW,
            "the pump read {written} bytes from the pod"
        );
        pod.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn ending_the_session_drops_every_upstream() {
        let (connection, mut lines) = tunnel_session();
        let (broker, mut upstreams) = pods(1024);
        let mut open = Vec::new();
        for tunnel in 1..=3 {
            open.push(
                opened_tunnel(&connection, &mut lines, &broker, &mut upstreams, tunnel).await,
            );
        }
        // Tunnel 1's pump is parked on a full lane.
        for _ in 0..TUNNEL_LANE {
            connection.inner.tunnels.try_send("{}".into()).unwrap();
        }
        open[0].pod.write_all(b"parked").await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        connection.end("The extension was stopped");
        for upstream in &mut open {
            reads_eof(&mut upstream.pod).await;
        }
        assert_eq!(open_tunnels(&connection), 0);
    }
}
