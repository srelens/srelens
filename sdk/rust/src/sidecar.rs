use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use srelens_sidecar_protocol::{is_reserved, shape, Peer};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{Context, Error, SidecarError};

pub(crate) type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
pub(crate) type OperationFn =
    Arc<dyn Fn(Context, Value) -> BoxFuture<Result<Value, Error>> + Send + Sync>;
pub(crate) type StreamFn = Arc<
    dyn Fn(Context, Value, crate::Frames) -> Result<BoxFuture<Result<(), Error>>, Error>
        + Send
        + Sync,
>;

/// The handlers a sidecar serves, by method name.
#[derive(Default)]
pub(crate) struct Registry {
    pub(crate) operations: HashMap<String, OperationFn>,
    pub(crate) streams: HashMap<String, StreamFn>,
}

/// A sidecar: its name, its version, and the handlers it serves.
pub struct Sidecar {
    pub(crate) identity: Peer,
    pub(crate) registry: Registry,
}

impl Sidecar {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Sidecar {
        Sidecar {
            identity: Peer {
                name: name.into(),
                version: version.into(),
            },
            registry: Registry::default(),
        }
    }

    /// Panics when `name` cannot name an operation, or is taken: a
    /// programming error, found the first time the sidecar starts.
    pub(crate) fn check_name(&self, name: &str) {
        assert!(
            shape::is_identifier(name) && !is_reserved(name),
            "`{name}` cannot name an operation or a stream: use 1 to 64 ASCII letters, digits and \
             hyphens, as the manifest does, and none of srelens's own methods"
        );
        assert!(
            !self.registry.operations.contains_key(name)
                && !self.registry.streams.contains_key(name),
            "`{name}` is registered twice"
        );
    }

    /// Serve srelens over `reader` and `writer` until it shuts the sidecar
    /// down or its input ends. For tests; a sidecar runs [`Sidecar::run_stdio`].
    pub async fn run<R, W>(self, reader: R, writer: W) -> Result<(), SidecarError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        crate::session::serve(self, reader, writer).await
    }

    /// Serve `name`: srelens's request `name` runs `handler` with its params
    /// read as `I`. Params that do not read as `I` are answered `-32602`
    /// without running it; its `O` is the result, and its `Error` the error.
    /// Never block `handler`'s thread: run blocking work with
    /// `tokio::task::spawn_blocking`, or srelens's `health` check can starve.
    pub fn operation<I, O, F, Fut>(mut self, name: &str, handler: F) -> Sidecar
    where
        I: DeserializeOwned + Send + 'static,
        O: Serialize + Send + 'static,
        F: Fn(Context, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<O, Error>> + Send + 'static,
    {
        self.check_name(name);
        let handler = Arc::new(handler);
        let run: OperationFn = Arc::new(move |ctx, params| {
            let handler = handler.clone();
            Box::pin(async move {
                let input: I = serde_json::from_value(params)
                    .map_err(|e| Error::invalid_params(e.to_string()))?;
                let output = handler(ctx, input).await?;
                Ok(serde_json::to_value(output)?)
            })
        });
        self.registry.operations.insert(name.to_owned(), run);
        self
    }

    /// Serve the stream `name`: srelens's `stream/open` for `name` runs
    /// `handler` with its params read as `I` (refused `-32602` otherwise).
    /// Returning `Ok` closes the stream; `Err` fails it with the message.
    pub fn stream<I, F, Fut>(mut self, name: &str, handler: F) -> Sidecar
    where
        I: DeserializeOwned + Send + 'static,
        F: Fn(Context, I, crate::Frames) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), Error>> + Send + 'static,
    {
        self.check_name(name);
        let handler = Arc::new(handler);
        let open: StreamFn = Arc::new(move |ctx, params, frames| {
            let input: I =
                serde_json::from_value(params).map_err(|e| Error::invalid_params(e.to_string()))?;
            let handler = handler.clone();
            Ok(Box::pin(async move { handler(ctx, input, frames).await }))
        });
        self.registry.streams.insert(name.to_owned(), open);
        self
    }

    /// Serve srelens over stdin and stdout, then exit: 0 after `shutdown` or
    /// when stdin closes, 1 when srelens wrote something that is not
    /// JSON-RPC. Installs the stderr logger and the panic hook first. Exits
    /// the process rather than returning: tokio's stdin reader cannot be
    /// cancelled, and would keep the runtime from shutting down.
    pub async fn run_stdio(self) -> std::process::ExitCode {
        crate::logging::install();
        let code = match self.run(tokio::io::stdin(), tokio::io::stdout()).await {
            Ok(()) => 0,
            Err(e) => {
                log::error!("{e}");
                1
            }
        };
        log::logger().flush();
        std::process::exit(code)
    }
}
