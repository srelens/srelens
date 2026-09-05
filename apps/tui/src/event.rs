use std::time::Duration;
use crossterm::event::{Event as CrosstermEvent, EventStream, KeyEvent, MouseEvent};
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc::{error::TryRecvError, unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::watch;

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
    pause_tx: watch::Sender<bool>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = unbounded_channel();
        let event_tx = tx.clone();
        let (pause_tx, mut pause_rx) = watch::channel(false);

        // Spawn background event listener for crossterm terminal events
        tokio::spawn(async move {
            let mut reader: Option<EventStream> = Some(EventStream::new());
            let mut interval = tokio::time::interval(tick_rate);

            loop {
                // If paused, drop reader so crossterm releases stdin completely
                if *pause_rx.borrow() {
                    let _ = reader.take();
                    while *pause_rx.borrow() {
                        if pause_rx.changed().await.is_err() {
                            return;
                        }
                    }
                    reader = Some(EventStream::new());
                }

                let delay = interval.tick();

                tokio::select! {
                    changed = pause_rx.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        // Next loop iteration will observe *pause_rx.borrow() == true and drop reader
                    }
                    _ = delay => {
                        if event_tx.send(AppEvent::Tick).is_err() {
                            break;
                        }
                    }
                    maybe_event = async {
                        if let Some(r) = reader.as_mut() {
                            r.next().await
                        } else {
                            futures::future::pending().await
                        }
                    } => {
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

        Self { tx, rx, pause_tx }
    }

    pub async fn recv(&mut self) -> Option<AppEvent> {
        self.rx.recv().await
    }

    pub fn try_recv(&mut self) -> Result<AppEvent, TryRecvError> {
        self.rx.try_recv()
    }

    pub fn pause(&self) {
        let _ = self.pause_tx.send(true);
    }

    pub fn resume(&self) {
        let _ = self.pause_tx.send(false);
    }
}
