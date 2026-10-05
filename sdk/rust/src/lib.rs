//! Write a srelens sidecar in Rust: an executable app's process, which srelens
//! starts in the OS sandbox and talks to over stdin and stdout
//! (`docs/extensions/sidecar-protocol.md`).
//!
//! ```no_run
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
//! The SDK answers the lifecycle and `health` on a thread of its own, so a
//! handler that blocks its thread cannot hold them up; runs each request on
//! its own task, on your runtime; cancels what srelens cancels; keeps every
//! line within the protocol's limits; and exits when srelens says to or goes
//! away.

mod context;
mod error;
mod host;
mod logging;
mod outbox;
mod session;
mod sidecar;
mod stream;

pub use context::Context;
pub use error::{Error, HostError, SidecarError, StreamClosed};
pub use host::Host;
pub use sidecar::Sidecar;
pub use srelens_sidecar_protocol::{code, CallContext, ContextError, InitializeLimits, RpcError};
pub use stream::Frames;
