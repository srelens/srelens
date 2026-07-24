//! Host-agnostic streaming cores shared by the Tauri desktop app and the web
//! server: each manager drives srelens-kube streams and emits events into an
//! [`EventSink`] implemented by the host (Tauri events, WebSocket frames).

pub mod exec;
pub mod forward;
pub mod helm;
pub mod logs;
pub mod sink;
pub mod terminal;
pub mod test_util;
pub mod watch;

pub use sink::EventSink;
