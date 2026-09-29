use srelens_sidecar_protocol::InitializeLimits;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// What `initialize` told the sidecar, shared by every handler.
pub(crate) struct Shared {
    pub(crate) data_dir: PathBuf,
    pub(crate) limits: InitializeLimits,
    pub(crate) api_version: String,
}

/// What a handler is given about its call and its sidecar.
#[derive(Clone)]
pub struct Context {
    shared: Arc<Shared>,
    cancel: CancellationToken,
}

impl Context {
    #[allow(dead_code)] // used from Task 4
    pub(crate) fn new(shared: Arc<Shared>, cancel: CancellationToken) -> Context {
        Context { shared, cancel }
    }

    /// The one directory the sidecar may write, which is also its working
    /// directory. Write scratch files here: `std::env::temp_dir()` is not
    /// writable on Windows.
    pub fn data_dir(&self) -> &Path {
        &self.shared.data_dir
    }

    /// The limits srelens runs the sidecar under.
    pub fn limits(&self) -> &InitializeLimits {
        &self.shared.limits
    }

    /// The sidecar API version srelens and the sidecar agreed on.
    pub fn api_version(&self) -> &str {
        &self.shared.api_version
    }

    /// Whether srelens cancelled this call, or the session is ending.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Resolves when srelens cancels this call, or the session ends.
    pub async fn cancelled(&self) {
        self.cancel.cancelled().await
    }
}
