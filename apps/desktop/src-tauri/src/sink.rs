//! Tauri implementation of the shared EventSink: events go to the WebView
//! over the exact same channels the frontend already subscribes to.

use srelens_streams::EventSink;
use tauri::{AppHandle, Emitter};

pub struct TauriSink(pub AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, channel: &str, payload: serde_json::Value) {
        let _ = self.0.emit(channel, payload);
    }
}
