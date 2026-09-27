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
//!
//! - [`Inspect`]: what the Inspector reads from a supervisor (#575), and
//!   [`AppLog`], the app's log it writes to.
//!
//! No manifest kind runs a sidecar yet (#574); the broker a sidecar calls back
//! into is #573, the macOS memory and CPU watchdog #713.

mod connection;
mod limits;
mod metrics;
pub mod protocol;
pub mod sandbox;
mod supervisor;

pub use crate::app_log::{AppLog, LogLevel, LogLine, LogSource, LOG_LINES, LOG_LINE_BYTES};
pub use connection::{Broker, NoBroker, RequestError, SidecarStream, StreamEvent, STREAM_BUFFER};
pub use limits::{Limits, Policy};
pub use metrics::{Inspect, Latency, RequestMetrics, SidecarMetrics, LATENCY_SAMPLES};
pub use protocol::SIDECAR_API_VERSIONS;
pub use sandbox::{
    Enforcement, Exit, LaunchError, Launched, Launcher, MemoryProbe, OsSandbox, Process,
    SandboxConfig, SidecarCommand,
};
pub use supervisor::{Action, SidecarConfig, SidecarStatus, Supervisor, UNEXPECTED_EXIT};
