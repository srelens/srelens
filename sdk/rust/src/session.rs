//! One session with srelens: the reader loop, the lifecycle, and the running
//! handlers. `health` and the other lifecycle calls are answered here, on the
//! reader, never behind handler work.

use serde_json::{json, Value};
use srelens_sidecar_protocol::{
    code, method, CancelParams, InitializeParams, InitializeResult, Message, Notification, Request,
    RequestId, Response, RpcError, UnsupportedApiVersion, SIDECAR_API_VERSIONS,
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
use crate::{Context, Error, SidecarError};

/// A running handler: an app request by its id's JSON text, or a stream.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    Request(String),
    #[allow(dead_code)] // used from Task 6
    Stream(u64),
}

/// Running handlers' cancellation tokens. Whoever removes an entry answers
/// for it: the handler when it finishes, or a cancellation.
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
            // Answers to the sidecar's own calls: routed in Task 5.
            Message::Response(_) => {}
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
                answer(&self.outbox, id, Ok(json!({}))).await;
            }
            method::SHUTDOWN => {
                self.cancel_all();
                answer(&self.outbox, id, Ok(json!({}))).await;
                return Flow::Shutdown;
            }
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
        self.shared = Some(Arc::new(Shared {
            data_dir: params.data_directory.into(),
            limits: params.limits,
            api_version: (*chosen).to_owned(),
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
        }
    }

    fn cancel_all(&self) {
        for (_, token) in self.running.lock().expect("not poisoned").drain() {
            token.cancel();
        }
    }

    /// The session is over: stop every handler.
    fn end(&mut self) {
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
                "the result is {bytes} bytes, over the {} a message may be",
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
}
