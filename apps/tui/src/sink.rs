use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use serde_json::Value;
use srelens_streams::EventSink;

use crate::event::AppEvent;

/// Bridges srelens_streams EventSink trait into the TUI's Tokio event bus
pub struct TuiSink {
    tx: UnboundedSender<AppEvent>,
}

impl TuiSink {
    pub fn new(tx: UnboundedSender<AppEvent>) -> Self {
        Self { tx }
    }

    pub fn arc(tx: UnboundedSender<AppEvent>) -> Arc<dyn EventSink> {
        Arc::new(Self::new(tx))
    }
}

impl EventSink for TuiSink {
    fn emit(&self, channel: &str, payload: Value) {
        let _ = self.tx.send(AppEvent::StreamEvent {
            channel: channel.to_string(),
            payload,
        });
    }
}
