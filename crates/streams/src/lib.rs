//! Host-agnostic streaming cores shared by the Tauri desktop app and the web
//! server: each manager drives srelens-kube streams and emits events into an
//! [`EventSink`] implemented by the host (Tauri events, WebSocket frames).

pub mod sink;
pub mod test_util;

pub use sink::EventSink;
