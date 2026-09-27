//! The runtime for executable apps (#572): a supervised sidecar process that
//! speaks JSON-RPC 2.0 over its stdin and stdout, inside the OS sandbox the
//! #571 spike chose for each platform.
//!
//! - [`protocol`]: the messages and their framing.
//! - [`Supervisor`]: starts the sidecar, negotiates the API version, admits
//!   requests under the [`Limits`], and restarts it after a crash on the
//!   [`Policy`]'s backoff, then disables it.
//! - [`sandbox`]: the per-OS backends behind [`Launcher`]. On an OS without
//!   one, a sidecar is refused.
//! - [`CapabilityBroker`]: what a sidecar calls back into (#573), the app
//!   facade the host UI calls, with the app's identity and an explicit cluster
//!   context on every call, confirmation for writes, and an audit record.
//! - [`data`]: its data directory (#573), the one path it may write: private
//!   to its app, limited in size, removed with the app.
//!
//! No manifest kind runs a sidecar yet (#574); per-app logs are #575, the
//! macOS memory and CPU watchdog #713.

mod broker;
mod connection;
pub mod data;
mod limits;
mod logs;
pub mod protocol;
pub mod sandbox;
mod supervisor;

pub use broker::{AppIdentity, CallContext, CapabilityBroker, Consent, ConsentRequest, NoConsent};
pub use connection::{Broker, NoBroker, RequestError, SidecarStream, StreamEvent, STREAM_BUFFER};
pub use limits::{Limits, Policy};
pub use logs::{LogLine, LogSource, LOG_LINES, LOG_LINE_BYTES};
pub use protocol::SIDECAR_API_VERSIONS;
pub use sandbox::{
    Enforcement, Exit, LaunchError, Launched, Launcher, OsSandbox, Process, SandboxConfig,
    SidecarCommand,
};
pub use supervisor::{Action, SidecarConfig, SidecarStatus, Supervisor, UNEXPECTED_EXIT};
