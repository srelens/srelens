//! Test helper: an EventSink that records every emitted event in memory.

use std::sync::Mutex;

use crate::sink::EventSink;

#[derive(Default)]
pub struct TestSink {
    events: Mutex<Vec<(String, serde_json::Value)>>,
}

impl TestSink {
    /// Payloads emitted on `channel`, in order.
    pub fn payloads_for(&self, channel: &str) -> Vec<serde_json::Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(c, _)| c == channel)
            .map(|(_, p)| p.clone())
            .collect()
    }

    /// Distinct channels that received at least one event, in first-seen order.
    pub fn channels(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for (c, _) in self.events.lock().unwrap().iter() {
            if !seen.contains(c) {
                seen.push(c.clone());
            }
        }
        seen
    }
}

impl EventSink for TestSink {
    fn emit(&self, channel: &str, payload: serde_json::Value) {
        self.events
            .lock()
            .unwrap()
            .push((channel.to_string(), payload));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_events_per_channel() {
        let sink = TestSink::default();
        sink.emit("a", serde_json::json!(1));
        sink.emit("b", serde_json::json!("x"));
        sink.emit("a", serde_json::json!(2));
        assert_eq!(
            sink.payloads_for("a"),
            vec![serde_json::json!(1), serde_json::json!(2)]
        );
        assert_eq!(sink.channels(), vec!["a".to_string(), "b".to_string()]);
    }
}
