use std::time::Duration;
use crossterm::event::{Event as CrosstermEvent, EventStream, KeyEvent, MouseEvent};
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Resize(u16, u16),
    Tick,
    StreamEvent {
        channel: String,
        payload: Value,
    },
    ActionResult {
        title: String,
        result: Result<String, String>,
    },
    LineageResult {
        kind: String,
        name: String,
        result: Result<srelens_kube::lineage::LineageNode, String>,
    },
    NodeInspectorResult {
        node_name: String,
        result: Result<srelens_kube::node_inspector::NodeInspectorDetails, String>,
    },
}

pub struct EventHandler {
    pub tx: UnboundedSender<AppEvent>,
    pub rx: UnboundedReceiver<AppEvent>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = unbounded_channel();
        let event_tx = tx.clone();

        // Spawn background event listener for crossterm terminal events
        tokio::spawn(async move {
            let mut reader = EventStream::new();
            let mut interval = tokio::time::interval(tick_rate);

            loop {
                let delay = interval.tick();
                let crossterm_event = reader.next();

                tokio::select! {
                    _ = delay => {
                        if event_tx.send(AppEvent::Tick).is_err() {
                            break;
                        }
                    }
                    maybe_event = crossterm_event => {
                        match maybe_event {
                            Some(Ok(evt)) => {
                                let app_evt = match evt {
                                    CrosstermEvent::Key(key) => AppEvent::Key(key),
                                    CrosstermEvent::Mouse(mouse) => AppEvent::Mouse(mouse),
                                    CrosstermEvent::Paste(text) => AppEvent::Paste(text),
                                    CrosstermEvent::Resize(cols, rows) => AppEvent::Resize(cols, rows),
                                    _ => continue,
                                };
                                if event_tx.send(app_evt).is_err() {
                                    break;
                                }
                            }
                            Some(Err(_)) => break,
                            None => break,
                        }
                    }
                }
            }
        });

        Self { tx, rx }
    }
}
