//! Write a srelens sidecar in Rust: an executable app's process, which srelens
//! starts in the OS sandbox and talks to over stdin and stdout
//! (`docs/extensions/sidecar-protocol.md`).
//!
//! ```ignore
//! # use srelens_sidecar::{Context, Error, Sidecar};
//! # #[derive(serde::Deserialize)] struct In { name: String }
//! #[tokio::main]
//! async fn main() -> std::process::ExitCode {
//!     Sidecar::new("hello", env!("CARGO_PKG_VERSION"))
//!         .operation("greet", |_ctx: Context, input: In| async move {
//!             Ok::<_, Error>(format!("Hello, {}", input.name))
//!         })
//!         .run_stdio()
//!         .await
//! }
//! ```
//!
//! The SDK answers the lifecycle and `health`, runs each request on its own
//! task, cancels what srelens cancels, keeps every line within the protocol's
//! limits, and exits when srelens says to or goes away.

mod context;
mod error;
mod outbox;
mod session;
mod sidecar;

pub use context::Context;
pub use error::{Error, SidecarError};
pub use sidecar::Sidecar;
pub use srelens_sidecar_protocol::{code, CallContext, ContextError, InitializeLimits, RpcError};
