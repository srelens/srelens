//! One session with srelens: the reader loop, the lifecycle, and the running
//! handlers. `health` and the other lifecycle calls are answered here, on the
//! reader, never behind handler work.

use serde_json::{json, Value};
use srelens_sidecar_protocol::{
    code, method, InitializeParams, InitializeResult, Message, Notification, Request, RequestId,
    Response, RpcError, UnsupportedApiVersion, SIDECAR_API_VERSIONS,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::context::Shared;
use crate::outbox::{limit_text, Outbox, Unsent};
use crate::sidecar::Sidecar;
use crate::SidecarError;

/// A running handler: an app request by its id's JSON text, or a stream.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    #[allow(dead_code)] // used from Task 4
    Request(String),
    #[allow(dead_code)] // used from Task 4
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
    let writer = tokio::spawn(lines.run(writer));
    let mut session = Session {
        sidecar: Arc::new(sidecar),
        outbox: outbox.clone(),
        shared: None,
        running: Running::default(),
        tasks: JoinSet::new(),
    };
    let mut input = BufReader::new(reader).lines();
    let ended = loop {
        let line = match input.next_line().await {
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
    };
    session.end();
    outbox.flush().await;
    writer.abort();
    ended
}

struct Session {
    sidecar: Arc<Sidecar>,
    outbox: Outbox,
    shared: Option<Arc<Shared>>,
    running: Running,
    tasks: JoinSet<()>,
}

impl Session {
    async fn handle(&mut self, message: Message) -> Flow {
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
            _ => {
                let why = RpcError::new(
                    code::METHOD_NOT_FOUND,
                    format!("this sidecar has no operation `{method}`"),
                );
                answer(&self.outbox, id, Err(why)).await;
            }
        }
        Flow::Continue
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

    async fn notify(&mut self, _note: Notification) {
        // `$/cancelRequest` and `stream/cancel`: Tasks 4 and 6.
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
