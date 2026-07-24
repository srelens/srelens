//! The host abstraction: streaming cores emit named-channel events through
//! this trait. The desktop implements it with Tauri events; the web server
//! implements it with WebSocket frames.

/// A destination for streamed events. `channel` names are dynamic (e.g.
/// `exec:out:3`) and mirror the Tauri event channels the WebView subscribes to.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, channel: &str, payload: serde_json::Value);
}
