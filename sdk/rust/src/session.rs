//! One session with srelens: the reader loop, the lifecycle, and the running
//! handlers. `health` and the other lifecycle calls are answered here, on the
//! reader, never behind handler work.

use serde_json::{json, Value};
use srelens_sidecar_protocol::{
    code, method, CancelParams, InitializeParams, InitializeResult, Message, Notification, Request,
    RequestId, Response, RpcError, StreamCancelParams, StreamCloseParams, StreamErrorParams,
    StreamOpenParams, UnsupportedApiVersion, SIDECAR_API_VERSIONS,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use crate::context::Shared;
use crate::outbox::{limit_text, Outbox, Unsent};
use crate::sidecar::Sidecar;
use crate::{Context, Error, Frames, Host, SidecarError};

/// A running handler: an app request by its id's JSON text, or a stream.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    Request(String),
    Stream(u64),
}

/// Running handlers' cancellation tokens. A handler finishing, or srelens's
/// cancel (`$/cancelRequest` or `stream/cancel`), removes the entry and
/// answers. At session end every remaining entry is removed by `cancel_all`
/// and nothing more is written for it.
pub(crate) type Running = Arc<Mutex<HashMap<Key, CancellationToken>>>;

enum Flow {
    Continue,
    Shutdown,
}

pub(crate) async fn serve<R, W>(sidecar: Sidecar, reader: R, writer: W) -> Result<(), SidecarError>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (outbox, lines) = Outbox::new();
    let mut writer = tokio::spawn(lines.run(writer));
    let mut session = Session::new(sidecar, outbox.clone());
    let mut input = BufReader::new(reader).lines();
    // Whether the loop below already consumed (awaited) the writer's
    // JoinHandle: that happens only when the writer itself ends the
    // session, in which case awaiting it again is neither needed nor safe.
    let mut writer_consumed = false;
    let ended = loop {
        tokio::select! {
            line = input.next_line() => {
                let line = match line {
                    Ok(Some(line)) => line,
                    Ok(None) => break Ok(()),
                    Err(e) => break Err(SidecarError::Io(e)),
                };
                if line.trim().is_empty() {
                    continue;
                }
                let message = match Message::parse(&line) {
                    Ok(message) => message,
                    Err(why) => {
                        log::error!("srelens wrote a line that is not JSON-RPC: {why}");
                        break Err(SidecarError::Protocol(why));
                    }
                };
                match session.handle(message).await {
                    Flow::Continue => {}
                    Flow::Shutdown => break Ok(()),
                }
            }
            // The writer stopped on its own -- a write to srelens failed, or
            // it panicked -- before the session otherwise decided to end.
            result = &mut writer => {
                writer_consumed = true;
                break Err(writer_stopped(result));
            }
        }
    };
    session.end();
    outbox.close().await;
    let ended = if writer_consumed {
        ended
    } else {
        // The session ended for its own reason (srelens's input ended, or it
        // shut the sidecar down); a write failure on the way out still
        // matters more than a clean `Ok(())`.
        match (ended, writer.await) {
            (Ok(()), Ok(Err(e))) => Err(SidecarError::Io(e)),
            (ended, _) => ended,
        }
    };
    ended
}

/// Why the session ends when the writer stops on its own, before the
/// session otherwise decided to: the writer's own error if it had one, or a
/// stand-in if it returned `Ok` unasked or panicked.
fn writer_stopped(result: Result<std::io::Result<()>, tokio::task::JoinError>) -> SidecarError {
    match result {
        Ok(Err(e)) => SidecarError::Io(e),
        _ => SidecarError::Io(std::io::Error::other("the writer stopped")),
    }
}

struct Session {
    sidecar: Arc<Sidecar>,
    outbox: Outbox,
    shared: Option<Arc<Shared>>,
    running: Running,
    tasks: JoinSet<()>,
}

impl Session {
    fn new(sidecar: Sidecar, outbox: Outbox) -> Session {
        Session {
            sidecar: Arc::new(sidecar),
            outbox,
            shared: None,
            running: Running::default(),
            tasks: JoinSet::new(),
        }
    }

    async fn handle(&mut self, message: Message) -> Flow {
        // Collect finished handler tasks as each message arrives, so a
        // long-lived sidecar's `tasks` does not grow without bound: a
        // `JoinSet` never reaps on its own unless something joins it.
        while self.tasks.try_join_next().is_some() {}
        match message {
            Message::Request(request) => return self.request(request).await,
            Message::Notification(note) => self.notify(note).await,
            // Only a handler calls srelens, and none runs before `initialize`:
            // an answer before it is for no call, so it is dropped.
            Message::Response(response) => {
                if let Some(shared) = &self.shared {
                    shared.host.answered(response);
                }
            }
        }
        Flow::Continue
    }

    async fn request(&mut self, request: Request) -> Flow {
        let Request {
            id, method, params, ..
        } = request;
        if method == method::INITIALIZE {
            self.initialize(id, params).await;
            return Flow::Continue;
        }
        if self.shared.is_none() {
            let why = RpcError::new(
                code::INVALID_REQUEST,
                "srelens has not initialized this sidecar",
            );
            answer(&self.outbox, id, Err(why)).await;
            return Flow::Continue;
        }
        match method.as_str() {
            method::ACTIVATE | method::HEALTH | method::DEACTIVATE => {
                // Ahead of every frame and answer queued: srelens restarts a
                // sidecar that leaves `health` unanswered. `{}` with srelens's
                // id is never near the size limit.
                let _ = self
                    .outbox
                    .send_lifecycle(&Response::ok(id, json!({})))
                    .await;
            }
            // Ended before the answer, as Go's session does: a handler giving
            // up a call to srelens on its way out, on its own thread, then
            // sends nothing after the answer. The answer is the session's last
            // line, on the general lane, behind every line queued before it.
            method::SHUTDOWN => {
                self.end();
                answer(&self.outbox, id, Ok(json!({}))).await;
                return Flow::Shutdown;
            }
            method::STREAM_OPEN => self.open_stream(id, params).await,
            name => self.operation(id, name, params).await,
        }
        Flow::Continue
    }

    async fn operation(&mut self, id: RequestId, name: &str, params: Value) {
        let Some(handler) = self.sidecar.registry.operations.get(name).cloned() else {
            let why = RpcError::new(
                code::METHOD_NOT_FOUND,
                format!("this sidecar has no operation `{name}`"),
            );
            return answer(&self.outbox, id, Err(why)).await;
        };
        let shared = self.shared.clone().expect("initialized");
        let key = Key::Request(serde_json::to_string(&id).expect("plain JSON"));
        let cancel = CancellationToken::new();
        self.running
            .lock()
            .expect("not poisoned")
            .insert(key.clone(), cancel.clone());
        let (outbox, running, name) = (self.outbox.clone(), self.running.clone(), name.to_owned());
        let work = handler(Context::new(shared, cancel), params);
        self.tasks.spawn(async move {
            // Its own task, so a panic is caught here; aborted with this one.
            let handler = AbortOnDropHandle::new(tokio::spawn(work));
            let outcome = match handler.await {
                Ok(result) => result.map_err(Error::into_rpc),
                Err(e) if e.is_panic() => {
                    log::error!("the handler for `{name}` panicked");
                    Err(RpcError::new(
                        code::INTERNAL_ERROR,
                        format!("the handler for `{name}` panicked"),
                    ))
                }
                Err(_) => return,
            };
            // Cancelled: `-32800` was the answer, and this one is dropped.
            if running.lock().expect("not poisoned").remove(&key).is_none() {
                return;
            }
            answer(&outbox, id, outcome).await;
        });
    }

    async fn open_stream(&mut self, id: RequestId, params: Value) {
        let open: StreamOpenParams = match serde_json::from_value(params) {
            Ok(open) => open,
            Err(e) => {
                let why = RpcError::new(code::INVALID_PARAMS, format!("stream/open: {e}"));
                return answer(&self.outbox, id, Err(why)).await;
            }
        };
        let Some(handler) = self.sidecar.registry.streams.get(&open.method).cloned() else {
            let why = RpcError::new(
                code::METHOD_NOT_FOUND,
                format!("this sidecar has no stream `{}`", open.method),
            );
            return answer(&self.outbox, id, Err(why)).await;
        };
        let shared = self.shared.clone().expect("initialized");
        let cancel = CancellationToken::new();
        let frames = Frames::new(open.stream, self.outbox.clone(), cancel.clone());
        let closer = frames.clone();
        let work = match handler(Context::new(shared, cancel.clone()), open.params, frames) {
            Ok(work) => work,
            Err(refused) => return answer(&self.outbox, id, Err(refused.into_rpc())).await,
        };
        let key = Key::Stream(open.stream);
        self.running
            .lock()
            .expect("not poisoned")
            .insert(key.clone(), cancel);
        // The ack is queued before the handler starts, so it precedes every frame.
        answer(&self.outbox, id, Ok(json!({}))).await;
        let (outbox, running, stream, name) = (
            self.outbox.clone(),
            self.running.clone(),
            open.stream,
            open.method,
        );
        self.tasks.spawn(async move {
            let handler = AbortOnDropHandle::new(tokio::spawn(work));
            let ended = match handler.await {
                Ok(result) => result.map_err(|e| e.message().to_owned()),
                Err(e) if e.is_panic() => {
                    log::error!("the stream `{name}` panicked");
                    Err(format!("the stream `{name}` panicked"))
                }
                Err(_) => return,
            };
            // Cancelled: srelens asked for nothing more, a terminal frame included.
            if running.lock().expect("not poisoned").remove(&key).is_none() {
                return;
            }
            // A `Frames` clone kept past the handler must not follow the
            // terminal frame with data.
            closer.finish().await;
            let terminal = match ended {
                Ok(()) => Notification::new(
                    method::STREAM_CLOSE,
                    serde_json::to_value(StreamCloseParams { stream }).expect("plain JSON"),
                ),
                Err(message) => Notification::new(
                    method::STREAM_ERROR,
                    serde_json::to_value(StreamErrorParams { stream, message })
                        .expect("plain JSON"),
                ),
            };
            // A `stream/close` can never be too large; a `stream/error` whose
            // handler-given message alone is over the limit still must end
            // the stream, so it is replaced with one that says why.
            if let Err(Unsent::TooLarge(bytes)) = outbox.send(&terminal).await {
                let short = Notification::new(
                    method::STREAM_ERROR,
                    serde_json::to_value(StreamErrorParams {
                        stream,
                        message: format!(
                            "the stream's error is {bytes} bytes, over the {} a message may be",
                            limit_text()
                        ),
                    })
                    .expect("plain JSON"),
                );
                let _ = outbox.send(&short).await;
            }
        });
    }

    async fn initialize(&mut self, id: RequestId, params: Value) {
        if self.shared.is_some() {
            let why = RpcError::new(
                code::INVALID_REQUEST,
                "srelens already initialized this sidecar",
            );
            return answer(&self.outbox, id, Err(why)).await;
        }
        let params: InitializeParams = match serde_json::from_value(params) {
            Ok(params) => params,
            Err(e) => {
                let why = RpcError::new(code::INVALID_PARAMS, format!("initialize: {e}"));
                return answer(&self.outbox, id, Err(why)).await;
            }
        };
        // The newest version both speak. Ours are oldest first.
        let chosen = SIDECAR_API_VERSIONS
            .iter()
            .rev()
            .find(|ours| params.api_versions.iter().any(|offered| offered == *ours));
        let Some(chosen) = chosen else {
            let mut why = RpcError::new(
                code::UNSUPPORTED_API_VERSION,
                format!(
                    "this sidecar speaks sidecar API {}; srelens offered {}",
                    SIDECAR_API_VERSIONS.join(", "),
                    params.api_versions.join(", ")
                ),
            );
            why.data = Some(
                serde_json::to_value(UnsupportedApiVersion {
                    supported: SIDECAR_API_VERSIONS
                        .iter()
                        .map(|v| (*v).to_owned())
                        .collect(),
                })
                .expect("plain JSON"),
            );
            return answer(&self.outbox, id, Err(why)).await;
        };
        // Built here, not in `Session::new`: how many calls it may have in
        // flight comes in the limits.
        let host = Host::new(self.outbox.clone(), &params.limits);
        self.shared = Some(Arc::new(Shared {
            data_dir: params.data_directory.into(),
            limits: params.limits,
            api_version: (*chosen).to_owned(),
            host,
        }));
        let result = InitializeResult {
            api_version: (*chosen).to_owned(),
            sidecar: Some(self.sidecar.identity.clone()),
        };
        answer(
            &self.outbox,
            id,
            Ok(serde_json::to_value(result).expect("plain JSON")),
        )
        .await;
    }

    async fn notify(&mut self, note: Notification) {
        if note.method == method::CANCEL {
            let Ok(CancelParams { id }) = serde_json::from_value(note.params) else {
                return;
            };
            let key = Key::Request(serde_json::to_string(&id).expect("plain JSON"));
            let token = self.running.lock().expect("not poisoned").remove(&key);
            if let Some(token) = token {
                token.cancel();
                let why = RpcError::new(code::REQUEST_CANCELLED, "srelens cancelled the request");
                answer(&self.outbox, id, Err(why)).await;
            }
        } else if note.method == method::STREAM_CANCEL {
            if let Ok(StreamCancelParams { stream }) = serde_json::from_value(note.params) {
                if let Some(token) = self
                    .running
                    .lock()
                    .expect("not poisoned")
                    .remove(&Key::Stream(stream))
                {
                    token.cancel();
                }
            }
        }
    }

    fn cancel_all(&self) {
        for (_, token) in self.running.lock().expect("not poisoned").drain() {
            token.cancel();
        }
    }

    /// The session is over: stop every handler. The host is disconnected
    /// first, before handlers are cancelled or aborted: a cancelled handler
    /// may give up its call to srelens, and aborting one drops its pending
    /// host-call futures, and each such drop would otherwise race the
    /// session's own shutdown by sending a `$/cancelRequest` of its own (see
    /// `CancelOnDrop` in `host.rs`). Runs before the `shutdown` answer, and
    /// again, doing nothing more, once the session ends.
    fn end(&mut self) {
        // No host before `initialize`, and then no handler to disconnect.
        if let Some(shared) = &self.shared {
            shared.host.disconnect();
        }
        self.cancel_all();
        self.tasks.abort_all();
    }
}

/// Answer request `id`. A result too large to send is answered with why.
pub(crate) async fn answer(outbox: &Outbox, id: RequestId, outcome: Result<Value, RpcError>) {
    if let Err(Unsent::TooLarge(bytes)) = outbox
        .send(&Response {
            id: id.clone(),
            outcome,
        })
        .await
    {
        let why = RpcError::new(
            code::INTERNAL_ERROR,
            format!(
                "the answer is {bytes} bytes, over the {} a message may be",
                limit_text()
            ),
        );
        let _ = outbox.send(&Response::err(id, why)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Context, Error};
    use serde_json::json;

    fn init_line() -> String {
        json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "apiVersions": ["0.1.0"],
                "host": {"name": "srelens", "version": "0.15.0"},
                "limits": {
                    "requestTimeoutMs": 30000,
                    "maxConcurrentRequests": 8,
                    "maxStreams": 5,
                    "memoryBytes": 268435456u64,
                    "cpus": 1.0,
                    "dataBytes": 1073741824u64,
                    "dataEntries": 100000
                },
                "dataDirectory": "/tmp"
            }
        })
        .to_string()
    }

    #[tokio::test]
    async fn finished_handlers_are_reaped_as_messages_arrive() {
        let sidecar = Sidecar::new("t", "1")
            .operation("echo", |_ctx: Context, v: Value| async move {
                Ok::<_, Error>(v)
            });
        let (outbox, writer) = Outbox::new();
        tokio::spawn(writer.run(tokio::io::sink()));
        let mut session = Session::new(sidecar, outbox);

        session.handle(Message::parse(&init_line()).unwrap()).await;

        for i in 1..=1000u64 {
            let request = json!({
                "jsonrpc": "2.0",
                "id": i,
                "method": "echo",
                "params": {"n": i}
            })
            .to_string();
            session.handle(Message::parse(&request).unwrap()).await;
            tokio::task::yield_now().await;
        }

        assert!(
            session.tasks.len() <= 16,
            "expected finished handler tasks to be reaped, found {} still tracked",
            session.tasks.len()
        );
    }

    // `shutdown` cancels every handler, and one that gives up its call to
    // srelens then drops it -- on its own thread, so possibly after the
    // shutdown answer is queued and before the session has ended. That
    // answer is the session's last line, so the drop must not send a
    // `$/cancelRequest` after it.
    #[tokio::test]
    async fn a_call_given_up_after_shutdown_is_not_cancelled_after_the_shutdown_answer() {
        let (outbox, writer) = Outbox::new();
        let written = tokio::spawn(async move {
            let mut out = Vec::new();
            writer.run(&mut out).await.map(|()| out)
        });
        let mut session = Session::new(Sidecar::new("t", "1"), outbox.clone());
        session.handle(Message::parse(&init_line()).unwrap()).await;
        let host = session.shared.as_ref().expect("initialized").host.clone();
        let context = crate::CallContext::new("kind-dev", None).unwrap();
        let mut call = Box::pin(host.read(&context, "apps"));
        // Polled once: its request is queued, and it waits for the answer.
        tokio::select! {
            biased;
            _ = &mut call => panic!("the call finished, though srelens never answered it"),
            () = std::future::ready(()) => {}
        }
        let shutdown = json!({"jsonrpc": "2.0", "id": 1, "method": "shutdown", "params": {}});
        let flow = session
            .handle(Message::parse(&shutdown.to_string()).unwrap())
            .await;
        assert!(matches!(flow, Flow::Shutdown));
        drop(call);
        // Lets a `$/cancelRequest` the drop spawned reach the queue before
        // the session ends.
        tokio::task::yield_now().await;
        session.end();
        outbox.close().await;
        let written = String::from_utf8(written.await.unwrap().unwrap()).unwrap();
        let last: Value = serde_json::from_str(written.lines().last().unwrap()).unwrap();
        assert_eq!(
            last,
            json!({"jsonrpc": "2.0", "id": 1, "result": {}}),
            "the last line written:\n{written}"
        );
    }
}
