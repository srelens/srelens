#[allow(unused_imports)] // used from Task 4
use serde::de::DeserializeOwned;
#[allow(unused_imports)] // used from Task 4
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

/// The handlers a sidecar serves, by method name.
#[derive(Default)]
pub(crate) struct Registry {
    #[allow(dead_code)] // used from Task 4
    pub(crate) operations: HashMap<String, OperationFn>,
}

/// A sidecar: its name, its version, and the handlers it serves.
pub struct Sidecar {
    pub(crate) identity: Peer,
    #[allow(dead_code)] // used from Task 4
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
    #[allow(dead_code)] // used from Task 4
    pub(crate) fn check_name(&self, name: &str) {
        assert!(
            shape::is_identifier(name) && !is_reserved(name),
            "`{name}` cannot name an operation: use 1 to 64 ASCII letters, digits and hyphens, \
             as the manifest does, and none of srelens's own methods"
        );
        assert!(
            !self.registry.operations.contains_key(name),
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
}
